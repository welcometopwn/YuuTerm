// External VNC server qualification (issue #21). Runs only after the Rust
// bootstrap accepts the explicitly configured "vnc-real" profile. The host
// script answers the native sign-in prompts and verifies the server side
// (typed text, pointer, clipboard, geometry) against the markers reported here.
import { invoke } from "@tauri-apps/api/core";
export async function checkVncRealServer() {
  const checks: string[] = [];
  const marker = crypto.randomUUID().replaceAll("-", "").slice(0, 12);
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const until = async (check: () => boolean, name: string, ms = 60000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      if (check()) {
        checks.push(name);
        return;
      }
      await sleep(40);
    }
    throw Error(name);
  };
  const dialog = (label: string) =>
    document.querySelector<HTMLDialogElement>(
      `dialog[aria-label="${label}"][open]`,
    );
  const button = (text: string) =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent?.trim() === text,
    );
  const surface = () => document.querySelector<HTMLElement>(".vnc-surface");
  const canvas = () =>
    document.querySelector<HTMLCanvasElement>(".vnc-surface canvas");
  // A real desktop: more than one colour across a coarse sample grid. The grid
  // includes the edges: a maximised window can fill everything inside them.
  const painted = () => {
    const c = canvas();
    if (!c || !c.width || !c.height) return false;
    const context = c.getContext("2d")!;
    const seen = new Set<string>();
    for (let y = 0; y < 8; y++)
      for (let x = 0; x < 8; x++)
        seen.add(
          context
            .getImageData(
              Math.floor((c.width * x) / 8),
              Math.floor((c.height * y) / 8),
              1,
              1,
            )
            .data.join(","),
        );
    return seen.size > 1;
  };
  const signIn = async (round: string) => {
    await until(
      () => !!dialog("VNC authentication"),
      `${round}: sign-in requested only after TLS identity validation`,
    );
    if (document.querySelector('input[type="password"]'))
      throw Error("VNC secret entry exists in the WebView");
    // The host script answers the native Windows Security prompt.
    dialog("VNC authentication")!
      .querySelector<HTMLFormElement>("form")!
      .requestSubmit();
    await until(
      () => !dialog("VNC authentication") && painted(),
      `${round}: real server desktop renders in the canvas`,
    );
  };
  const key = (target: HTMLElement, type: string, k: string, code: string) =>
    target.dispatchEvent(
      new KeyboardEvent(type, { key: k, code, bubbles: true }),
    );
  try {
    checks.push(`marker ${marker}`);
    await (
      await import("./fixture-gateway-presentation")
    ).checkGatewayPresentation(checks);
    await until(
      () => !!dialog("Verify VNC certificate"),
      "Unknown real server certificate requires review",
    );
    const details = dialog("Verify VNC certificate")!.querySelector(
      ".vnc-certificate",
    )!.textContent!;
    if (
      !["Subject", "Issuer", "Valid from", "Valid until", "SHA-256"].every(
        (v) => details.includes(v),
      )
    )
      throw Error("Certificate details incomplete");
    if (
      document.querySelector(
        'input[type="password"], dialog[aria-label="VNC authentication"]',
      )
    )
      throw Error("Credentials requested before trust decision");
    checks.push("Certificate review shows details before any credentials");
    button("Trust this certificate")!.click();
    await signIn("First connection");
    const firstId = surface()!.dataset.vncId;

    // Resize opt-in: the server desktop follows the visible pane size.
    const pane = () => {
      const r = surface()!.getBoundingClientRect();
      return [
        Math.max(1, Math.min(2048, Math.floor(r.width))),
        Math.max(1, Math.min(2048, Math.floor(r.height))),
      ];
    };
    // Servers may refuse SetDesktopSize (x11vnc: "resize prohibited"); then the
    // refusal must be visible and the desktop stays at the server size, scaled.
    const note = () =>
      document.querySelector(".vnc-actions small")?.textContent;
    let refused = false;
    let expected = pane;
    await until(
      () => {
        const c = canvas()!;
        const [w, h] = pane();
        refused = note() === "Server refused resize · scaled";
        return ((c.width === w && c.height === h) || refused) && painted();
      },
      "Server desktop resized to the visible pane after the opt-in request, or the refusal is shown",
      15000,
    ).catch((error) => {
      const note =
        document.querySelector(".vnc-actions small")?.textContent ??
        document.querySelector(".connection-message")?.textContent;
      throw Error(
        `${error.message}: canvas ${canvas()!.width}x${canvas()!.height}, pane ${pane().join("x")}, ${note}`,
      );
    });
    if (refused) {
      const [w, h] = [canvas()!.width, canvas()!.height];
      expected = () => [w, h];
      checks.push(
        `Server refused the resize request; desktop stays ${w}x${h} and is scaled`,
      );
    }
    checks.push(`size ${canvas()!.width}x${canvas()!.height}`);

    // Cursor pseudo-encoding: the server's pointer shape becomes the local
    // cursor, so the desktop never shows a second, lagging arrow. Checked
    // before typing: xterm hides the pointer on key input (empty shape).
    // A pointer hidden by xterm reappears (as a new shape) once it moves.
    const box = canvas()!.getBoundingClientRect();
    canvas()!.dispatchEvent(
      new PointerEvent("pointermove", {
        // A new position every run: xterm only unhides after real motion.
        clientX: box.left + box.width * (0.2 + Math.random() * 0.2),
        clientY: box.top + box.height * (0.2 + Math.random() * 0.2),
        pointerId: 1,
        pointerType: "mouse",
        bubbles: true,
      }),
    );
    await until(
      () => canvas()!.style.cursor.startsWith("url("),
      "Server cursor shape is the local cursor",
      15000,
    );

    // Keyboard through the canvas key mapping: Shift+letter, digits, Enter.
    const c = canvas()!;
    c.focus();
    key(c, "keydown", "Shift", "ShiftLeft");
    key(c, "keydown", "Y", "KeyY");
    key(c, "keyup", "Y", "KeyY");
    key(c, "keyup", "Shift", "ShiftLeft");
    for (const ch of `t-${marker}`) {
      const code = /[0-9]/.test(ch)
        ? `Digit${ch}`
        : ch === "-"
          ? "Minus"
          : `Key${ch.toUpperCase()}`;
      key(c, "keydown", ch, code);
      key(c, "keyup", ch, code);
      await sleep(15);
    }
    key(c, "keydown", "Enter", "Enter");
    key(c, "keyup", "Enter", "Enter");
    checks.push(`typed Yt-${marker}`);

    // German layout as WebView2 reports it: AltGr is a synthetic left Control
    // plus AltGraph, the character arrives in keypress; a dead key composes.
    const event = (type: string, k: string, code: string, altGr = false) =>
      c.dispatchEvent(
        new KeyboardEvent(type, {
          key: k,
          code,
          ctrlKey: altGr,
          altKey: altGr,
          bubbles: true,
          cancelable: true,
        }),
      );
    key(c, "keydown", "k", "KeyK");
    key(c, "keyup", "k", "KeyK");
    event("keydown", "Control", "ControlLeft", true);
    event("keydown", "AltGraph", "AltRight", true);
    event("keydown", "@", "KeyQ", true);
    event("keypress", "@", "KeyQ", true);
    event("keyup", "@", "KeyQ", true);
    event("keyup", "AltGraph", "AltRight");
    event("keyup", "Control", "ControlLeft");
    event("keydown", "Dead", "Equal");
    event("keyup", "Dead", "Equal");
    event("keydown", "e", "KeyE");
    event("keypress", "é", "KeyE");
    event("keyup", "e", "KeyE");
    for (const ch of marker) {
      const code = /[0-9]/.test(ch) ? `Digit${ch}` : `Key${ch.toUpperCase()}`;
      key(c, "keydown", ch, code);
      key(c, "keyup", ch, code);
      await sleep(15);
    }
    key(c, "keydown", "Enter", "Enter");
    key(c, "keyup", "Enter", "Enter");
    checks.push(`altgr k@é${marker}`);

    // Pointer: move to the centre, then wheel up (button 4 press/release).
    const r = c.getBoundingClientRect();
    const at = { clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 };
    c.dispatchEvent(
      new PointerEvent("pointermove", {
        ...at,
        pointerId: 1,
        pointerType: "mouse",
        bubbles: true,
      }),
    );
    c.dispatchEvent(
      new WheelEvent("wheel", { ...at, deltaY: -100, bubbles: true }),
    );
    checks.push(
      `pointer ${Math.floor(c.width / 2)}x${Math.floor(c.height / 2)}`,
    );
    await sleep(300);

    // Clipboard opt-in: explicit review, then ClientCutText. The server
    // answers with its own clipboard text, which must be discarded silently.
    button("Send clipboard text")!.click();
    await until(
      () =>
        !!document.querySelector(
          'textarea[aria-label="Clipboard text to send"]',
        ),
      "Clipboard send opens the explicit review",
    );
    const text = document.querySelector<HTMLTextAreaElement>(
      'textarea[aria-label="Clipboard text to send"]',
    )!;
    Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!.call(text, `yuuterm-clip-${marker}`);
    text.dispatchEvent(new Event("input", { bubbles: true }));
    await sleep(80);
    button("Send text")!.click();
    await until(
      () => !dialog("Send clipboard text"),
      "Clipboard text sent only after review confirmation",
    );
    await sleep(3000);
    if (surface()?.dataset.vncId !== firstId || !painted())
      throw Error("Session did not survive the server clipboard answer");
    if (document.querySelector(".app-notice"))
      throw Error("Unexpected application error notice");
    checks.push("Server clipboard text is discarded and the session stays up");

    // Reconnect: a fresh incarnation reuses the saved pin, never a password.
    document
      .querySelector<HTMLButtonElement>('button[title="Disconnect"]')!
      .click();
    await until(
      () => !!button("Reconnect"),
      "Disconnect ends the first session",
    );
    button("Reconnect")!.click();
    await until(
      () =>
        !!dialog("VNC authentication") || !!dialog("Verify VNC certificate"),
      "Reconnect starts a new connection",
    );
    if (dialog("Verify VNC certificate"))
      throw Error("Pinned certificate was reviewed again on reconnect");
    checks.push("Reconnect validates the pinned certificate without review");
    await signIn("Reconnect");
    const secondId = surface()!.dataset.vncId;
    if (!secondId || secondId === firstId)
      throw Error("Reconnect reused the graphical session ID");
    checks.push("Reconnect uses a new graphical session ID");
    await until(
      () => {
        const [w, h] = expected();
        return canvas()!.width === w && canvas()!.height === h;
      },
      "Reconnected session keeps the expected desktop size",
      15000,
    );
    // Let queued input drain before the capture shutdown closes the session.
    await sleep(1000);
    await (
      await import("./fixture-gateway-presentation")
    ).showGatewayEditor(checks);
    await (
      await import("./fixture-gateway-presentation")
    ).redactGatewayLabels();
    await invoke("capture_report", { checks, passed: true });
    await invoke("capture_ready", { terminalReady: true });
  } catch (error) {
    const message = document.querySelector(".connection-message")?.textContent;
    await invoke("capture_report", {
      checks: [...checks, String(error), `status: ${message ?? "none"}`],
      passed: false,
    });
    throw error;
  }
}
