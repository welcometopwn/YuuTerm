import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap, Profile } from "./types";
export async function checkTransfer(
  ids: number[],
  until: (f: () => boolean | Promise<boolean>, name: string) => Promise<void>,
  checks: string[],
) {
  const button = (text: string) =>
    Array.from(document.querySelectorAll<HTMLButtonElement>("button")).find(
      (b) => b.textContent?.trim() === text,
    )!;
  const menu = async (text: string) => {
    document
      .querySelector<HTMLButtonElement>(
        '[aria-label="Session import and export"]',
      )!
      .click();
    await until(() => !!button(text), `Session menu offers ${text}`);
    button(text).click();
  };
  button("Sessions").click();
  await until(
    () => !!document.querySelector('[aria-label="Session import and export"]'),
    "Session transfer menu appears in the sidebar",
  );
  const before = (await invoke<Bootstrap>("bootstrap")).prefs;
  await menu("Export sessions");
  await until(
    () => !!document.querySelector('dialog[aria-label="Export sessions"]'),
    "Export explains private metadata before writing a file",
  );
  button("Cancel").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Cancel dismisses export review",
  );
  if (
    JSON.stringify((await invoke<Bootstrap>("bootstrap")).prefs) !==
    JSON.stringify(before)
  )
    throw new Error("Export cancel changed preferences");
  checks.push("Export cancel preserves all saved preferences");
  await menu("Export sessions");
  await until(() => !!button("Export file"), "Export confirmation is explicit");
  button("Export file").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Export writes the actual isolated JSON file",
  );
  const probe = await invoke<{ token: number; format: string }>(
    "profiles_import_preview",
  );
  const moba = probe.format === "MobaXterm";
  const additions = moba ? 5 : 0;
  await invoke("profiles_import_cancel", { token: probe.token });
  await menu("Import sessions");
  await until(
    () =>
      document.querySelectorAll(".session-import-list details").length ===
      additions,
    "Actual import file is parsed into reviewed additions",
  );
  if (!moba) {
    await until(
      () => !!document.querySelector('[aria-label="Duplicate import entries"]'),
      "Exported existing sessions are reported as duplicates",
    );
    const apply = document.querySelector<HTMLButtonElement>(
      "dialog button.primary",
    );
    if (!apply?.disabled)
      throw new Error("Duplicate-only import can be applied");
    checks.push(
      "Round-trip import skips existing bookmarks without creating copies",
    );
    return;
  }
  if (moba) {
    if (
      document.querySelectorAll('[aria-label="Skipped import entries"] p')
        .length !== 4
    )
      throw new Error("Unsupported/malformed MobaXterm entries disappeared");
    checks.push(
      "MobaXterm preview includes all five protocols and four explicit rejected entries",
    );
  }
  document
    .querySelector<HTMLButtonElement>('dialog [aria-label="Close dialog"]')!
    .click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Close discards import preview",
  );
  if (
    JSON.stringify((await invoke<Bootstrap>("bootstrap")).prefs) !==
    JSON.stringify(before)
  )
    throw new Error("Import preview changed preferences");
  checks.push("Import review and close leave existing preferences unchanged");
  await menu("Import sessions");
  await until(
    () => !!document.querySelector('dialog[aria-label="Import sessions"]'),
    "Import review can be reopened",
  );
  const apply = Array.from(
    document.querySelectorAll<HTMLButtonElement>("dialog button"),
  ).find((b) => b.textContent?.trim() === "Import sessions")!;
  apply.click();
  await until(
    async () =>
      !document.querySelector("dialog[open]") &&
      (await invoke<Bootstrap>("bootstrap")).prefs.profiles.length ===
        4 + additions,
    "Confirm appends reviewed sessions without replacing originals",
  );
  const after = (await invoke<Bootstrap>("bootstrap")).prefs;
  if (moba) {
    const icon = after.profiles[4].icon;
    if (!icon?.startsWith("data:image/png;base64,"))
      throw new Error("Embedded MobaXterm PNG icon was lost on apply");
    const image = new Image();
    image.src = icon;
    await image.decode();
    if (image.naturalWidth !== 1 || image.naturalHeight !== 1)
      throw new Error("Imported synthetic PNG dimensions changed");
    checks.push(
      "Embedded MobaXterm PNG survives apply and decodes in the native WebView",
    );
  }
  if (
    JSON.stringify(after.profiles.slice(0, 4)) !==
    JSON.stringify(before.profiles)
  )
    throw new Error("Original bookmarks were modified");
  if (
    after.profiles
      .slice(4)
      .some(
        (p) =>
          p.id <= 4 ||
          (!moba && !p.name.endsWith(" (imported)")) ||
          p.logging.enabled ||
          p.logging.file,
      )
  )
    throw new Error("Imported profiles lack fresh identity or logging reset");
  checks.push(
    "Originals are unchanged; additions have fresh IDs, distinct names and disabled logging",
  );
  if (
    ids.some((id) => !document.querySelector(`[data-live-id="${id}"]`)) ||
    document.querySelectorAll(".terminal-host").length !== 4
  )
    throw new Error("Import started or replaced live sessions");
  checks.push(
    "All four original live sessions survive; no imported session auto-connects",
  );
  const first = await invoke<{ token: number; profiles: Profile[] }>(
    "profiles_import_preview",
  );
  const second = await invoke<{ token: number; profiles: Profile[] }>(
    "profiles_import_preview",
  );
  let rejected = false;
  try {
    await invoke("profiles_import_apply", { token: first.token });
  } catch {
    rejected = true;
  }
  if (!rejected) throw new Error("Stale import review was accepted");
  checks.push("An older server-side review token cannot be applied");
  await invoke("profiles_import_cancel", { token: second.token });
  rejected = false;
  try {
    await invoke("profiles_import_apply", { token: second.token });
  } catch {
    rejected = true;
  }
  if (
    !rejected ||
    (await invoke<Bootstrap>("bootstrap")).prefs.profiles.length !==
      4 + additions
  )
    throw new Error("Cancelled preview was applied");
  checks.push("Cancelled review cannot be replayed and changes no bookmarks");
  await menu("Import sessions");
  await until(
    () =>
      document.querySelectorAll(".session-import-list details").length === 0 &&
      document
        .querySelector('[aria-label="Duplicate import entries"]')
        ?.textContent?.includes("5 duplicate") === true,
    "Repeated import skips every already imported session",
  );
  if (
    !document.querySelector<HTMLButtonElement>("dialog button.primary")
      ?.disabled
  )
    throw new Error("Repeated import remains enabled");
  checks.push(
    "Duplicate-only review disables Import and preserves existing bookmarks",
  );
}
