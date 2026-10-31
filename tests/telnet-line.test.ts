import test from "node:test";
import assert from "node:assert/strict";
import net from "node:net";
import { once } from "node:events";
import { TelnetLineDraft } from "../ui/telnet-line-policy.ts";

test("line drafts are bounded, valid Unicode and never silently trim or execute pasted controls", () => {
  const d = new TelnetLineDraft();
  assert.equal(d.change("界".repeat(1365)), null);
  const before = d.text;
  for (const invalid of [
    "界".repeat(1366),
    "a\ncommand",
    "\x1b[A",
    "\x7f",
    "\ud800",
    "\udfff",
  ]) {
    assert.ok(d.change(invalid));
    assert.equal(d.text, before);
  }
  assert.ok(d.append("界"));
  assert.equal(d.text, before);
  d.clear();
  assert.equal(d.text, "");
});
test("real TCP peer sees no draft until explicit submission and only the final edited Unicode line", async () => {
  const server = net.createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const client = net.createConnection(
    (server.address() as net.AddressInfo).port,
    "127.0.0.1",
  );
  const [peer] = (await once(server, "connection")) as [net.Socket];
  client.setTimeout(3000,()=>client.destroy(new Error("TCP fixture deadline")));
  peer.setTimeout(3000,()=>peer.destroy());
  try {
    const d = new TelnetLineDraft();
    d.change("wrong");
    d.change("final Grüße 界");
    let received = Buffer.alloc(0);
    peer.on("data", (b) => {
      received = Buffer.concat([received, b]);
      peer.write("ack");
    });
    const ack = once(client, "data");
    client.write("BARRIER");
    await ack;
    assert.equal(received.toString(), "BARRIER");
    const done = once(client, "data");
    await d.submit(
      (text) =>
        new Promise<void>((resolve, reject) =>
          client.write(text, (e) => (e ? reject(e) : resolve())),
        ),
    );
    await done;
    assert.equal(received.toString(), "BARRIERfinal Grüße 界\r");
    assert.equal(d.text, "");
  } finally {
    client.destroy();
    peer.destroy();
    server.close();
  }
});
test("a busy send cannot duplicate a line; failure retains a draft and retry is explicit", async () => {
  const d = new TelnetLineDraft();
  d.change("one");
  let release!: () => void;
  let calls = 0;
  const pending = d.submit(async () => {
    calls++;
    await new Promise<void>((r) => {
      release = r;
    });
  });
  assert.equal(
    await d.submit(async () => {
      calls++;
    }),
    false,
  );
  assert.ok(d.change("two"));
  release();
  await pending;
  assert.equal(calls, 1);
  assert.equal(d.text, "");
  d.change("retry");
  await assert.rejects(
    d.submit(async () => {
      throw Error("synthetic queue failure");
    }),
  );
  assert.equal(d.text, "retry");
  assert.equal(d.busy, false);
  d.clear();
  assert.equal(new TelnetLineDraft().text, "");
});
