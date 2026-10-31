import { test } from "node:test";
import assert from "node:assert/strict";
import { symbolicMode } from "../ui/file-mode.ts";

test("modes render like ls -l, including special bits", () => {
  assert.equal(symbolicMode(0o755), "rwxr-xr-x");
  assert.equal(symbolicMode(0o640), "rw-r-----");
  assert.equal(symbolicMode(0o000), "---------");
  assert.equal(symbolicMode(0o4755), "rwsr-xr-x");
  assert.equal(symbolicMode(0o4644), "rwSr--r--");
  assert.equal(symbolicMode(0o2750), "rwxr-s---");
  assert.equal(symbolicMode(0o1777), "rwxrwxrwt");
  assert.equal(symbolicMode(0o1776), "rwxrwxrwT");
});
