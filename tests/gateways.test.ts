import test from "node:test";
import assert from "node:assert/strict";
import {
  MAX_GATEWAYS,
  brokenGateways,
  gatewayDependents,
  gatewayPath,
  importGatewayNote,
  savedChoices,
  viaLabel,
} from "../ui/gateways.ts";
import type { Profile } from "../ui/types.ts";

const profile = (id: number, extra: Partial<Profile> = {}): Profile =>
  ({
    id,
    name: `S${id}`,
    protocol: "Ssh",
    host: `host${id}`,
    port: 22,
    username: "admin",
    font_size: null,
    statistics_enabled: null,
    logging: { enabled: false, file: "" },
    telnet: { local_line_editing: false },
    serial: {} as Profile["serial"],
    ssh: {
      auth: "WindowsAgent",
      agent_pipe: "",
      identity_fingerprint: "",
      config_host: "",
      config_file: "",
      key_file: "",
    },
    ...extra,
  }) as Profile;

const manual = (host: string) => ({
  kind: "Manual" as const,
  host,
  port: 22,
  username: "jump",
  auth: "WindowsAgent" as const,
  key_file: "",
  agent_pipe: "",
  identity_fingerprint: "",
});

test("gateway path names every hop in connection order", () => {
  const bastion = profile(2, { name: "bastion" });
  const direct = profile(1, { host: "target", port: 2222 });
  assert.equal(gatewayPath(direct, [direct]), "This PC → target:2222");
  const one = profile(1, {
    host: "target",
    gateways: [{ kind: "Saved", profile_id: 2 }],
  });
  assert.equal(
    gatewayPath(one, [one, bastion]),
    "This PC → bastion → target:22",
  );
  const two = profile(1, {
    host: "target",
    gateways: [{ kind: "Saved", profile_id: 2 }, manual("jump2")],
  });
  assert.equal(
    gatewayPath(two, [two, bastion]),
    "This PC → bastion → jump2 → target:22",
  );
  assert.equal(MAX_GATEWAYS, 4);
});

test("via label is empty, the gateway name, or a count", () => {
  const bastion = profile(2, { name: "bastion" });
  assert.equal(viaLabel(profile(1), []), "");
  const one = profile(1, { gateways: [{ kind: "Saved", profile_id: 2 }] });
  assert.equal(viaLabel(one, [one, bastion]), "via bastion");
  const manualOne = profile(1, { gateways: [manual("jump.example")] });
  assert.equal(viaLabel(manualOne, [manualOne]), "via jump.example");
  const two = profile(1, {
    gateways: [{ kind: "Saved", profile_id: 2 }, manual("jump2")],
  });
  assert.equal(viaLabel(two, [two, bastion]), "via 2 gateways");
});

test("broken gateways flag deleted, non-SSH and self references", () => {
  const telnet = profile(3, { protocol: "Telnet" });
  const p = profile(1, {
    gateways: [
      { kind: "Saved", profile_id: 2 },
      { kind: "Saved", profile_id: 9 },
      { kind: "Saved", profile_id: 3 },
      { kind: "Saved", profile_id: 1 },
      manual("ok"),
    ],
  });
  assert.deepEqual(brokenGateways(p, [p, profile(2), telnet]), [1, 2, 3]);
});

test("saved choices are other saved SSH sessions", () => {
  const p = profile(1);
  const choices = savedChoices(p, [
    p,
    profile(2),
    profile(3, { protocol: "Rdp" }),
    profile(4, { temporary: true }),
  ]);
  assert.deepEqual(
    choices.map((c) => c.id),
    [2],
  );
});

test("dependents count surviving sessions that use a deleted one", () => {
  const uses = (id: number, ref: number) =>
    profile(id, { gateways: [{ kind: "Saved", profile_id: ref }] });
  const all = [uses(1, 3), uses(2, 3), profile(3), uses(4, 5), profile(5)];
  assert.equal(gatewayDependents(all, [3]), 2);
  assert.equal(gatewayDependents(all, [1, 3]), 1);
  assert.equal(gatewayDependents(all, [1, 2, 3]), 0);
  assert.equal(gatewayDependents(all, [4]), 0);
});

test("import preview names every gateway with account and port", () => {
  assert.equal(importGatewayNote(profile(1)), "");
  const p = profile(1, {
    gateways: [
      { ...manual("bastion.example"), port: 2222 },
      { ...manual("jump2"), username: "" },
    ],
  });
  assert.equal(
    importGatewayNote(p),
    "Through SSH gateways: jump@bastion.example:2222 → jump2:22",
  );
});
