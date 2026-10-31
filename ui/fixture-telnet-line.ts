import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap } from "./types";
export async function checkTelnetLine(
  until: (f: () => boolean, label: string) => Promise<void>,
  checks: string[],
) {
  const profile = (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0];
  const pane = () =>
    document.querySelector<HTMLElement>(
      `.pane[data-profile-id="${profile.id}"]`,
    )!;
  const id = () => Number(pane().dataset.liveId);
  const rows = () => pane().querySelector(".xterm-rows")?.textContent ?? "";
  const line = () =>
    pane().querySelector<HTMLInputElement>('[aria-label="TELNET local line"]')!;
  const checkbox = () =>
    document.querySelector<HTMLInputElement>(
      '[aria-label="TELNET local line editing"]',
    )!;
  const button = (text: string) =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent?.trim() === text,
    )!;
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
    await until(() => !!checkbox(), "TELNET line mode is in Terminal settings");
  };
  const set = (value: string) => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(line(), value);
    line().dispatchEvent(new Event("input", { bubbles: true }));
  };
  const barrier = async (marker: string) => {
    await new Promise((r) => setTimeout(r, 40));
    await invoke("session_input", {
      id: id(),
      bytes: Array.from(new TextEncoder().encode(marker + "\r\n")),
    });
    await until(
      () => rows().includes(marker),
      "Real TCP peer returns " + marker,
    );
  };
  if (profile.telnet.local_line_editing || line())
    throw Error("Local editing should default off");
  await open();
  checkbox().click();
  button("Cancel").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Cancel discards line-mode draft",
  );
  if (
    (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0].telnet
      .local_line_editing
  )
    throw Error("Cancelled setting saved");
  await open();
  checkbox().click();
  await new Promise((r) => setTimeout(r, 30));
  button("Save session").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Explicit Save commits the preference",
  );
  if (line()) throw Error("Setting changed the running incarnation");
  const previous = id();
  await invoke("session_disconnect", { id: previous });
  await until(
    () => !!pane().querySelector("button.connect"),
    "Disconnect exposes reconnect",
  );
  pane().querySelector<HTMLButtonElement>("button.connect")!.click();
  await until(
    () => id() !== previous && !!line() && !line().disabled,
    "Fresh incarnation has its own local line editor",
  );
  set("UNSENT_界");
  await barrier("WIRE_BARRIER_ONE");
  if (rows().includes("UNSENT_界"))
    throw Error("Typing leaked a partial line to the peer");
  line().dispatchEvent(
    new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
  );
  await until(() => line().value === "", "Escape discards unsent text");
  set("draft");
  set("界".repeat(1366));
  await until(
    () => !!pane().querySelector('[role="alert"]'),
    "Oversized Unicode draft is rejected",
  );
  if (line().value !== "draft")
    throw Error("Oversize silently replaced/truncated draft");
  set("FINAL_Grüße_界");
  line().dispatchEvent(
    new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
  );
  await until(
    () => rows().includes("FINAL_Grüße_界") && line().value === "",
    "Enter sends only the edited Unicode line and clears acknowledged draft",
  );
  set("UNSENT_NEW");
  await barrier("WIRE_BARRIER_TWO");
  if (rows().includes("UNSENT_NEW")) throw Error("New draft leaked");
  if (
    document.querySelectorAll('[aria-label="TELNET local line"]').length !== 1
  )
    throw Error("Mode leaked to another session");
  checks.push(
    "Other live terminals remain in character mode; no automatic send on typing or reconnect",
  );
  const old = id();
  await invoke("session_disconnect", { id: old });
  await until(
    () => !!pane().querySelector("button.connect"),
    "Disconnect ends the edited incarnation",
  );
  pane().querySelector<HTMLButtonElement>("button.connect")!.click();
  await until(
    () => id() !== old && !!line() && !line().disabled,
    "Another incarnation starts with a fresh draft",
  );
  if (line().value !== "") throw Error("Unsent draft survived reconnect");
  await barrier("WIRE_BARRIER_THREE");
  if (rows().includes("UNSENT_NEW"))
    throw Error("Reconnect sent the previous draft");
}
