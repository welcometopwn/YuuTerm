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
  const heights: number[] = [];
  for (const name of ["connection", "terminal", "statistics", "connection"]) {
    document.getElementById(`session-tab-${name}`)!.click();
    await new Promise((r) => setTimeout(r, 80));
    heights.push(
      document.querySelector<HTMLElement>("dialog[open]")!.offsetHeight,
    );
  }
  await until(
    () => heights.every((h) => h === heights[0]),
    "SSH session editor keeps one height across all sections",
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
        .textContent!.includes(
          "A Windows prompt asks for the password when connecting",
        ),
    "SSH Password option explains the Windows prompt at connect time",
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
