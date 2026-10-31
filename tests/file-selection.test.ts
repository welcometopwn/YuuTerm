import { test } from "node:test";
import assert from "node:assert/strict";
import { selectEntry, MAX_SELECTION } from "../ui/file-selection.ts";

const names = ["a", "b", "c", "d", "e"];
const start = { selected: [] as string[], anchor: null as string | null };

test("plain click selects only the clicked entry and sets the anchor", () => {
  const next = selectEntry(
    names,
    { selected: ["a", "c"], anchor: "a" },
    "d",
    {},
  );
  assert.deepEqual(next, { selected: ["d"], anchor: "d" });
});

test("ctrl click toggles one entry and moves the anchor", () => {
  let s = selectEntry(names, start, "b", {});
  s = selectEntry(names, s, "d", { ctrl: true });
  assert.deepEqual(s, { selected: ["b", "d"], anchor: "d" });
  s = selectEntry(names, s, "b", { ctrl: true });
  assert.deepEqual(s, { selected: ["d"], anchor: "b" });
});

test("shift click selects the range from the anchor in list order", () => {
  let s = selectEntry(names, start, "d", {});
  s = selectEntry(names, s, "b", { shift: true });
  assert.deepEqual(s, { selected: ["b", "c", "d"], anchor: "d" });
  s = selectEntry(names, s, "e", { shift: true });
  assert.deepEqual(s, { selected: ["d", "e"], anchor: "d" });
});

test("ctrl+shift adds the range to the existing selection", () => {
  let s = selectEntry(names, start, "a", {});
  s = selectEntry(names, s, "d", { ctrl: true });
  s = selectEntry(names, s, "e", { ctrl: true, shift: true });
  assert.deepEqual(s, { selected: ["a", "d", "e"], anchor: "d" });
});

test("shift click without a visible anchor behaves like a plain click", () => {
  const s = selectEntry(names, { selected: ["x"], anchor: "x" }, "c", {
    shift: true,
  });
  assert.deepEqual(s, { selected: ["c"], anchor: "c" });
});

test("selection is capped at the bulk operation limit", () => {
  const many = Array.from({ length: MAX_SELECTION + 20 }, (_, i) => `f${i}`);
  let s = selectEntry(many, start, "f0", {});
  s = selectEntry(many, s, many.at(-1)!, { shift: true });
  assert.equal(s.selected.length, MAX_SELECTION);
});
