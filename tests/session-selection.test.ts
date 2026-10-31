import { test } from "node:test";
import assert from "node:assert/strict";
import { sessionSelection, contextSelection } from "../ui/session-selection.ts";
test("Ctrl-click toggles without losing other selections; context targets are explicit", () => {
  const two = sessionSelection([1], 2, true);
  assert.deepEqual(two, [1, 2]);
  assert.deepEqual(sessionSelection(two, 1, true), [2]);
  assert.deepEqual(sessionSelection(two, 3, false), [3]);
  assert.deepEqual(contextSelection(two, 1), [1, 2]);
  assert.deepEqual(contextSelection(two, 3), [3]);
});
test("Shift follows visible sidebar order, supports reverse ranges and preserves its anchor", () => {
  const order = [9, 4, 7, 2];
  assert.deepEqual(
    sessionSelection([4], 2, false, { shift: true, anchor: 4, order }),
    [4, 7, 2],
  );
  assert.deepEqual(
    sessionSelection([7], 9, false, { shift: true, anchor: 7, order }),
    [9, 4, 7],
  );
  assert.deepEqual(
    sessionSelection([4, 7, 2], 7, false, { shift: true, anchor: 4, order }),
    [4, 7],
  );
  assert.deepEqual(
    sessionSelection([8, 4], 2, true, { shift: true, anchor: 4, order }),
    [8, 4, 7, 2],
  );
  assert.deepEqual(
    sessionSelection([4], 2, false, { shift: true, anchor: 99, order }),
    [2],
  );
  assert.deepEqual(
    sessionSelection([], 2, false, { shift: true, anchor: null, order }),
    [2],
  );
});
