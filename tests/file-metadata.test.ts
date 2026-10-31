import { test } from "node:test";
import assert from "node:assert/strict";
import { fileOwner } from "../ui/file-metadata.ts";
test("remote names take precedence; unmapped IDs and absent fields remain explicit", () => {
  assert.equal(fileOwner(0, 0, "root", "root"), "root / root");
  assert.equal(fileOwner(0, 0), "0 / 0");
  assert.equal(fileOwner(123, 456, "alice"), "alice / 456");
  assert.equal(fileOwner(null, null), "— / —");
});
