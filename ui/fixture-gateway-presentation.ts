import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap } from "./types";

/** Real native fixture only; no protocol state is synthesized here. */
export async function checkGatewayPresentation(checks: string[]) {
  const profile = (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0];
  if (!profile.gateways?.length) return;
  [...document.querySelectorAll<HTMLButtonElement>("button")]
    .find((b) => b.textContent?.trim() === "Sessions")
    ?.click();
  await new Promise((r) => setTimeout(r, 80));
  const via = document.querySelector(".endpoint-via");
  if (
    !via?.textContent?.startsWith("via ") ||
    !document.querySelector(".gateway-marker")
  )
    throw Error("Gateway session header or sidebar marker missing");
  checks.push("Real session header and sidebar identify the SSH gateway");
}

/** Redact only labels after protocol assertions; leave real framebuffer intact. */
export async function redactGatewayLabels() {
  const profile = (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0];
  const replacements = [
    [profile.host, "<test-host>"],
    [profile.username, "<test-account>"],
    ...(profile.gateways ?? []).flatMap((g): string[][] =>
      g.kind === "Manual"
        ? [
            [g.host, "<gateway>"],
            [g.username, "<gateway-account>"],
          ]
        : [],
    ),
  ]
    .filter(([value]) => value.length)
    .sort((a, b) => b[0].length - a[0].length);
  const replace = (value: string) =>
    replacements.reduce((s, [from, to]) => s.replaceAll(from, to), value);
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  while (walker.nextNode())
    walker.currentNode.textContent = replace(
      walker.currentNode.textContent ?? "",
    );
  document.querySelectorAll<HTMLElement>("[title]").forEach((el) => {
    el.title = replace(el.title);
  });
  document
    .querySelectorAll<HTMLInputElement>("dialog[open] input")
    .forEach((el) => {
      el.value = replace(el.value);
    });
}

export async function showGatewayEditor(checks: string[]) {
  const profile = (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0];
  if (!profile.gateways?.length) return;
  document
    .querySelector<HTMLButtonElement>(".session-row button.edit")!
    .click();
  const end = Date.now() + 5000;
  while (
    !document.querySelector(".gateway-settings .gateway-path") &&
    Date.now() < end
  )
    await new Promise((r) => setTimeout(r, 50));
  const gateway = document.querySelector(".gateway-settings .gateway-path");
  if (!gateway) throw Error("Network bookmark editor has no gateway section");
  gateway.scrollIntoView({ block: "center" });
  checks.push("Network bookmark editor shows the SSH gateway and ordered path");
}
