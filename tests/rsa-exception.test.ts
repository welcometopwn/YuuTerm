import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { validateException } from "../scripts/review-rsa-exception.mjs";
const policy = JSON.parse(readFileSync("docs/evidence/rsa-exception.json", "utf8"));
const lock = readFileSync("Cargo.lock", "utf8").replaceAll("\r\n", "\n");
const auth = readFileSync("crates/yuuterm-ssh/src/ssh.rs", "utf8");
test("RSA exception is limited by review date, exact packages, graph and reviewed source", () => {
  const now = new Date("2026-10-18T00:00:00Z");
  assert.equal(validateException(policy, lock, auth, now).status, "accepted-risk");
  assert.throws(() => validateException(policy, lock, auth, new Date(policy.review_by + "T00:00:00Z")), /expired/);
  assert.throws(() => validateException(policy, lock.replace('version = "0.10.0-rc.18"', 'version = "0.10.0-rc.19"'), auth, now), /changed/);
  assert.throws(() => validateException(policy, lock, auth + "\n// new path", now), /source changed/);
  assert.throws(() => validateException(policy, lock + '\n[[package]]\nname = "new-consumer"\ndependencies = [\n "rsa",\n]\n', auth, now), /dependent/);
});
