import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap } from "./types";
// Native editor qualification only: no password prompt/storage/IPC exists here.
export async function checkSshPasswordEditor(
  until: (
    predicate: () => boolean | Promise<boolean>,
    label: string,
  ) => Promise<void>,
  checks: string[],
) {
  document.querySelector<HTMLElement>(".brand")!.click();
  document.querySelector<HTMLButtonElement>(".sidebar .new-session")!.click();
  await until(
    () => !!document.querySelector("dialog[open]"),
    "New SSH editor opens for password mode check",
  );
  const select = Array.from(
    document.querySelectorAll<HTMLSelectElement>("dialog[open] select"),
  ).find((s) => Array.from(s.options).some((o) => o.value === "WindowsAgent"))!;
  if (!select) throw Error("SSH authentication selector is missing");
  select.value = "Password";
  select.dispatchEvent(new Event("change", { bubbles: true }));
  await until(
    () =>
      select.value === "Password" &&
      document
        .querySelector("dialog[open]")!
        .textContent!.includes("Credential prompt integration is pending"),
    "SSH Password option explains the pending desktop credential hook",
  );
  const labels = Array.from(
    document.querySelectorAll("dialog[open] label"),
  ).map((l) => l.textContent);
  if (
    labels.some((l) => l?.includes("Private-key path")) ||
    document.querySelector("dialog[open] input[type=password]")
  )
    throw Error("Password mode showed an identity field or secret entry");
  checks.push(
    "Password metadata mode hides key-file fields and does not collect secrets",
  );
  const profiles = (await invoke<Bootstrap>("bootstrap")).prefs.profiles;
  if (profiles.some((p) => p.ssh.auth === "Password"))
    throw Error("Unsaved password editor changed fixture bookmarks");
  checks.push(
    "Password selection remains an unsaved draft; live key-authenticated sessions are unchanged",
  );
}
