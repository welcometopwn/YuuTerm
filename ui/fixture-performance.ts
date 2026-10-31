import { invoke } from "@tauri-apps/api/core";

// Instrumented release fixture, not a physical-keyboard latency measurement.
export async function measureWorkspace(ids: number[]) {
  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const until = async (f: () => boolean, label: string) => {
    const end = performance.now() + 10000;
    while (!f()) {
      if (performance.now() > end) throw new Error(label);
      await delay(5);
    }
  };
  const host = (id: number) =>
    document.querySelector<HTMLElement>(
      `[data-live-id="${id}"] .terminal-host`,
    )!;
  const send = (id: number, text: string) =>
    invoke("session_input", {
      id,
      bytes: Array.from(new TextEncoder().encode(text)),
    });
  try {
    await until(
      () =>
        document.querySelectorAll(".xterm-helper-textarea").length ===
        ids.length,
      "Terminal mount deadline",
    );
    const readyMs = performance.now();
    const navigation = performance.getEntriesByType("navigation")[0] as
      PerformanceNavigationTiming | undefined;
    const navigationMs = navigation
      ? {
          response_end: navigation.responseEnd,
          dom_interactive: navigation.domInteractive,
          dom_complete: navigation.domComplete,
        }
      : null;
    await invoke("capture_metrics", {
      report: {
        phase: "idle",
        connections: ids.length,
        frontend_ready_ms: readyMs,
        navigation_ms: navigationMs,
      },
    });
    // The host samples CPU/memory during the first 12 seconds of this hold.
    await delay(30000);
    const echoes: number[] = [];
    for (let n = 1; n <= 20; n++) {
      const id = ids[(n - 1) % Math.min(4, ids.length)];
      const token = `bench-${n}`;
      const start = performance.now();
      await send(id, `${token}\r\n`);
      await until(
        () => host(id).dataset.performanceEcho === token,
        "Echo deadline",
      );
      await new Promise<void>((r) =>
        requestAnimationFrame(() => requestAnimationFrame(() => r())),
      );
      echoes.push(performance.now() - start);
    }
    const start = performance.now();
    await send(
      ids[0],
      Array.from({ length: 1000 }, (_, i) => `flood-line-${i}\r\n`).join("") +
        "bench-done\r\n",
    );
    await until(
      () => host(ids[0]).dataset.performanceEcho === "bench-done",
      "Flood deadline",
    );
    const floodMs = performance.now() - start;
    const history = Number(host(ids[0]).dataset.historyLength);
    const rows = Number(host(ids[0]).dataset.terminalRows);
    if (history > 500 + rows)
      throw new Error("History exceeded configured bound");
    const sorted = [...echoes].sort((a, b) => a - b);
    await invoke("capture_metrics", {
      report: {
        phase: "complete",
        passed: true,
        connections: ids.length,
        frontend_ready_ms: readyMs,
        navigation_ms: navigationMs,
        echo_samples_ms: echoes,
        echo_median_ms: (sorted[9] + sorted[10]) / 2,
        echo_p95_ms: sorted[18],
        flood_lines: 1000,
        flood_observed_ms: floodMs,
        retained_lines: history,
        terminal_rows: rows,
        document_visibility: document.visibilityState,
        qualification:
          "Instrumented release; loopback Windows-agent SSH/SFTP; default highlighting/statistics/history. IPC-to-xterm echo plus two animation frames, not physical input-to-paint or external network latency.",
      },
    });
  } catch (e) {
    await invoke("capture_metrics", {
      report: {
        phase: "complete",
        passed: false,
        error: String(e),
        connections: ids.length,
      },
    });
  }
}
