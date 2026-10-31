import { test } from "node:test";
import assert from "node:assert/strict";
import { connectionState } from "../ui/connection-state.ts";

test("connection labels distinguish unopened, pending and finished attempts", () => {
  assert.equal(connectionState(), "Disconnected");
  assert.equal(connectionState({}), "Connecting…");
  const meta = { connected: false, finished: false } as any;
  assert.equal(connectionState({ meta }), "Connecting…");
  assert.equal(
    connectionState({ meta: { ...meta, connected: true } }),
    "Connected",
  );
  assert.equal(
    connectionState({ meta: { ...meta, finished: true } }),
    "Disconnected",
  );
  assert.equal(
    connectionState({ meta: { ...meta, connected: true, finished: true } }),
    "Disconnected",
  );
  // A reconnect's new incarnation does not inherit the previous failure label.
  assert.equal(connectionState({}), "Connecting…");
});
