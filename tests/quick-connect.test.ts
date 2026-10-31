import test from "node:test";
import assert from "node:assert/strict";
import {
  parseDestination,
  explicitDestination,
  findBookmark,
  quickProfile,
} from "../ui/quick-connect.ts";
import { blankProfile } from "../ui/types.ts";

test("Quick connect parses users, ports, schemes and IPv6", () => {
  assert.deepEqual(parseDestination("alice@server.test"), {
    protocol: "Ssh",
    host: "server.test",
    port: 22,
    username: "alice",
  });
  assert.deepEqual(parseDestination(" host.test:2222 "), {
    protocol: "Ssh",
    host: "host.test",
    port: 2222,
    username: "",
  });
  assert.deepEqual(parseDestination("RDP://DOMAIN\\bob@192.0.2.5/"), {
    protocol: "Rdp",
    host: "192.0.2.5",
    port: 3389,
    username: "DOMAIN\\bob",
  });
  assert.deepEqual(parseDestination("vnc://[fd00::5]:5901"), {
    protocol: "Vnc",
    host: "fd00::5",
    port: 5901,
    username: "",
  });
  assert.equal(parseDestination("fd00::5")?.host, "fd00::5");
  assert.equal(parseDestination("a@b@host.test")?.username, "a@b");
  assert.equal(parseDestination("telnet://switch")?.port, 23);
});

test("Quick connect refuses option-like, spaced and malformed input", () => {
  for (const bad of [
    "",
    "-oProxyCommand=x",
    "user@-host",
    "two words",
    "host:0",
    "host:65536",
    "host:abc",
    "ftp://host",
    "serial://COM3",
    "@host",
    "host/path",
    "[not-ipv6]",
    "x".repeat(301),
    "host\u0007",
  ])
    assert.equal(parseDestination(bad), null, bad);
});

test("Only explicit destinations replace the session search", () => {
  assert.equal(explicitDestination("alice@server"), true);
  assert.equal(explicitDestination("server:22"), true);
  assert.equal(explicitDestination("rdp://pc"), true);
  assert.equal(explicitDestination("prod"), false, "plain words stay a search");
  assert.equal(explicitDestination("alice@"), false);
});

test("Quick connect reuses the newest matching bookmark and names new ones", () => {
  const d = parseDestination("rdp://admin@pc.test:3390")!;
  const a = { ...quickProfile(d), id: 3 };
  const b = { ...quickProfile(d), id: 9 };
  const other = { ...blankProfile(), id: 12, host: "pc.test", port: 3390 };
  assert.equal(findBookmark([a, other, b], d)?.id, 9);
  assert.equal(findBookmark([other], d), undefined);
  assert.equal(
    findBookmark([{ ...a, host: "PC.TEST" }], d)?.id,
    3,
    "host names compare case-insensitively",
  );
  assert.equal(quickProfile(d).name, "RDP admin@pc.test:3390");
  assert.equal(quickProfile(parseDestination("u@h.test")!).name, "u@h.test");
  assert.equal(quickProfile(d).id, 0);
});
