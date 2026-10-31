import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap, Profile } from "./types";
export async function checkSerialImport(
  until: (f: () => boolean, label: string) => Promise<void>,
  checks: string[],
) {
  const before = (await invoke<Bootstrap>("bootstrap")).prefs;
  const preview = await invoke<{
    token: number;
    profiles: Profile[];
    skipped: unknown[];
  }>("profiles_import_preview");
  if (preview.profiles.length !== 4 || preview.skipped.length !== 4)
    throw Error("Wrong serial preview counts");
  const p = preview.profiles[1];
  if (
    p.serial.data_bits !== 6 ||
    p.serial.stop_bits !== 2 ||
    p.serial.parity !== "Odd" ||
    p.serial.flow_control !== "Software"
  )
    throw Error("Wrong SERIAL mapping");
  if (
    preview.profiles.some(
      (p) =>
        p.credential_id || p.logging.enabled || p.serial.dtr || p.serial.rts,
    )
  )
    throw Error("Import activated local actions");
  await invoke("profiles_import_cancel", { token: preview.token });
  checks.push(
    "Actual backend preview maps supported SERIAL variants and lists Mark/Space/DSR-DTR/unknown stop bits as rejected",
  );
  const button = (text: string) =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent?.trim() === text,
    )!;
  button("Sessions").click();
  const open = async () => {
    document
      .querySelector<HTMLButtonElement>(
        '[aria-label="Session import and export"]',
      )!
      .click();
    await until(() => !!button("Import sessions"), "Import menu opens");
    button("Import sessions").click();
    await until(
      () =>
        document.querySelectorAll(".session-import-list details").length === 4,
      "Actual native review displays four SERIAL additions",
    );
  };
  await open();
  document.querySelectorAll<HTMLDetailsElement>(
    ".session-import-list details",
  )[1].open = true;
  if (
    !document
      .querySelector(".session-import-list")!
      .textContent!.includes(
        "6 data bits · Odd parity · 2 stop bits · Software flow control",
      )
  )
    throw Error("Review omitted framing/flow");
  button("Cancel").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Cancel discards review",
  );
  if (
    JSON.stringify((await invoke<Bootstrap>("bootstrap")).prefs) !==
    JSON.stringify(before)
  )
    throw Error("Cancel changed saved profiles");
  await open();
  [...document.querySelectorAll<HTMLButtonElement>("dialog button")]
    .find((b) => b.textContent?.trim() === "Import sessions")!
    .click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Explicit Import applies the reviewed metadata",
  );
  const after = (await invoke<Bootstrap>("bootstrap")).prefs;
  if (
    after.profiles.length !== 8 ||
    after.profiles
      .slice(4)
      .some((p) => p.id <= 4 || p.logging.enabled || p.credential_id)
  )
    throw Error("Merge did not preserve safe fresh identities");
  if (document.querySelectorAll(".terminal-host").length !== 4)
    throw Error("Imported sessions automatically connected");
  checks.push(
    "Additive merge preserves original live terminals and starts no SERIAL connection",
  );
  await open();
  document.querySelectorAll<HTMLDetailsElement>(
    ".session-import-list details",
  )[1].open = true;
}
