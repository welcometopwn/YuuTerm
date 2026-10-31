import { test } from "node:test";
import assert from "node:assert/strict";
import { statusParts, statusSummary } from "../ui/status-text.ts";

const rdp516 =
  "Remote Desktop can't connect to the remote computer for one of these reasons: " +
  "1) Remote access to the server is not enabled 2) The remote computer is turned off " +
  "3) The remote computer is not available on the network Make sure the remote computer " +
  "is turned on and connected to the network, and that remote access is enabled. (code 516)";

test("numbered RDP reasons become a list with advice and code", () => {
  assert.deepEqual(statusParts(rdp516), {
    lead: "Remote Desktop can't connect to the remote computer for one of these reasons",
    reasons: [
      "Remote access to the server is not enabled",
      "The remote computer is turned off",
      "The remote computer is not available on the network",
    ],
    advice:
      "Make sure the remote computer is turned on and connected to the network, and that remote access is enabled.",
    code: "code 516",
  });
  assert.equal(
    statusSummary(rdp516),
    "Remote Desktop can't connect to the remote computer (code 516)",
  );
});

test("plain messages stay whole", () => {
  assert.deepEqual(statusParts("Disconnected"), {
    lead: "Disconnected",
    reasons: [],
    advice: "",
    code: "",
  });
  assert.equal(
    statusSummary("The user name or password was not accepted (code 2055)"),
    "The user name or password was not accepted (code 2055)",
  );
  assert.equal(statusSummary("Version 1) only"), "Version 1) only");
});
