import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap } from "./types";
export async function checkLogging(
  _ids: number[],
  until: (p: () => boolean | Promise<boolean>, label: string) => Promise<void>,
  checks: string[],
) {
  const initial = (await invoke<Bootstrap>("bootstrap")).prefs;
  const profile = initial.profiles[0];
  const customPath = profile.logging.file;
  const pane = () =>
    document.querySelector<HTMLElement>(
      `.pane[data-profile-id="${profile.id}"]`,
    )!;
  const host = () => pane().querySelector<HTMLElement>(".terminal-host")!;
  const id = () => Number(pane().dataset.liveId);
  const button = (text: string) =>
    Array.from(document.querySelectorAll<HTMLButtonElement>("button")).find(
      (b) => b.textContent?.trim() === text,
    )!;
  const field = () =>
    document.querySelector<HTMLInputElement>('[aria-label="Log file path"]')!;
  const checkbox = () =>
    document.querySelector<HTMLInputElement>(
      '[aria-label="Log terminal output"]',
    )!;
  const saved = async () =>
    (await invoke<Bootstrap>("bootstrap")).prefs.profiles.find(
      (p) => p.id === profile.id,
    )!;
  const setFile = (value: string) => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(field(), value);
    field().dispatchEvent(new Event("input", { bubbles: true }));
  };
  const open = async () => {
    const tab = Array.from(document.querySelectorAll<HTMLElement>(".tab")).find(
      (t) => t.textContent?.includes(profile.name),
    )!;
    tab.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        clientX: 320,
        clientY: 65,
      }),
    );
    await until(
      () => !!button("Edit session"),
      "Session context opens its settings",
    );
    button("Edit session").click();
    await until(
      () => !!document.getElementById("session-tab-terminal"),
      "Session editor has a Terminal tab",
    );
    document.getElementById("session-tab-terminal")!.click();
    await until(
      () =>
        !!field() && !field().closest<HTMLElement>('[role="tabpanel"]')?.hidden,
      "Automatic logging controls are in session Terminal settings",
    );
  };
  const save = async () => {
    await new Promise((r) => setTimeout(r, 40));
    button("Save session").click();
    await until(
      () => !document.querySelector("dialog[open]"),
      "Explicit Save commits the session logging draft",
    );
  };
  const reconnect = async () => {
    const previous = id();
    await invoke("session_disconnect", { id: previous });
    await until(
      () => !!pane().querySelector("button.connect"),
      "Disconnected terminal exposes reconnect",
    );
    pane().querySelector<HTMLButtonElement>("button.connect")!.click();
    await until(
      () => id() !== previous && !!pane().querySelector(".dot.connected"),
      "Reconnect creates a fresh real SSH connection",
    );
  };
  const send = async (marker: string) => {
    const before = Number(host().dataset.logBytes);
    await invoke("session_input", {
      id: id(),
      bytes: Array.from(new TextEncoder().encode(marker + "\r\n")),
    });
    await until(
      () =>
        (pane().querySelector(".xterm-rows")?.textContent ?? "").includes(
          marker,
        ),
      "Remote terminal echoes " + marker,
    );
    if (marker !== "logging-disabled-marker")
      await until(
        () => Number(host().dataset.logBytes) > before,
        "Automatic writer records " + marker,
      );
  };
  await until(
    () => !!pane()?.querySelector(".dot.connected"),
    "Initial SSH terminal is connected",
  );
  if (profile.logging.enabled || host().dataset.logActive === "true")
    throw new Error("Logging started without a saved checkbox");
  checks.push("Existing sessions remain default-off");
  await open();
  checkbox().click();
  setFile("");
  button("Cancel").click();
  if ((await saved()).logging.enabled)
    throw new Error("Cancelled logging draft was persisted");
  checks.push("Cancel does not enable automatic logging");
  await open();
  checkbox().click();
  setFile("");
  await save();
  const enabled = (await saved()).logging;
  if (!enabled.enabled || enabled.file !== "")
    throw new Error("Default path selection was not saved");
  checks.push(
    "Enabled checkbox and empty default-path override persist in the fixture backend",
  );
  if (host().dataset.logActive === "true")
    throw new Error("Setting unexpectedly retroactively captured output");
  checks.push(
    "Current connection is unchanged; the saved option applies on next connect",
  );
  await reconnect();
  await until(
    () =>
      host().dataset.logActive === "true" &&
      /Documents\\YuuTerm\\SSH fixture 1\\\d{4}-/.test(
        host().dataset.logPath ?? "",
      ),
    "Connect automatically writes a timestamped file under Documents/YuuTerm/session name",
  );
  const firstPath = host().dataset.logPath;
  await send("default-auto-one");
  await reconnect();
  await until(
    () =>
      host().dataset.logActive === "true" &&
      !!host().dataset.logPath &&
      host().dataset.logPath !== firstPath,
    "Reconnect automatically creates a different default file",
  );
  await send("default-auto-two");
  await open();
  if (!checkbox().checked || field().value !== "")
    throw new Error("Saved default logging setting did not reopen");
  setFile("relative.log");
  button("Save session").click();
  await until(
    () =>
      !!document.querySelector("dialog[open]") &&
      /absolute Windows/.test(
        document.querySelector(".app-notice")?.textContent ?? "",
      ),
    "Backend rejects a relative custom log path without closing the draft",
  );
  if ((await saved()).logging.file !== "")
    throw new Error("Invalid custom path replaced saved settings");
  checks.push("Invalid logging save preserves previous persisted values");
  setFile(customPath);
  await save();
  document
    .querySelector<HTMLButtonElement>('[aria-label="Dismiss notification"]')
    ?.click();
  await reconnect();
  await until(
    () =>
      host().dataset.logActive === "true" &&
      host().dataset.logPath === customPath,
    "Custom file path is used automatically on connection",
  );
  await send("custom-auto-one");
  await reconnect();
  await until(
    () =>
      host().dataset.logActive === "true" &&
      host().dataset.logPath === customPath,
    "Custom path is reused for append on reconnect",
  );
  await send("custom-auto-two");
  if (
    document.querySelectorAll('[aria-label="Output logging enabled"]')
      .length !== 1
  )
    throw new Error("Logging leaked to another session");
  checks.push("Only the configured session gets the small logging indicator");
  await open();
  checkbox().click();
  await save();
  await reconnect();
  await until(
    () =>
      host().dataset.logActive === "false" &&
      !pane().querySelector('[aria-label="Output logging enabled"]'),
    "Unchecking logging disables it on the next connection",
  );
  await send("logging-disabled-marker");
  host().dispatchEvent(
    new MouseEvent("contextmenu", {
      bubbles: true,
      clientX: 400,
      clientY: 200,
    }),
  );
  await until(
    () => !!document.querySelector(".context-menu"),
    "Terminal context menu still opens",
  );
  if (
    document
      .querySelector(".context-menu")!
      .textContent?.includes("Start output log") ||
    button("Pause log") ||
    button("Resume log")
  )
    throw new Error("Old manual logging controls remain");
  checks.push("Manual Start, Pause and Resume controls have been removed");
  document.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
  );
  await open();
  checkbox().click();
  await save();
  await reconnect();
  await until(
    () => host().dataset.logActive === "true",
    "Saved checkbox starts logging again without a separate prompt",
  );
  await send("custom-auto-three");
  await open();
  setFile("C:\\blabla\\custom.log");
  checks.push(
    "Native screenshot shows the requested custom-file example as an unsaved draft",
  );
}
