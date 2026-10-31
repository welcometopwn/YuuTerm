import test from "node:test";
import assert from "node:assert/strict";
import { groupSessions, groupNames } from "../ui/session-groups.ts";
import { blankProfile } from "../ui/types.ts";

const p = (id: number, name: string, group?: string) => ({
  ...blankProfile(),
  id,
  name,
  ...(group === undefined ? {} : { group }),
});

test("Ungrouped sessions come first, then groups by name", () => {
  const groups = groupSessions([
    p(1, "web", "Kunden"),
    p(2, "nas"),
    p(3, "pve", "Hosting"),
    p(4, "db", "Kunden"),
    p(5, "router", ""),
    p(6, "x", "Group 10"),
    p(7, "y", "Group 9"),
  ]);
  assert.deepEqual(
    groups.map((g) => [g.name, g.profiles.map((x) => x.id)]),
    [
      ["", [2, 5]],
      ["Group 9", [7]],
      ["Group 10", [6]],
      ["Hosting", [3]],
      ["Kunden", [1, 4]],
    ],
  );
});

test("Group suggestions are unique and sorted", () => {
  assert.deepEqual(
    groupNames([p(1, "a", "b"), p(2, "c", "a"), p(3, "d", "b"), p(4, "e")]),
    ["a", "b"],
  );
});
