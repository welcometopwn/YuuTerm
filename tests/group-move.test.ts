import { test } from "node:test";
import assert from "node:assert/strict";
import { normalizedGroup } from "../ui/group-move-policy.ts";

test("group moves trim names, allow ungrouping, and bound UTF-8 without truncation", () => {
  assert.equal(normalizedGroup("  Team / Lab  "), "Team / Lab");
  assert.equal(normalizedGroup("  "), "");
  assert.equal(normalizedGroup("界".repeat(21) + "a").length, 22);
  for (const value of [
    "a".repeat(65),
    "界".repeat(22),
    "a\u001bb",
    "a\u0085b",
    "a\nb",
  ])
    assert.throws(() => normalizedGroup(value), /64 UTF-8 bytes/);
});
