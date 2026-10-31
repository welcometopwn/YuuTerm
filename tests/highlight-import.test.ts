import { test } from "node:test";
import assert from "node:assert/strict";
import { reviewSyntax } from "../ui/highlight-import-policy.ts";
test("syntax preview lists frontend-only regex rejections without mutating backend data", () => {
  const input = {
    accepted: [
      {
        line: 3,
        source: "red",
        rule: { regex: true, keywords: "a{101}", color: "#ff0000" },
      },
      {
        line: 2,
        source: "green",
        rule: { regex: true, keywords: "success", color: "#00ff00" },
      },
    ],
    rejected: [{ line: 5, reason: "unknown property" }],
  };
  const before = JSON.stringify(input),
    reviewed = reviewSyntax(input);
  assert.equal(reviewed.accepted.length, 1);
  assert.equal(reviewed.accepted[0].source, "green");
  assert.deepEqual(
    reviewed.rejected.map((r) => r.line),
    [3, 5],
  );
  assert.equal(JSON.stringify(input), before);
});
