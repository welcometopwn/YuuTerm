import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap } from "./types";

export async function checkHistory(
  ids: number[],
  until: (f: () => boolean | Promise<boolean>, name: string) => Promise<void>,
  checks: string[],
) {
  const hosts = () =>
    Array.from(document.querySelectorAll<HTMLElement>(".terminal-host"));
  const host = () =>
    document.querySelector<HTMLElement>(
      `[data-live-id="${ids[0]}"] .terminal-host`,
    )!;
  const field = () =>
    document.querySelector<HTMLInputElement>(
      '[aria-label="Terminal history lines"]',
    )!;
  const button = (text: string) =>
    Array.from(
      document.querySelectorAll<HTMLButtonElement>("dialog button"),
    ).find((b) => b.textContent?.trim() === text)!;
  const open = async () => {
    document
      .querySelector<HTMLButtonElement>('[aria-label="Settings"]')!
      .click();
    await until(() => !!field(), "History control opens in Settings");
  };
  const change = (n: number) => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(field(), String(n));
    field().dispatchEvent(new Event("input", { bubbles: true }));
  };
  const limit = (n: number) =>
    hosts().length === 4 &&
    hosts().every((h) => Number(h.dataset.scrollbackLimit) === n);
  await until(
    () => limit(500),
    "Older profiles retain a 500-line history on all four real SSH terminals",
  );
  const before = (await invoke<Bootstrap>("bootstrap")).prefs;
  await open();
  change(10001);
  await until(
    () => button("Save settings").disabled,
    "Oversized history disables Save",
  );
  change(1000);
  document
    .querySelector<HTMLButtonElement>('dialog [aria-label="Close dialog"]')!
    .click();
  await until(
    () => !field() && limit(500),
    "Closing the history draft leaves live limits unchanged",
  );
  if ((await invoke<Bootstrap>("bootstrap")).prefs.scrollback_lines !== 500)
    throw new Error("History draft was persisted");
  checks.push("Closing Settings preserves persisted history");
  const save = async (n: number) => {
    await open();
    change(n);
    await new Promise((resolve) => setTimeout(resolve, 40));
    button("Save settings").click();
    await until(
      () => !document.querySelector("dialog[open]") && limit(n),
      `Save applies ${n} history lines to all existing terminal instances`,
    );
  };
  await save(1000);
  await invoke("session_input", {
    id: ids[0],
    bytes: Array.from(new TextEncoder().encode("history-line\r\n".repeat(800))),
  });
  await until(
    () => Number(host().dataset.historyLength) > 700,
    "Actual SSH output retains more than the old 500-line limit",
  );
  await open();
  change(0);
  await new Promise((resolve) => setTimeout(resolve, 60));
  button("Cancel").click();
  await until(
    () => !field() && limit(1000) && Number(host().dataset.historyLength) > 700,
    "Cancelling a reduction preserves the existing terminal history",
  );
  await save(500);
  await until(
    () =>
      Number(host().dataset.historyLength) <=
      500 + Number(host().dataset.terminalRows),
    "Saving a smaller limit trims actual xterm history to its configured bound",
  );
  await save(0);
  await until(
    () =>
      Number(host().dataset.historyLength) <=
      Number(host().dataset.terminalRows),
    "Zero retains the visible screen without scrollback",
  );
  const payload = {
    mocha: before.mocha,
    fontSize: before.font_size,
    fontFamily: before.font_family,
    highlighting: before.highlighting,
    statistics: before.statistics,
    scrollbackLines: 10001,
  };
  let rejected = false;
  try {
    await invoke("save_appearance", payload);
  } catch {
    rejected = true;
  }
  if (
    !rejected ||
    (await invoke<Bootstrap>("bootstrap")).prefs.scrollback_lines !== 0
  )
    throw new Error("Invalid IPC history was accepted");
  checks.push(
    "Backend refuses an oversized IPC value and preserves saved settings",
  );
  if (
    document.querySelectorAll("[data-live-id]").length !== 4 ||
    ids.some((id) => !document.querySelector(`[data-live-id="${id}"]`))
  )
    throw new Error("History setting replaced a connection");
  checks.push("All four original live SSH incarnation IDs remain unchanged");
  await save(500);
  await open();
}
