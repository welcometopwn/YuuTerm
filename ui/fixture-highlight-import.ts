import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap } from "./types";
export async function checkHighlightImport(
  until: (f: () => boolean, label: string) => Promise<void>,
  checks: string[],
) {
  const button = (text: string) =>
    [
      ...document.querySelectorAll<HTMLButtonElement>("dialog[open] button"),
    ].find((b) => b.textContent === text)!;
  const current = async () =>
    JSON.stringify((await invoke<Bootstrap>("bootstrap")).prefs.highlighting);
  const before = await current();
  const open = async () => {
    document
      .querySelector<HTMLButtonElement>('[aria-label="Settings"]')!
      .click();
    await until(
      () => !!document.getElementById("settings-tab-highlighting"),
      "Settings is available in the real WebView",
    );
    document.getElementById("settings-tab-highlighting")!.click();
  };
  const read = async () => {
    button("Import MobaXterm syntax").click();
    await until(
      () =>
        !!document.querySelector(
          'dialog[aria-label="Review MobaXterm syntax"]',
        ),
      "Synthetic syntax file produces an explicit native-app preview",
    );
    const text = document.querySelector(
      'dialog[aria-label="Review MobaXterm syntax"]',
    )!.textContent!;
    if (!text.includes("2 accepted") || !text.includes("5 rejected"))
      throw Error("Unexpected syntax preview counts");
    if (text.includes("private-fixture-value"))
      throw Error("Rejected property value leaked into preview");
    checks.push(
      "Preview lists accepted patterns and five rejection reasons without unrelated values",
    );
  };
  await open();
  await read();
  button("Cancel import").click();
  await until(
    () =>
      !document.querySelector('dialog[aria-label="Review MobaXterm syntax"]'),
    "Cancel import discards its review",
  );
  if ((await current()) !== before)
    throw Error("Preview changed persisted settings");
  await read();
  button("Add to draft").click();
  await until(
    () =>
      !document.querySelector('dialog[aria-label="Review MobaXterm syntax"]'),
    "Explicit addition returns to the Settings draft",
  );
  if ((await current()) !== before)
    throw Error("Adding to draft saved settings");
  document
    .querySelector<HTMLButtonElement>(
      'dialog[aria-label="Settings"] .dialog-header button',
    )!
    .click();
  await until(
    () => !document.querySelector('dialog[aria-label="Settings"]'),
    "Closing Settings discards the imported draft",
  );
  if ((await current()) !== before)
    throw Error("Discarded import changed settings");
  await open();
  await read();
  button("Add to draft").click();
  await until(
    () =>
      !document.querySelector('dialog[aria-label="Review MobaXterm syntax"]'),
    "Accepted rules return to the draft before Save",
  );
  button("Save settings").click();
  await until(
    () => !document.querySelector('dialog[aria-label="Settings"]'),
    "Save explicitly commits imported rules",
  );
  const after = (await invoke<Bootstrap>("bootstrap")).prefs.highlighting;
  if (
    after.rules.length !== JSON.parse(before).rules.length + 2 ||
    after.enabled !== JSON.parse(before).enabled
  )
    throw Error("Import replaced rules or enabled highlighting");
  checks.push(
    "Additive save preserves existing rules and highlighting switches",
  );
  await open();
  const count = () =>
    document.querySelectorAll(".highlight-rule:not(.ip-highlight-rule)").length;
  for (let n = count(); n < 12; n++) {
    button("Add color rule").click();
    await until(() => count() === n + 1, "Draft rule capacity is bounded");
  }
  await read();
  if (!button("Add to draft").disabled)
    throw Error("Oversized addition was not blocked");
  checks.push(
    "Import exceeding 12 total rules is blocked without silently dropping entries",
  );
  button("Cancel import").click();
  await until(
    () =>
      !document.querySelector('dialog[aria-label="Review MobaXterm syntax"]'),
    "Capacity review can be cancelled",
  );
  document
    .querySelector<HTMLButtonElement>(
      'dialog[aria-label="Settings"] .dialog-header button',
    )!
    .click();
  await until(
    () => !document.querySelector('dialog[aria-label="Settings"]'),
    "Capacity-test draft discarded",
  );
  await open();
  await read(); // retain the actual preview in the native capture
}
