import { test } from "node:test";
import assert from "node:assert/strict";
import { setImmediate } from "node:timers/promises";
import { FileNavigation, FileClickPairs } from "../ui/file-navigation.ts";
test("continued native click counts activate exactly once per extra left-click pair", () => {
  for (const counts of [
    [0, 1, 2, 3, 3, 3, 3, 3, 3],
    [0, 1, 2, 3, 4, 5, 6, 7, 8],
  ]) {
    const pairs = new FileClickPairs();
    assert.deepEqual(
      counts.flatMap((detail, index) =>
        pairs.continued("7:/fixture/..", detail, 0) ? [index] : [],
      ),
      [4, 6, 8],
    );
  }
});
test("continued click pairs require the same target and reset for a new native sequence", () => {
  const pairs = new FileClickPairs();
  assert.equal(pairs.continued("7:/one", 3, 0), false);
  assert.equal(pairs.continued("7:/two", 3, 0), false);
  assert.equal(pairs.continued("8:/two", 3, 0), false);
  assert.equal(pairs.continued("8:/two", 1, 0), false);
  assert.equal(pairs.continued("8:/two", 2, 0), false);
  assert.equal(pairs.continued("8:/two", 3, 0), false);
  assert.equal(pairs.continued("8:/two", 3, 0), true);
  for (const button of [1, 2]) {
    assert.equal(pairs.continued("8:/two", 3, 0), false);
    assert.equal(pairs.continued("8:/two", 3, button), false);
    assert.equal(pairs.continued("8:/two", 3, 0), false);
    assert.equal(pairs.continued("8:/two", 3, 0), true);
  }
});
function harness() {
  const calls: {
    id: number;
    path: string;
    resolve: (s: string) => void;
    reject: (e: Error) => void;
  }[] = [];
  const errors: unknown[] = [];
  const nav = new FileNavigation(
    (id, path) =>
      new Promise<string>((resolve, reject) => {
        calls.push({ id, path, resolve, reject });
      }),
    (e) => errors.push(e),
  );
  nav.sync(7, "/fixture/logs/one", true);
  return { nav, calls, errors };
}
test("rapid parent actions wait for each canonical result and keep every step", async () => {
  const { nav, calls, errors } = harness();
  nav.navigate(7, { kind: "parent" });
  nav.navigate(7, { kind: "parent" });
  assert.deepEqual(
    calls.map((v) => v.path),
    ["/fixture/logs/one/.."],
  );
  calls[0].resolve("/fixture/logs");
  await setImmediate();
  assert.deepEqual(
    calls.map((v) => v.path),
    ["/fixture/logs/one/..", "/fixture/logs/.."],
  );
  calls[1].resolve("/fixture");
  await setImmediate();
  nav.sync(7, "/fixture/logs", true); // a delayed UI poll must not undo the confirmed final path
  nav.navigate(7, { kind: "parent" });
  assert.equal(calls[2].path, "/fixture/..");
  calls[2].resolve("/");
  await setImmediate();
  assert.deepEqual(errors, []);
});
test("duplicate folder activations coalesce but different targets remain actionable", async () => {
  const { nav, calls } = harness();
  for (let n = 0; n < 12; n++)
    nav.navigate(7, { kind: "path", path: "/fixture/logs" });
  nav.navigate(7, { kind: "path", path: "/fixture/other" });
  assert.equal(calls.length, 1);
  calls[0].resolve("/fixture/logs");
  await setImmediate();
  assert.equal(calls.length, 2);
  assert.equal(calls[1].path, "/fixture/other");
  calls[1].resolve("/fixture/other");
  await setImmediate();
});
test("failure cancels remaining steps and old incarnations cannot navigate a new session", async () => {
  const { nav, calls, errors } = harness();
  nav.navigate(7, { kind: "parent" });
  nav.navigate(7, { kind: "parent" });
  calls[0].reject(new Error("listing refused"));
  await setImmediate();
  assert.equal(calls.length, 1);
  assert.equal(errors.length, 1);
  nav.navigate(7, { kind: "parent" });
  nav.navigate(7, { kind: "parent" });
  nav.sync(8, "/new", true);
  calls[1].resolve("/fixture/logs");
  await setImmediate();
  nav.navigate(7, { kind: "parent" });
  assert.equal(calls.length, 2);
  nav.navigate(8, { kind: "parent" });
  assert.equal(calls[2].id, 8);
  assert.equal(calls[2].path, "/new/..");
  nav.dispose();
  calls[2].resolve("/");
  await setImmediate();
  assert.equal(calls.length, 3);
});
test("pending navigation is bounded and disconnect discards queued work", () => {
  const { nav, calls, errors } = harness();
  for (let n = 0; n < 34; n++) nav.navigate(7, { kind: "parent" });
  assert.equal(calls.length, 1);
  assert.equal(errors.length, 1);
  nav.sync(7, "/fixture", false);
  nav.navigate(7, { kind: "parent" });
  assert.equal(calls.length, 1);
  nav.dispose();
});
