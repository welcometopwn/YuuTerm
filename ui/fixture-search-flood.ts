// Instrumented native release qualification; synthetic events, real loopback
// TCP output. This module is selected only by the backend's capture scope gate.
import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap, Preferences } from "./types";

export async function checkSearchFlood(
  ids: number[],
  apply: (prefs: Preferences) => void,
) {
  const checks: string[] = [];
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const until = async (f: () => boolean | Promise<boolean>, label: string) => {
    const end = performance.now() + 20000;
    while (!(await f())) {
      if (performance.now() > end) throw Error(label);
      await sleep(20);
    }
    checks.push(label);
  };
  const host = (n = 0) =>
    document.querySelector<HTMLElement>(
      `[data-live-id="${ids[n]}"] .terminal-host`,
    )!;
  const send = (text: string) =>
    invoke("session_input", {
      id: ids[0],
      bytes: Array.from(new TextEncoder().encode(text + "\r")),
    });
  const traces = () =>
    Promise.all(
      ids.map((id) =>
        invoke<number[][]>("capture_trace", { id }).then((v) =>
          JSON.stringify(v),
        ),
      ),
    );
  const input = () =>
    document.querySelector<HTMLInputElement>(".terminal-find input")!;
  const status = () =>
    document.querySelector('.terminal-find [role="status"]')?.textContent;
  const open = async () => {
    document
      .querySelector<HTMLButtonElement>(
        `[data-live-id="${ids[0]}"] [aria-label="Find in terminal"]`,
      )!
      .click();
    await until(() => !!input(), "Local search field opens");
  };
  const key = (name: string, shiftKey = false) =>
    input().dispatchEvent(
      new KeyboardEvent("keydown", {
        key: name,
        shiftKey,
        bubbles: true,
        cancelable: true,
      }),
    );
  const find = async (text: string, expected: boolean, previous = false) => {
    const durations: number[] = [];
    if (input().value !== text) {
      const before = host().dataset.searchSequence;
      Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!.call(input(), text);
      input().dispatchEvent(new Event("input", { bubbles: true }));
      await until(
        () => host().dataset.searchSequence !== before,
        "Debounced local query completed",
      );
      durations.push(Number(host().dataset.searchDurationMs));
    }
    const sequence = host().dataset.searchSequence;
    key("Enter", previous);
    await until(
      () => host().dataset.searchSequence !== sequence,
      "Enter navigation completed locally",
    );
    if (status() !== (expected ? "Match" : "No match"))
      throw Error(`Wrong search status for ${text}`);
    if (expected && host().dataset.searchSelection !== text)
      throw Error("Selected xterm text differs from query");
    durations.push(Number(host().dataset.searchDurationMs));
    return durations.map((ms) => Math.round(ms * 1000) / 1000);
  };
  try {
    await until(
      () =>
        ids.length === 4 &&
        ids.every((_, i) => !!host(i)?.dataset.historyLength),
      "Four real TCP terminals expose bounded fixture metadata",
    );
    await until(
      async () =>
        (await traces()).every((v) =>
          new TextDecoder()
            .decode(new Uint8Array(JSON.parse(v).flat()))
            .includes("ui-stream-check"),
        ),
      "All four startup probes completed before isolation checks",
    );
    const prefs = (await invoke<Bootstrap>("bootstrap")).prefs;
    for (const limit of [500, 10000]) {
      const next = await invoke<Preferences>("save_appearance", {
        mocha: prefs.mocha,
        fontSize: prefs.font_size,
        fontFamily: prefs.font_family,
        highlighting: prefs.highlighting,
        statistics: prefs.statistics,
        scrollbackLines: limit,
      });
      apply(next);
      await until(
        () =>
          ids.every(
            (_, i) => Number(host(i).dataset.scrollbackLimit) === limit,
          ),
        `${limit}: history limit applied to all four terminals`,
      );
      await send("YT_SEARCH_PREP");
      await until(
        () =>
          host().dataset.bufferTail?.includes("trim-me-before-flood") === true,
        `${limit}: real server baseline rendered`,
      );
      if (!input()) await open();
      await find("trim-me-before-flood", true);
      await find("flood-target", false);
      await send(`YT_SEARCH_FLOOD_${limit}`);
      const baseline = await traces();
      const start = performance.now();
      await until(
        () => host().dataset.bufferTail?.includes("flood-target") === true,
        `${limit}: paced remote output is arriving`,
      );
      const durations: number[] = [];
      let peakRetained = 0;
      for (let sample = 0; sample < 8; sample++) {
        durations.push(
          ...(await find(
            sample % 2 ? "missing-search-token" : "flood-target",
            sample % 2 === 0,
            sample % 3 === 0,
          )),
        );
        peakRetained = Math.max(
          peakRetained,
          Number(host().dataset.historyLength),
        );
        if (peakRetained > limit + Number(host().dataset.terminalRows))
          throw Error("Retained history exceeded its bound during flood");
      }
      if (host().dataset.bufferTail?.includes(`flood-done-${limit}`))
        throw Error("Search samples did not overlap active remote output");
      checks.push(
        `${limit}: eight hit/miss forward/backward searches overlap active remote output; retained rows remain bounded`,
      );
      await until(
        () =>
          host().dataset.bufferTail?.includes(`flood-done-${limit}`) === true,
        `${limit}: all 20000 paced server lines completed`,
      );
      const elapsed = performance.now() - start;
      const finalRetained = Number(host().dataset.historyLength);
      const rows = Number(host().dataset.terminalRows);
      if (finalRetained !== limit + rows)
        throw Error(
          "Flood did not fill and respect the configured history bound",
        );
      const fullHistory = [
        ...(await find("trim-me-before-flood", false)),
        ...(await find("flood-target", true)),
      ];
      if (JSON.stringify(await traces()) !== JSON.stringify(baseline))
        throw Error("Search leaked input to a terminal transport");
      checks.push(
        `${limit}: query/navigation never sends terminal input; trimmed baseline is no longer searchable`,
      );
      if (
        ids
          .slice(1)
          .some((_, i) =>
            host(i + 1).dataset.bufferTail?.includes("flood-target"),
          )
      )
        throw Error("Output leaked between panes");
      checks.push(`${limit}: other panes remain isolated`);
      checks.push(
        JSON.stringify({
          limit,
          search_sync_ms: durations,
          full_history_sync_ms: fullHistory,
          paced_output_ms: Math.round(elapsed),
          peak_rows_during_queries: peakRetained,
          final_retained_rows: finalRetained,
          terminal_rows: rows,
        }),
      );
      key("Escape");
      await until(() => !input(), `${limit}: Escape closes search after flood`);
      if (JSON.stringify(await traces()) !== JSON.stringify(baseline))
        throw Error("Escape leaked terminal input");
    }
    await open();
    await find("flood-target", true);
    await invoke("capture_report", { checks, passed: true });
    await invoke("capture_ready", { terminalReady: true });
  } catch (error) {
    await invoke("capture_report", {
      checks: [...checks, String(error)],
      passed: false,
    });
    throw error;
  }
}
