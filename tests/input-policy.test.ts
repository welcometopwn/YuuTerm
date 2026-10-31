import { test } from "node:test";
import assert from "node:assert/strict";
import {
  assignPane,
  fillPanes,
  showTab,
  swapPane,
  quotedPath,
  requiresPasteConfirmation,
  displayName,
} from "../ui/input-policy.ts";
test("pane reassignment retains unrelated connections and never duplicates a bookmark", () => {
  assert.deepEqual(assignPane([1, 2, 3, 4], 0, 3), [3, 2, null, 4]);
  assert.deepEqual(assignPane([1, null, 3, 4], 1, 9), [1, 9, 3, 4]);
  assert.deepEqual(assignPane([1, 2, 3, 4], 3, 4), [1, 2, 3, 4]);
});
test("path insertion quotes shell metacharacters and does not append Enter", () => {
  assert.equal(
    quotedPath("/tmp/a'b $(touch x); file"),
    "'/tmp/a'\\''b $(touch x); file'",
  );
  for (const value of ["/tmp/a\nb", "/tmp/a\rb", "/tmp/a\x1bb", "/tmp/a\0b"])
    assert.throws(() => quotedPath(value));
});
test("pastes with execution or terminal control characters require review", () => {
  for (const value of [
    "hello\nworld",
    "\r",
    "\t",
    "\x1b[200~",
    "\x7f",
    "\u0085",
  ])
    assert.equal(requiresPasteConfirmation(value), true);
  for (const value of ["hello", "äöü € @ \\", "/home/user/a b"])
    assert.equal(requiresPasteConfirmation(value), false);
  assert.equal(displayName("a\n\x1b"), "a\\x0a\\x1b");
});

test("F1/F2/F3 map to layouts only without modifiers", async () => {
  const { layoutForKey } = await import("../ui/input-policy.ts");
  const k = (key: string, mods: Partial<Record<string, boolean>> = {}) => ({
    key,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    ...mods,
  });
  assert.equal(layoutForKey(k("F1")), 1);
  assert.equal(layoutForKey(k("F2")), 2);
  assert.equal(layoutForKey(k("F3")), 4);
  assert.equal(layoutForKey(k("F4")), null);
  assert.equal(layoutForKey(k("F3", { shiftKey: true })), null);
  assert.equal(layoutForKey(k("F1", { ctrlKey: true })), null);
});

test("Windows drive paths use PowerShell literal quoting without the SFTP slash", () => {
  assert.equal(
    quotedPath("/C:/test/a'b Ω $(whoami).txt"),
    "'C:/test/a''b Ω $(whoami).txt'",
  );
  assert.equal(quotedPath("C:\\test\\a'b.txt"), "'C:\\test\\a''b.txt'");
});
test("more panes fill with hidden open tabs in tab order", () => {
  // Two tabs, single view showing 2: F2 puts 1 beside it.
  assert.deepEqual(fillPanes([2, null, null, null], 2, [1, 2]), [
    2,
    1,
    null,
    null,
  ]);
  // Three tabs in four panes: the fourth stays empty for the chooser.
  assert.deepEqual(fillPanes([1, null], 4, [1, 2, 3]), [1, 2, 3, null]);
  // A tab parked in a hidden slot moves instead of duplicating.
  assert.deepEqual(fillPanes([1, null, null, 3], 2, [1, 3]), [
    1,
    3,
    null,
    null,
  ]);
  assert.deepEqual(fillPanes([1, 2, 3, 4], 2, [1, 2, 3, 4]), [1, 2, 3, 4]);
  // A closed tab left in a hidden slot is dropped.
  assert.deepEqual(fillPanes([1, 4, 3, 2], 4, [1, 3, 4]), [1, 4, 3, null]);
});
test("a visible tab only takes focus; a hidden one replaces the focused pane", () => {
  const panes = [1, 2, 3, 4];
  assert.deepEqual(showTab(panes, 4, 0, 3), { panes, focused: 2 });
  assert.deepEqual(showTab(panes, 4, 1, 5), {
    panes: [1, 5, 3, 4],
    focused: 1,
  });
  // In two panes, slot 3 is hidden: the tab comes into the focused pane.
  assert.deepEqual(showTab(panes, 2, 0, 3), {
    panes: [3, 2, null, 4],
    focused: 0,
  });
});
test("the pane switcher swaps visible tabs and replaces with hidden ones", () => {
  assert.deepEqual(swapPane([1, 2, 3, 4], 0, 3), [3, 2, 1, 4]);
  // Five tabs in four panes: 5 replaces 2, which stays open as a tab.
  assert.deepEqual(swapPane([1, 2, 3, 4], 1, 5), [1, 5, 3, 4]);
});
