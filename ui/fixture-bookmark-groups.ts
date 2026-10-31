import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap } from "./types";

export async function checkBookmarkGroups(
  until: (f: () => boolean, label: string) => Promise<void>,
  checks: string[],
  ids: number[],
) {
  const button = (name: string) =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent?.trim() === name,
    )!;
  const saved = async () => (await invoke<Bootstrap>("bootstrap")).prefs;
  const before = await saved();
  const liveIds = () =>
    [...document.querySelectorAll<HTMLElement>(".pane[data-live-id]")]
      .map((p) => p.dataset.liveId)
      .join(",");
  const originalIds = liveIds();
  const openImport = async () => {
    document
      .querySelector<HTMLButtonElement>(
        '[aria-label="Session import and export"]',
      )!
      .click();
    await until(() => !!button("Import sessions"), "Import menu opens");
    button("Import sessions").click();
    await until(
      () =>
        document.querySelectorAll(".session-import-list details").length === 6,
      "Review displays six supported synthetic bookmarks",
    );
    const review = document.querySelector(".session-import-list")!.textContent!;
    if (
      !review.includes("Group: Team / Production") ||
      !review.includes("Group: Team / Lab") ||
      !review.includes("Ungrouped")
    )
      throw Error("Missing section-local groups in preview");
    if (
      !review.includes("64 UTF-8") ||
      !review.includes("Duplicate bookmark folder")
    )
      throw Error("Invalid folder rejections missing from preview");
    checks.push(
      "Preview displays nested and late SubRep groups, root ungrouped, and both folder rejections",
    );
  };
  await openImport();
  button("Cancel").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Cancel discards folder import preview",
  );
  if (JSON.stringify(await saved()) !== JSON.stringify(before))
    throw Error("Cancel changed saved metadata");
  await openImport();
  [...document.querySelectorAll<HTMLButtonElement>("dialog button")]
    .find((b) => b.textContent?.trim() === "Import sessions")!
    .click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Explicit import adds grouped bookmarks",
  );
  const after = await saved();
  if (
    after.profiles.length !== before.profiles.length + 6 ||
    after.profiles
      .slice(4)
      .some((p) => p.id <= 4 || p.credential_id || p.logging.enabled)
  )
    throw Error("Unsafe import merge");
  if (liveIds() !== originalIds)
    throw Error("Import changed live sessions or auto-connected");
  checks.push(
    "Import uses fresh IDs, no credential/logging state or automatic connections; live incarnations unchanged",
  );

  const openMove = async () => {
    document.querySelector<HTMLElement>(".tab")!.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        clientX: 340,
        clientY: 64,
      }),
    );
    await until(
      () => !!button("Move to group"),
      "Session context menu offers Move to group",
    );
    button("Move to group").click();
    await until(
      () => !!document.querySelector('[aria-label="Destination group"]'),
      "Move dialog opens",
    );
  };
  const set = (value: string) => {
    const input = document.querySelector<HTMLInputElement>(
      '[aria-label="Destination group"]',
    )!;
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  };
  await openMove();
  set("a".repeat(65));
  await until(
    () =>
      button("Move").disabled &&
      !!document.querySelector('dialog [role="alert"]'),
    "Oversized group is rejected before saving",
  );
  set("Team / Lab");
  button("Cancel").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Cancel leaves group unchanged",
  );
  if ((await saved()).profiles[0].group) throw Error("Canceled move persisted");
  for (const target of ["Team / Lab", "", "  Moved fixture  "]) {
    await openMove();
    set(target);
    await until(
      () =>
        !button("Move").disabled &&
        document
          .querySelector<HTMLInputElement>('[aria-label="Destination group"]')
          ?.getAttribute("value") === target,
      "Valid destination permits explicit move",
    );
    button("Move").click();
    await until(
      () => !document.querySelector("dialog[open]"),
      "Explicit move closes dialog",
    );
    if (((await saved()).profiles[0].group ?? "") !== target.trim())
      throw Error("Move/ungroup failed to persist");
    if (liveIds() !== originalIds)
      throw Error("Moving restarted a live connection");
  }
  await invoke("session_input", {
    id: ids[0],
    bytes: Array.from(new TextEncoder().encode("group-move-live-marker\r\n")),
  });
  await until(
    () =>
      document
        .querySelector<HTMLElement>(`[data-live-id="${ids[0]}"] .terminal-host`)
        ?.dataset.bufferTail?.includes("group-move-live-marker") ?? false,
    "The same real TCP session still echoes after move, ungroup and new-group creation",
  );
  await until(
    () =>
      [...document.querySelectorAll(".session-group")].some((g) =>
        g.textContent?.includes("Moved fixture"),
      ),
    "Saved group is visible in sidebar",
  );
  await openImport(); // Retain the real grouped preview in the native still.
}
