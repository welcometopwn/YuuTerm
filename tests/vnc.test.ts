import test from "node:test";
import assert from "node:assert/strict";
import {
  VncKeyboard,
  keysym,
  fit,
  pointer,
  packet,
  clipboardError,
  type KeyLike,
} from "../ui/vnc-wire.ts";
test("VNC clipboard validates classic Latin-1 and refuses oversized text", () => {
  assert.equal(clipboardError("Grüße\n"), null);
  assert.match(clipboardError("€")!, /Latin-1/);
  assert.match(clipboardError("x".repeat(16385))!, /16 KiB/);
});
test("VNC key symbols preserve modifiers Unicode and releases by physical code", () => {
  assert.equal(keysym("Shift", 2), 0xffe2);
  assert.equal(keysym("F12"), 0xffc9);
  assert.equal(keysym("€"), 0x010020ac);
  assert.equal(keysym("Dead"), null);
  assert.equal(keysym("A"), 65);
});
test("VNC pointer coordinates scale and clamp to the source framebuffer", () => {
  assert.deepEqual(
    pointer(110, 70, { left: 10, top: 20, width: 200, height: 100 }, 640, 400),
    { x: 320, y: 200 },
  );
  assert.deepEqual(
    pointer(
      -100,
      900,
      { left: 10, top: 20, width: 200, height: 100 },
      640,
      400,
    ),
    { x: 0, y: 399 },
  );
});
test("VNC binary tiles reject every truncated prefix oversized dimensions and excess counts", () => {
  const status = new TextEncoder().encode(
    JSON.stringify({ id: 1, width: 1, height: 1 }),
  );
  const buffer = new ArrayBuffer(4 + status.length + 2 + 8 + 4 + 1),
    view = new DataView(buffer);
  view.setUint32(0, status.length);
  new Uint8Array(buffer, 4, status.length).set(status);
  let at = 4 + status.length;
  view.setUint16(at, 1);
  at += 2;
  view.setUint16(at + 4, 1);
  view.setUint16(at + 6, 1);
  new Uint8Array(buffer, at + 8).set([1, 2, 3, 255]);
  assert.equal(packet(buffer).tiles[0].rgba[3], 255);
  for (let n = 0; n < buffer.byteLength; n++)
    assert.throws(() => packet(buffer.slice(0, n)));
  view.setUint16(4 + status.length, 1025);
  assert.throws(() => packet(buffer));
});

const keys = (k: VncKeyboard, steps: [string, KeyLike][]) =>
  steps.flatMap(([type, e]) =>
    (type === "down"
      ? k.down(e).inputs
      : type === "press"
        ? k.press(e)
        : k.up(e)
    ).map((i) =>
      i.kind === "key"
        ? `${i.down ? "+" : "-"}${i.keysym.toString(16)}`
        : i.kind,
    ),
  );
test("VNC AltGr characters reach the server without the synthetic Control", () => {
  const altGr = (key: string, code: string): KeyLike => ({
    key,
    code,
    ctrlKey: true,
    altKey: true,
    getModifierState: (m) => m === "AltGraph",
  });
  const k = new VncKeyboard();
  assert.equal(k.down(altGr("AltGraph", "AltRight")).prevent, true);
  assert.deepEqual(
    keys(k, [
      ["down", { key: "Control", code: "ControlLeft", ctrlKey: true }],
      ["down", altGr("AltGraph", "AltRight")],
      ["down", altGr("@", "KeyQ")],
      ["press", altGr("@", "KeyQ")],
      ["up", altGr("@", "KeyQ")],
      ["down", altGr("€", "KeyE")],
      ["press", altGr("€", "KeyE")],
      ["up", altGr("€", "KeyE")],
      ["up", { key: "Control", code: "ControlLeft" }],
    ]),
    ["+ffe3", "-ffe3", "+40", "-40", "+10020ac", "-10020ac"],
  );
  // No keypress (e.g. Ctrl+Alt+T): sent at keyup, still without AltGr's Control.
  assert.deepEqual(
    keys(new VncKeyboard(), [
      ["down", { key: "Control", code: "ControlLeft", ctrlKey: true }],
      ["down", altGr("t", "KeyT")],
      ["up", altGr("t", "KeyT")],
    ]),
    ["+ffe3", "-ffe3", "+74", "-74"],
  );
});
test("VNC dead keys send the composed character and shortcuts stay immediate", () => {
  const k = new VncKeyboard();
  const dead = k.down({ key: "Dead", code: "Backquote" });
  assert.deepEqual(dead, { inputs: [], prevent: false });
  assert.equal(k.down({ key: "e", code: "KeyE" }).prevent, false);
  assert.deepEqual(
    keys(k, [
      ["press", { key: "ê", code: "KeyE" }],
      ["up", { key: "e", code: "KeyE" }],
      ["down", { key: "Control", code: "ControlLeft", ctrlKey: true }],
      ["down", { key: "c", code: "KeyC", ctrlKey: true }],
      ["up", { key: "c", code: "KeyC", ctrlKey: true }],
      ["up", { key: "Control", code: "ControlLeft" }],
      ["down", { key: "F5", code: "F5" }],
      ["up", { key: "F5", code: "F5" }],
      ["press", { key: "Enter", code: "Enter" }],
    ]),
    ["+ea", "-ea", "+ffe3", "+63", "-63", "-ffe3", "+ffc2", "-ffc2"],
  );
  // A character without keypress is sent before the next key, keeping order.
  assert.deepEqual(
    keys(new VncKeyboard(), [
      ["down", { key: "a", code: "KeyA" }],
      ["down", { key: "Shift", code: "ShiftLeft", location: 1 }],
    ]),
    ["+61", "-61", "+ffe1"],
  );
});
test("VNC desktop scales to the pane in both directions", () => {
  assert.deepEqual(fit(1280, 800, 2560, 1000), { width: 1600, height: 1000 });
  assert.deepEqual(fit(1280, 800, 640, 1000), { width: 640, height: 400 });
  assert.equal(fit(0, 800, 640, 400), null);
});
test("VNC cursor shapes are bounded by the status dimensions", () => {
  const build = (cursor: object | null, tail: number[]) => {
    const status = new TextEncoder().encode(
      JSON.stringify({ id: 1, width: 1, height: 1, cursor }),
    );
    const buffer = new ArrayBuffer(4 + status.length + 2 + tail.length),
      view = new DataView(buffer);
    view.setUint32(0, status.length);
    new Uint8Array(buffer, 4, status.length).set(status);
    new Uint8Array(buffer, 6 + status.length).set(tail);
    return buffer;
  };
  const shape = { serial: 1, hotspot_x: 0, hotspot_y: 0, width: 1, height: 1 };
  assert.equal(packet(build(shape, [0])).cursor, null);
  assert.deepEqual(
    [...packet(build(shape, [1, 9, 8, 7, 255])).cursor!],
    [9, 8, 7, 255],
  );
  assert.throws(() => packet(build(shape, [1, 9, 8, 7])));
  assert.throws(() => packet(build(null, [1])));
  assert.throws(() => packet(build(shape, [2])));
  assert.throws(() => packet(build({ ...shape, width: 129 }, [1])));
});
