import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap } from "./types";
export async function checkLogRotation(
  until: (f: () => boolean, label: string) => Promise<void>,
  checks: string[],
) {
  const profile = (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0];
  const pane = () =>
    document.querySelector<HTMLElement>(
      `.pane[data-profile-id="${profile.id}"]`,
    )!;
  const host = () => pane().querySelector<HTMLElement>(".terminal-host")!;
  const id = () => Number(pane().dataset.liveId);
  const button = (text: string) =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent?.trim() === text,
    )!;
  const input = (name: string) =>
    document.querySelector<HTMLInputElement>(`[aria-label="${name}"]`)!;
  const set = (name: string, value: string) => {
    const field = input(name);
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  };
  const saved = async () =>
    (await invoke<Bootstrap>("bootstrap")).prefs.profiles.find(
      (p) => p.id === profile.id,
    )!.logging;
  const open = async () => {
    [...document.querySelectorAll<HTMLElement>(".tab")]
      .find((t) => t.textContent?.includes(profile.name))!
      .dispatchEvent(
        new MouseEvent("contextmenu", {
          bubbles: true,
          clientX: 320,
          clientY: 65,
        }),
      );
    await until(() => !!button("Edit session"), "Session menu opens");
    button("Edit session").click();
    await until(
      () => !!document.getElementById("session-tab-terminal"),
      "Session editor opens",
    );
    document.getElementById("session-tab-terminal")!.click();
    await until(
      () => !!input("Rotate output logs"),
      "Rotation is available in Terminal settings",
    );
  };
  const save = async () => {
    await new Promise((r) => setTimeout(r, 40));
    button("Save session").click();
    await until(
      () => !document.querySelector("dialog[open]"),
      "Explicit Save closes session draft",
    );
  };
  const reconnect = async () => {
    const previous = id();
    await invoke("session_disconnect", { id: previous });
    await until(
      () => !!pane().querySelector("button.connect"),
      "Disconnect exposes reconnect",
    );
    pane().querySelector<HTMLButtonElement>("button.connect")!.click();
    await until(
      () => id() !== previous && !!pane().querySelector(".dot.connected"),
      "Fresh loopback connection is ready",
    );
  };
  await until(
    () => !!pane()?.querySelector(".dot.connected"),
    "Real loopback terminal is connected",
  );
  if (
    profile.logging.enabled ||
    profile.logging.rotation ||
    host().dataset.logActive === "true"
  )
    throw Error("Logs or rotation started without opt-in");
  await open();
  if (
    input("Rotate output logs").checked ||
    !input("Rotate output logs").disabled
  )
    throw Error("Rotation must start off/disabled with logging off");
  input("Log terminal output").click();
  input("Rotate output logs").click();
  await until(
    () => !!input("Log rotation MiB"),
    "Opt-in reveals bounded rotation controls",
  );
  if (
    input("Log rotation MiB").value !== "16" ||
    input("Log rotation kept files").value !== "4"
  )
    throw Error("Wrong initial rotation values");
  button("Cancel").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Cancel discards opt-in",
  );
  if ((await saved()).rotation) throw Error("Cancel persisted rotation");
  await open();
  input("Log terminal output").click();
  input("Rotate output logs").click();
  await until(() => !!input("Log rotation MiB"), "Rotation controls reopen");
  set("Log rotation MiB", "1");
  set("Log rotation kept files", "2");
  set("Log file path", profile.logging.file);
  await save();
  const settings = await saved();
  if (
    !settings.enabled ||
    settings.rotation?.max_mib !== 1 ||
    settings.rotation.keep_files !== 2
  )
    throw Error("Rotation setting failed to persist");
  if (host().dataset.logActive === "true")
    throw Error("Save changed existing recorder");
  checks.push(
    "Rotation opt-in is explicit, cancelable, per-session and applies on next connection",
  );
  await reconnect();
  await until(
    () => host().dataset.logActive === "true",
    "Reconnect starts the actual output writer",
  );
  const bytes = Array.from(
    new TextEncoder().encode("rotation-synthetic ".repeat(862) + "\r\n"),
  );
  const start = Number(host().dataset.logBytes);
  for (let n = 0; n < 144; n++) {
    const before = Number(host().dataset.logBytes);
    await invoke("session_input", { id: id(), bytes });
    const end = Date.now() + 10000;
    while (Number(host().dataset.logBytes) < before + bytes.length) {
      if (Date.now() > end)
        throw Error("Writer did not confirm loopback output");
      await new Promise((r) => setTimeout(r, 20));
    }
    if (host().dataset.logActive !== "true")
      throw Error("Rotating writer stopped");
  }
  if (Number(host().dataset.logBytes) - start < 2 * 1024 * 1024)
    throw Error("Fixture did not cross two size boundaries");
  checks.push(
    "More than 2 MiB of real loopback output recorded through bounded queues and size rollovers",
  );
  await reconnect();
  await until(
    () => host().dataset.logActive === "true",
    "Reconnect appends under the same rotation policy",
  );
  await invoke("session_input", {
    id: id(),
    bytes: Array.from(
      new TextEncoder().encode("rotation-reconnect-marker\r\n"),
    ),
  });
  await until(
    () => Number(host().dataset.logBytes) > 25,
    "Reconnected writer confirms output",
  );
  await open();
  if (
    !input("Rotate output logs").checked ||
    input("Log rotation kept files").value !== "2"
  )
    throw Error("Rotation settings did not reopen");
  checks.push(
    "Saved rotation controls reopen; disk retention and byte checks are performed by the PowerShell runner",
  );
}
