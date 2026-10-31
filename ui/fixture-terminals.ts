// Capture builds: terminal text straight from the xterm buffer, so checks work
// with the WebGL renderer, which draws to a canvas and has no DOM text rows.
import type { Terminal } from "@xterm/xterm";

export const fixtureTerminals = new WeakMap<HTMLElement, Terminal>();

/** Visible rows of the terminal inside `scope` (or the first one), one per line. */
export function terminalText(scope: ParentNode = document) {
  const host = scope.querySelector<HTMLElement>(".terminal-host");
  const term = host && fixtureTerminals.get(host);
  if (!term) return "";
  const buffer = term.buffer.active;
  const lines: string[] = [];
  for (let y = 0; y < term.rows; y++)
    lines.push(
      buffer.getLine(buffer.viewportY + y)?.translateToString(true) ?? "",
    );
  return lines.join("\n");
}
