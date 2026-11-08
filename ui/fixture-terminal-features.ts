import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap, Preferences } from "./types";

export async function checkTerminalFeatures(
  ids: number[],
  apply: (prefs: Preferences) => void,
  scope: string,
) {
  const checks: string[] = [];
  const until = async (f: () => boolean, label: string) => {
    const end = Date.now() + 20000;
    while (!f()) {
      if (Date.now() > end) throw Error(label);
      await new Promise((r) => setTimeout(r, 30));
    }
    checks.push(label);
  };
  const host = () =>
    document.querySelector<HTMLElement>(
      `[data-live-id="${ids[0]}"] .terminal-host`,
    )!;
  const rects = () =>
    (
      JSON.parse(host().dataset.highlightRects ?? "[]") as {
        line: number;
        spans: { x: number; width: number; color: string }[];
      }[]
    ).flatMap((r) => r.spans);
  const send = (text: string) =>
    invoke("session_input", {
      id: ids[0],
      bytes: Array.from(new TextEncoder().encode(text)),
    });
  try {
    await until(
      () => !!host()?.dataset.terminalCols,
      "Real loopback terminal has mounted",
    );
    const prefs = (await invoke<Bootstrap>("bootstrap")).prefs;
    if (scope === "session-actions") {
      const { checkSessionActions } = await import("./fixture-session-actions");
      await checkSessionActions(until, checks, ids, apply);
      await invoke("capture_report", { checks, passed: true });
      await invoke("capture_ready", { terminalReady: true });
      return;
    }
    if (scope === "bookmark-groups") {
      const { checkBookmarkGroups } = await import("./fixture-bookmark-groups");
      await checkBookmarkGroups(until, checks, ids);
      checks.push(
        "Locked desktop: functional checks only; no focus/performance qualification",
      );
      await invoke("capture_report", { checks, passed: true });
      await invoke("capture_ready", { terminalReady: true });
      return;
    }
    if (scope === "telnet-line") {
      const { checkTelnetLine } = await import("./fixture-telnet-line");
      await checkTelnetLine(until, checks);
      await invoke("capture_report", { checks, passed: true });
      await invoke("capture_ready", { terminalReady: true });
      return;
    }
    if (scope === "serial-import") {
      const { checkSerialImport } = await import("./fixture-serial-import");
      await checkSerialImport(until, checks);
      await invoke("capture_report", { checks, passed: true });
      await invoke("capture_ready", { terminalReady: true });
      return;
    }
    if (scope === "log-rotation") {
      const { checkLogRotation } = await import("./fixture-log-rotation");
      await checkLogRotation(until, checks);
      checks.push(
        "Locked desktop: functional checks only; no focus or performance conclusions",
      );
      await invoke("capture_report", { checks, passed: true });
      await invoke("capture_ready", { terminalReady: true });
      return;
    }
    if (scope === "highlight-import") {
      apply(
        await invoke<Preferences>("save_appearance", {
          mocha: prefs.mocha,
          fontSize: prefs.font_size,
          fontFamily: prefs.font_family,
          statistics: prefs.statistics,
          highlighting: { ...prefs.highlighting, enabled: false },
        }),
      );
      await new Promise((r) => setTimeout(r, 50));
      const { checkHighlightImport } =
        await import("./fixture-highlight-import");
      await checkHighlightImport(until, checks);
      checks.push(
        "Locked desktop: no focus, physical picker/input, performance or installed-MobaXterm round-trip qualification",
      );
      await invoke("capture_report", { checks, passed: true });
      await invoke("capture_ready", { terminalReady: true });
      return;
    }
    if (scope !== "highlight-wrap")
      throw Error("Unexpected terminal feature scope");
    apply(
      await invoke<Preferences>("save_appearance", {
        mocha: prefs.mocha,
        fontSize: prefs.font_size,
        fontFamily: prefs.font_family,
        statistics: prefs.statistics,
        highlighting: {
          ...prefs.highlighting,
          enabled: true,
          ipv4: false,
          ipv6: false,
          rules: [{ keywords: "error", color: "#ff0000", regex: false }],
        },
      }),
    );
    const cols = Number(host().dataset.terminalCols);
    if (cols < 5 || cols > 400)
      throw Error("Fixture geometry outside highlighting bounds");
    const prefix = "\x1b[3J\x1b[2J\x1b[H" + " ".repeat(cols - 2);
    await send(prefix + "ERROR\r\n");
    await until(
      () =>
        rects()
          .filter((r) => r.color === "#ff0000")
          .map((r) => r.width)
          .join(",") === "2,3",
      "A word spanning a real xterm wrap paints two physical rows",
    );
    await send(prefix + "\x1b[31mER\x1b[0mROR\r\n");
    await until(
      () => rects().length === 0,
      "ANSI foreground on either wrapped fragment suppresses the complete token",
    );
    await send("\x1b[3J\x1b[2J\x1b[HER\r\nROR\r\n");
    await until(
      () => rects().length === 0,
      "Hard line breaks never form a logical highlight match",
    );
    await send(prefix + "ERROR\r\n");
    await until(
      () => rects().length === 2,
      "Wrapped decorations return after erase and repaint",
    );
    checks.push(
      "Locked local desktop: no physical focus, input, DPI or performance conclusion",
    );
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
