// Native editor evidence only. No packaged X server or graphical X client exists
// in this fixture; the genuine SSH channel tests are a separate protocol gate.
import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap } from "./types";
export async function checkX11Settings(
  until: (
    predicate: () => boolean | Promise<boolean>,
    name: string,
  ) => Promise<void>,
  checks: string[],
) {
  const probe = await invoke<{ available: boolean; message: string }>(
    "x11_probe",
  );
  if (
    probe.available ||
    !probe.message.includes("no installed VcXsrv or X410 detected")
  )
    throw new Error(
      "X11 fixture requires the actual no-installed-server native result",
    );
  checks.push(
    "Registered native X11 command reports no installed VcXsrv/X410 without a mocked response",
  );
  document.getElementById("session-tab-connection")!.click();
  const checkbox = () =>
    document.querySelector<HTMLInputElement>(
      'input[aria-label="Forward X11 applications"]',
    );
  await until(
    () =>
      !!checkbox() &&
      checkbox()!.disabled &&
      !checkbox()!.checked &&
      document.querySelector(".x11-settings details small")?.textContent ===
        probe.message &&
      document.querySelector(".x11-settings [role=status]")?.textContent ===
        "X11 is unavailable. Check the setup details below.",
    "Native SSH editor explains unavailable X11 and refuses enabling forwarding",
  );
  checkbox()!.click();
  await until(
    async () =>
      !checkbox()!.checked &&
      !(await invoke<Bootstrap>("bootstrap")).prefs.profiles[0].ssh.x11_forward,
    "Unavailable X server never enables the bookmark or changes the active SSH session",
  );
  const button = Array.from(
    document.querySelectorAll<HTMLButtonElement>(".x11-settings button"),
  )[0];
  button.click();
  await until(
    () => button.disabled,
    "Native X11 recheck enters a pending state before completing",
  );
  await until(
    () =>
      !button.disabled &&
      checkbox()!.disabled &&
      document.querySelector(".x11-settings details small")?.textContent ===
        probe.message,
    "X server readiness can be rechecked without connecting or launching a server",
  );
  checks.push(
    "X11 capture is unavailable-server UI evidence only; no graphical X application was rendered",
  );
}
