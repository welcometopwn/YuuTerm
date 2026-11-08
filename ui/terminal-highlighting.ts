import type { Terminal, IDisposable, IMarker } from "@xterm/xterm";
import type { Highlighting } from "./types";
import {
  compileHighlightWords,
  highlightViewport,
  type HighlightCell,
  type HighlightSpan,
} from "./highlight-policy";

// Decorations are presentation only: buffer text, selection/copy and SSH bytes
// are untouched. Never scan the whole scrollback or decorate full-screen TUIs.
export function terminalHighlighting(
  term: Terminal,
  host: HTMLElement,
  fixture = false,
) {
  let config: Highlighting = {
    enabled: false,
    ipv4: false,
    ipv6: false,
    ipv4_color: "#89b4fa",
    ipv6_color: "#89b4fa",
    rules: [],
  };
  let resources: IDisposable[] = [];
  let words = compileHighlightWords(config);
  let signature = "",
    timer: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;
  const clear = () => {
    resources.forEach((r) => r.dispose());
    resources = [];
  };
  const paint = () => {
    timer = undefined;
    if (disposed) return;
    const buffer = term.buffer.active;
    if (!config.enabled || buffer.type !== "normal" || host.clientWidth === 0) {
      clear();
      signature = "";
      delete host.dataset.highlightSpans;
      delete host.dataset.highlightRects;
      return;
    }
    const rows: { line: number; spans: HighlightSpan[] }[] = [];
    const visible = [];
    for (let y = 0; y < Math.min(term.rows, 100); y++) {
      const index = buffer.viewportY + y,
        line = buffer.getLine(index);
      if (!line) continue;
      const cells: HighlightCell[] = [];
      for (let x = 0; x < Math.min(term.cols, 400); x++) {
        const cell = line.getCell(x);
        cells.push({
          chars: cell?.getChars() ?? "",
          width: cell?.getWidth() ?? 1,
          plain:
            !!cell &&
            cell.isFgDefault() &&
            !cell.isInverse() &&
            !cell.isDim() &&
            !cell.isInvisible(),
        });
      }
      visible.push({ cells, isWrapped: line.isWrapped });
      rows.push({ line: index, spans: [] });
    }
    const spans =
      term.cols > 400
        ? []
        : highlightViewport(
            visible,
            buffer.getLine(buffer.viewportY + visible.length)?.isWrapped ??
              false,
            config,
            words,
          );
    rows.forEach((row, i) => {
      row.spans = spans[i] ?? [];
    });
    // Marker line positions move when scrollback trims. Include base/length so
    // stale decorations cannot survive scroll/erase/reflow with identical text.
    const next = JSON.stringify([buffer.baseY, buffer.length, term.cols, rows]);
    if (
      next === signature &&
      resources.every((r) => !("isDisposed" in r) || !r.isDisposed)
    )
      return;
    clear();
    signature = next;
    let count = 0;
    for (const row of rows) {
      if (!row.spans.length || count >= 512) continue;
      const marker: IMarker | undefined = term.registerMarker(
        row.line - buffer.baseY - buffer.cursorY,
      );
      if (!marker) continue;
      resources.push(marker);
      for (const span of row.spans) {
        if (count >= 512) break;
        const decoration = term.registerDecoration({
          marker,
          x: span.x,
          width: span.width,
          foregroundColor: span.color,
          layer: "bottom",
        });
        if (decoration) {
          resources.push(decoration);
          count++;
        }
      }
    }
    host.dataset.highlightSpans = String(count);
    if (fixture) host.dataset.highlightRects = JSON.stringify(rows);
  };
  const schedule = () => {
    if (!disposed && timer === undefined) timer = setTimeout(paint, 100);
  };
  const listeners = [
    term.onWriteParsed(schedule),
    term.onScroll(schedule),
    term.onResize(() => {
      signature = "";
      schedule();
    }),
    term.buffer.onBufferChange(schedule),
  ];
  const observer = new ResizeObserver(schedule);
  observer.observe(host);
  return {
    update(value: Highlighting) {
      config = value;
      words = compileHighlightWords(value);
      signature = "";
      schedule();
    },
    dispose() {
      disposed = true;
      if (timer !== undefined) clearTimeout(timer);
      listeners.forEach((l) => l.dispose());
      observer.disconnect();
      clear();
      delete host.dataset.highlightSpans;
    },
  };
}
