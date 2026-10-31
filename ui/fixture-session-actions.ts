import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap, Preferences } from "./types";
export async function checkSessionActions(
  until: (f: () => boolean, label: string) => Promise<void>,
  checks: string[],
  ids: number[],
  apply: (p: Preferences) => void,
) {
  const saved = async () => (await invoke<Bootstrap>("bootstrap")).prefs;
  await until(
    () =>
      [...document.querySelectorAll<HTMLElement>(".terminal-host")].every(
        (host) => {
          const screen = host.querySelector<HTMLElement>(".xterm-screen");
          return (
            !!screen &&
            host.getBoundingClientRect().bottom -
              screen.getBoundingClientRect().bottom >=
              8
          );
        },
      ),
    "Terminal rows fit above the bottom inset in every visible pane",
  );
  const button = (name: string) =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent?.trim() === name,
    )!;
  const row = (id: number) =>
    document.querySelector<HTMLElement>(
      `.session-row[data-profile-id="${id}"]`,
    )!;
  const input = (el: HTMLInputElement, value: string) => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  };
  const menu = async (id: number) => {
    row(id).dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        clientX: 130,
        clientY: 300,
      }),
    );
    await new Promise(requestAnimationFrame);
    await until(() => !!button("Wake on LAN"), "Session context menu opens");
  };
  const click = (id: number, ctrl = false) =>
    row(id)
      .querySelector(".session-select")!
      .dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: ctrl }));
  button("Sessions").click();
  const original = await saved();
  const ungrouped = () =>
    document.querySelector('[aria-label="Move sessions to Ungrouped"]');
  const selectionCount = () =>
    document.querySelector(".session-selection-count");
  await until(
    () => !!ungrouped(),
    "Ungrouped is visible for ungrouped bookmarks",
  );
  click(original.profiles[0].id);
  await until(
    () =>
      row(original.profiles[0].id).getAttribute("aria-selected") === "true" &&
      !selectionCount(),
    "A single selected session has no selection counter",
  );
  click(original.profiles[1].id, true);
  await until(
    () =>
      selectionCount()?.textContent?.trim() === "2 selected" &&
      !ungrouped()?.contains(selectionCount()),
    "Multiple selection has its own counter outside the Ungrouped heading",
  );
  const fixtureProfiles = original.profiles.map((p) => p.id);
  apply(
    await invoke<Preferences>("move_profiles", {
      profileIds: fixtureProfiles,
      group: "Sidebar fixture",
      beforeId: null,
    }),
  );
  await until(
    () =>
      !ungrouped() &&
      row(original.profiles[0].id).classList.contains("grouped"),
    "Ungrouped disappears when every bookmark belongs to a group",
  );
  const dragRoot = document.querySelector<HTMLElement>(".app")!;
  const originalCapture = dragRoot.setPointerCapture;
  const originalHasCapture = dragRoot.hasPointerCapture;
  const dragRect = row(original.profiles[0].id).getBoundingClientRect();
  try {
    dragRoot.setPointerCapture = () => {};
    dragRoot.hasPointerCapture = () => false;
    row(original.profiles[0].id).dispatchEvent(
      new PointerEvent("pointerdown", {
        bubbles: true,
        button: 0,
        pointerId: 71,
        clientX: dragRect.x + 50,
        clientY: dragRect.y + 20,
      }),
    );
    dragRoot.dispatchEvent(
      new PointerEvent("pointermove", {
        bubbles: true,
        pointerId: 71,
        clientX: dragRect.x + 65,
        clientY: dragRect.y + 20,
      }),
    );
    await until(
      () => !!ungrouped(),
      "Dragging exposes Ungrouped even when the group is empty",
    );
    dragRoot.dispatchEvent(
      new PointerEvent("pointercancel", { bubbles: true, pointerId: 71 }),
    );
    await until(
      () => !ungrouped(),
      "Cancelling the drag hides the empty Ungrouped target",
    );
  } finally {
    dragRoot.setPointerCapture = originalCapture;
    dragRoot.hasPointerCapture = originalHasCapture;
  }
  apply(
    await invoke<Preferences>("move_profiles", {
      profileIds: fixtureProfiles,
      group: "",
      beforeId: null,
    }),
  );
  await until(
    () => !!ungrouped(),
    "Moving bookmarks back restores the Ungrouped heading",
  );
  document
    .querySelector('[aria-label="Saved sessions"]')!
    .dispatchEvent(
      new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }),
    );
  await until(
    () => !selectionCount(),
    "Escape clears the multiple-selection counter",
  );
  document
    .querySelector<HTMLButtonElement>('[aria-label="Edit TELNET fixture 1"]')!
    .click();
  await until(
    () =>
      !!document.querySelector('[aria-label="MAC address for Wake-on-LAN"]'),
    "Session settings expose the optional WoL MAC field",
  );
  if (
    document.querySelector("dialog")?.textContent?.includes("still being built")
  )
    throw Error("Session editor retains the obsolete preview notice");
  checks.push(
    "Session editor omits the obsolete protocol/password preview notice",
  );
  input(
    document.querySelector('[aria-label="MAC address for Wake-on-LAN"]')!,
    "invalid",
  );
  await until(
    () => button("Save session").disabled,
    "Malformed MAC disables Save",
  );
  button("Cancel").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Cancel discards the MAC draft",
  );
  if (JSON.stringify(original) !== JSON.stringify(await saved()))
    throw Error("Cancel saved the draft");
  document
    .querySelector<HTMLButtonElement>('[aria-label="Edit TELNET fixture 1"]')!
    .click();
  await until(
    () =>
      !!document.querySelector('[aria-label="MAC address for Wake-on-LAN"]'),
    "MAC editor reopens",
  );
  input(
    document.querySelector('[aria-label="MAC address for Wake-on-LAN"]')!,
    "02:11:22:33:44:55",
  );
  await until(() => !button("Save session").disabled, "Valid MAC enables Save");
  button("Save session").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Explicit Save stores the MAC",
  );
  if ((await saved()).profiles[0].mac_address !== "02:11:22:33:44:55")
    throw Error("MAC save lost metadata");
  const base = original.profiles[0];
  let prefs = await invoke<Preferences>("save_profile", {
    profile: {
      ...base,
      id: 0,
      name: "Bulk fixture A",
      mac_address: "02:11:22:33:44:55",
    },
  });
  const a = prefs.profiles.at(-1)!;
  prefs = await invoke<Preferences>("save_profile", {
    profile: {
      ...base,
      id: 0,
      name: "Bulk fixture B",
      mac_address: "02:11:22:33:44:55",
    },
  });
  const b = prefs.profiles.at(-1)!;
  apply(prefs);
  await until(() => !!row(b.id), "Two isolated saved sessions appear");
  const tabCount = document.querySelectorAll(".tab").length;
  click(base.id);
  await until(
    () => row(base.id).getAttribute("aria-selected") === "true",
    "Plain click establishes the range anchor",
  );
  row(a.id)
    .querySelector(".session-select")!
    .dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
  await until(
    () =>
      document.querySelectorAll('.session-row[aria-selected="true"]').length ===
      5,
    "Shift-click selects the inclusive visible range",
  );
  row(original.profiles[1].id)
    .querySelector(".session-select")!
    .dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
  await until(
    () =>
      document.querySelectorAll('.session-row[aria-selected="true"]').length ===
      2,
    "Repeated Shift-click retains its anchor and shrinks the range",
  );
  row(b.id)
    .querySelector(".session-select")!
    .dispatchEvent(
      new MouseEvent("click", { bubbles: true, shiftKey: true, ctrlKey: true }),
    );
  await until(
    () =>
      document.querySelectorAll('.session-row[aria-selected="true"]').length ===
      6,
    "Ctrl+Shift-click extends the selected range",
  );
  if (document.querySelectorAll(".tab").length !== tabCount)
    throw Error("Range selection opened a session tab");
  checks.push("Modified range clicks do not open or connect sessions");
  document
    .querySelector(".session-list")!
    .dispatchEvent(
      new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }),
    );
  await until(
    () =>
      document.querySelectorAll('.session-row[aria-selected="true"]').length ===
      0,
    "Escape clears the range and anchor",
  );
  let refused = false;
  try {
    await invoke("delete_profiles", { profileIds: [base.id, a.id] });
  } catch {
    refused = true;
  }
  if (!refused || (await saved()).profiles.length !== prefs.profiles.length)
    throw Error("Active-session batch deletion was not atomic");
  checks.push(
    "Backend refuses a batch containing an active connection without removing any bookmark",
  );
  click(a.id, true);
  click(b.id, true);
  await until(
    () =>
      row(a.id).getAttribute("aria-selected") === "true" &&
      row(b.id).getAttribute("aria-selected") === "true",
    "Ctrl-click marks multiple sessions",
  );
  click(a.id, true);
  await until(
    () => row(a.id).getAttribute("aria-selected") === "false",
    "Ctrl-click toggles only its own selection",
  );
  click(a.id, true);
  await until(
    () => row(a.id).getAttribute("aria-selected") === "true",
    "The selected set is committed before opening its context menu",
  );
  await menu(a.id);
  await until(
    () => !!button("Connect selected sessions"),
    "Right-click preserves the selected set",
  );
  for (const name of [
    "Wake on LAN",
    "Move to group",
    "Copy hosts",
    "Delete sessions",
  ])
    if (!button(name)) throw Error(`Missing bulk action ${name}`);
  const clipboard = navigator.clipboard.writeText;
  let copied = "";
  try {
    navigator.clipboard.writeText = async (value) => {
      copied = value;
    };
    button("Copy hosts").click();
    await until(
      () => copied === base.host,
      "Copy hosts passes the unique selected hosts to the clipboard writer",
    );
  } finally {
    navigator.clipboard.writeText = clipboard;
  }
  await menu(a.id);
  button("Wake on LAN").click();
  await until(
    () =>
      document.body.textContent?.includes(
        "Wake packets sent to 1 computer(s)",
      ) === true,
    "Explicit bulk wake sends one packet for a shared MAC",
  );
  await menu(a.id);
  button("Connect selected sessions").click();
  await until(
    () => document.querySelectorAll(".tab").length >= 6,
    "Bulk Connect opens both selected tabs",
  );
  for (const profile of [a, b]) {
    [...document.querySelectorAll<HTMLButtonElement>(".tab")]
      .find((tab) => tab.textContent?.includes(profile.name))!
      .click();
    await until(
      () =>
        document
          .querySelector(`.pane[data-profile-id="${profile.id}"]`)
          ?.textContent?.includes("Connected") === true,
      `Bulk Connect establishes ${profile.name} over real loopback TCP`,
    );
  }
  await menu(a.id);
  button("Move to group").click();
  await until(
    () => !!document.querySelector('[aria-label="Destination group"]'),
    "Bulk Move opens its group dialog",
  );
  input(
    document.querySelector('[aria-label="Destination group"]')!,
    "Bulk group",
  );
  await new Promise(requestAnimationFrame);
  button("Move").click();
  await until(
    () =>
      !document.querySelector("dialog[open]") &&
      row(a.id)?.classList.contains("grouped") === true,
    "Both selected sessions move together",
  );
  const moved = await saved();
  if (
    moved.profiles
      .filter((p) => [a.id, b.id].includes(p.id))
      .some((p) => p.group !== "Bulk group")
  )
    throw Error("Partial group move");
  const source = row(a.id).getBoundingClientRect();
  const target = document
    .querySelector('[aria-label="Move sessions to Ungrouped"]')!
    .getBoundingClientRect();
  row(a.id).dispatchEvent(
    new PointerEvent("pointerdown", {
      bubbles: true,
      button: 0,
      pointerId: 72,
      clientX: source.x + 50,
      clientY: source.y + 20,
    }),
  );
  // Synthetic pointers cannot own OS pointer capture; isolate that native call.
  const root = document.querySelector<HTMLElement>(".app")!;
  const capture = root.setPointerCapture;
  const release = root.releasePointerCapture;
  const has = root.hasPointerCapture;
  try {
    root.setPointerCapture = () => {};
    root.releasePointerCapture = () => {};
    root.hasPointerCapture = () => false;
    root.dispatchEvent(
      new PointerEvent("pointermove", {
        bubbles: true,
        pointerId: 72,
        clientX: target.x + 30,
        clientY: target.y + 8,
      }),
    );
    root.dispatchEvent(
      new PointerEvent("pointerup", {
        bubbles: true,
        pointerId: 72,
        clientX: target.x + 30,
        clientY: target.y + 8,
      }),
    );
  } finally {
    root.setPointerCapture = capture;
    root.releasePointerCapture = release;
    root.hasPointerCapture = has;
  }
  await until(
    () =>
      !row(a.id)?.classList.contains("grouped") &&
      !row(b.id)?.classList.contains("grouped"),
    "Pointer drag moves the entire selection back to Ungrouped",
  );
  if (
    (await saved()).profiles
      .filter((p) => [a.id, b.id].includes(p.id))
      .some((p) => p.group)
  )
    throw Error("Partial pointer drop");
  const reorderFrom = row(a.id).getBoundingClientRect();
  const reorderTo = row(original.profiles[0].id).getBoundingClientRect();
  row(a.id).dispatchEvent(
    new PointerEvent("pointerdown", {
      bubbles: true,
      button: 0,
      pointerId: 73,
      clientX: reorderFrom.x + 50,
      clientY: reorderFrom.y + 20,
    }),
  );
  try {
    root.setPointerCapture = () => {};
    root.releasePointerCapture = () => {};
    root.hasPointerCapture = () => false;
    root.dispatchEvent(
      new PointerEvent("pointermove", {
        bubbles: true,
        pointerId: 73,
        clientX: reorderTo.x + 40,
        clientY: reorderTo.y + 10,
      }),
    );
    root.dispatchEvent(
      new PointerEvent("pointerup", {
        bubbles: true,
        pointerId: 73,
        clientX: reorderTo.x + 40,
        clientY: reorderTo.y + 10,
      }),
    );
  } finally {
    root.setPointerCapture = capture;
    root.releasePointerCapture = release;
    root.hasPointerCapture = has;
  }
  await until(
    () =>
      document
        .querySelector(".session-row")
        ?.getAttribute("data-profile-id") === String(a.id),
    "Pointer drop before a row reorders the selected block",
  );
  for (const id of ids) {
    await invoke("session_input", { id, bytes: [] });
  }
  checks.push(
    "All four original loopback connections survive selection and moving",
  );
  await menu(a.id);
  button("Delete sessions").click();
  await until(
    () => !!document.querySelector('dialog[aria-label="Delete sessions"]'),
    "Bulk deletion requires confirmation",
  );
  button("Cancel").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Cancel preserves selected sessions",
  );
  await menu(a.id);
  button("Delete sessions").click();
  await until(
    () => !!document.querySelector('dialog[aria-label="Delete sessions"]'),
    "Confirmed delete dialog opens again",
  );
  button("Delete").click();
  await until(
    () => !row(a.id) && !row(b.id),
    "Confirmed deletion removes both selected sessions",
  );
  const remaining = await saved();
  if (remaining.profiles.length !== original.profiles.length)
    throw Error("Deletion changed unrelated sessions");
  for (const id of ids) {
    await invoke("session_input", { id, bytes: [] });
  }
  checks.push(
    "Bulk deletion preserves all unrelated bookmarks and live sessions",
  );
  await menu(original.profiles[1].id);
  await until(
    () => !!button("Wake on LAN") && button("Wake on LAN").disabled,
    "Single-session Wake on LAN is disabled without a MAC",
  );
  document
    .querySelector('[role="menu"]')!
    .dispatchEvent(
      new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }),
    );
  await menu(original.profiles[1].id);
  button("Duplicate session").click();
  await until(
    () => document.querySelectorAll(".session-row").length === 5,
    "Duplicate session inserts one new saved row",
  );
  const duplicated = await saved();
  const sourcePosition = duplicated.profiles.findIndex(
    (p) => p.id === original.profiles[1].id,
  );
  const copy = duplicated.profiles[sourcePosition + 1];
  if (
    !copy.name.includes("(copy)") ||
    row(original.profiles[1].id).nextElementSibling !== row(copy.id)
  )
    throw Error("Copy is not directly under its source");
  checks.push(
    "Duplicate session appears directly below its original in saved and visible order",
  );
  click(original.profiles[0].id);
  await until(
    () => row(original.profiles[0].id).getAttribute("aria-selected") === "true",
    "Selection remains available after duplication",
  );
  row(original.profiles[3].id)
    .querySelector(".session-select")!
    .dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
  await until(
    () =>
      document.querySelectorAll('.session-row[aria-selected="true"]').length ===
      5,
    "Shift range includes the copy at its new position",
  );
  await menu(original.profiles[0].id);
  document
    .querySelector('[role="menu"]')!
    .dispatchEvent(
      new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }),
    );
  const tabs = () => [...document.querySelectorAll<HTMLElement>(".tabs .tab")];
  const active = () => document.querySelector<HTMLElement>(".tabs .tab.active");
  const previousActive = active();
  const background = tabs().find((tab) => tab !== previousActive)!;
  const auxiliary = (target: Element, button: number, type = "auxclick") => {
    const event = new MouseEvent(type, {
      bubbles: true,
      cancelable: true,
      button,
    });
    target.dispatchEvent(event);
    return event.defaultPrevented;
  };
  const middleClickTabCount = tabs().length;
  auxiliary(background, 2);
  if (tabs().length !== middleClickTabCount)
    throw Error("Right auxiliary click closed a tab");
  if (!auxiliary(background, 1, "mousedown"))
    throw Error("Middle mousedown did not prevent autoscroll");
  if (!auxiliary(background.querySelector("button")!, 1))
    throw Error("Middle click did not prevent the default action");
  await until(
    () =>
      tabs().length === middleClickTabCount - 1 &&
      !background.isConnected &&
      active() === previousActive,
    "Middle click on a background tab label closes only that tab and preserves focus",
  );
  auxiliary(active()!, 1);
  await until(
    () =>
      tabs().length === middleClickTabCount - 2 &&
      !!active() &&
      !previousActive!.isConnected,
    "Middle click on the active tab chooses a remaining tab",
  );
  if ((await saved()).profiles.length !== duplicated.profiles.length)
    throw Error("Middle click deleted a saved bookmark");
  checks.push("Middle-click closing preserves saved bookmarks");
}
