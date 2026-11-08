import type { Highlighting } from "./types";
import { RE2JS } from "re2js";

export interface HighlightCell {
  chars: string;
  width: number;
  plain: boolean;
}
export interface HighlightSpan {
  x: number;
  width: number;
  color: string;
}

type CompiledRule = { color: string; words?: Set<string>; pattern?: RE2JS };
const regexCache = new Map<string, RE2JS | string>();
export function compileRegex(pattern: string): RE2JS {
  const cached = regexCache.get(pattern);
  if (typeof cached === "string") throw new Error(cached);
  if (cached) return cached;
  try {
    if (
      !pattern ||
      new TextEncoder().encode(pattern).length > 128 ||
      /[\x00-\x1f\x7f]/.test(pattern)
    )
      throw new Error(
        "Use a pattern of 1–128 bytes without control characters.",
      );
    let expansion = 1;
    for (const match of pattern.matchAll(/\{(\d+)(?:,(\d*))?\}/g)) {
      const count = Number(match[2] || match[1]);
      expansion *= Math.max(1, count);
      if (count > 100 || expansion > 1024)
        throw new Error("Counted repetitions are too large.");
    }
    const compiled = RE2JS.compile(pattern, RE2JS.CASE_INSENSITIVE);
    if (compiled.programSize() > 512)
      throw new Error("Pattern is too complex.");
    if (regexCache.size >= 64)
      regexCache.delete(regexCache.keys().next().value!);
    regexCache.set(pattern, compiled);
    return compiled;
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    if (regexCache.size >= 64)
      regexCache.delete(regexCache.keys().next().value!);
    regexCache.set(pattern, message);
    throw new Error(message);
  }
}

// First rule wins; validated IPs take precedence. Never use backtracking JS
// RegExp for user patterns. Work stays within one bounded visible logical line.
export function compileHighlightWords(config: Highlighting): CompiledRule[] {
  const compiled: CompiledRule[] = [];
  for (const rule of config.rules.slice(0, 12)) {
    if (!/^#[0-9a-f]{6}$/i.test(rule.color)) continue;
    if (rule.regex) {
      try {
        compiled.push({
          color: rule.color,
          pattern: compileRegex(rule.keywords),
        });
      } catch {
        /* invalid drafts do not paint */
      }
      continue;
    }
    const words = new Set<string>();
    for (const word of rule.keywords
      .slice(0, 1100)
      .split(/[,\t\n\r\f\v ]+/)
      .slice(0, 32))
      if (/^[a-z0-9_-]{1,32}$/i.test(word)) words.add(word.toLowerCase());
    compiled.push({ color: rule.color, words });
  }
  return compiled;
}
export function highlightCells(
  cells: HighlightCell[],
  config: Highlighting,
  rules: CompiledRule[] = compileHighlightWords(config),
): HighlightSpan[] {
  return highlightBounded(cells, config, rules, 400);
}

function highlightBounded(
  cells: HighlightCell[],
  config: Highlighting,
  rules: CompiledRule[],
  limit: number,
): HighlightSpan[] {
  if (!config.enabled || cells.length > limit) return [];
  let text = "";
  const positions: number[] = [],
    ends: number[] = [],
    plain: boolean[] = [];
  for (let x = 0; x < cells.length; x++) {
    const cell = cells[x];
    if (!cell.width) continue; // trailing cell of a wide glyph
    const chars = cell.chars || " ";
    // Pathological combining sequences should not make this a history-sized scan.
    if (chars.length > 64 || text.length + chars.length > 4096) return [];
    text += chars;
    for (let i = 0; i < chars.length; i++) {
      positions.push(x);
      ends.push(x + cell.width);
      plain.push(cell.plain);
    }
  }
  const spans: HighlightSpan[] = [],
    occupied = new Set<number>();
  const add = (start: number, end: number, color: string) => {
    if (end <= start || start < 0 || end > text.length || spans.length >= 64)
      return;
    // Preserve explicit ANSI colors, inverse, dim and invisible cells.
    for (let i = start; i < end; i++) if (!plain[i] || occupied.has(i)) return;
    for (let i = start; i < end; i++) occupied.add(i);
    spans.push({
      x: positions[start],
      width: ends[end - 1] - positions[start],
      color,
    });
  };
  if (config.ipv6 && /^#[0-9a-f]{6}$/i.test(config.ipv6_color)) {
    for (const match of text.matchAll(/[0-9a-f:.]+(?:%[a-z0-9_.-]+)?/gi)) {
      const token = match[0],
        start = match.index,
        end = start + token.length;
      if (
        !token.includes(":") ||
        token.length > 128 ||
        /[a-z0-9_.:%-]/i.test(text[start - 1] ?? "") ||
        /[a-z0-9_.:%-]/i.test(text[end] ?? "")
      )
        continue;
      try {
        // URL's IPv6 parser validates compression, embedded IPv4 and group counts.
        const address = token.split("%")[0];
        const url = new URL(`http://[${address}]/`);
        if (url.hostname.startsWith("[")) add(start, end, config.ipv6_color);
      } catch {
        /* not a valid IPv6 address */
      }
    }
  }
  if (config.ipv4 && /^#[0-9a-f]{6}$/i.test(config.ipv4_color)) {
    for (const match of text.matchAll(/\b(?:[0-9]{1,3}\.){3}[0-9]{1,3}\b/g)) {
      const start = match.index,
        end = start + match[0].length;
      if (/[\w.]/.test(text[start - 1] ?? "") || /[\w.]/.test(text[end] ?? ""))
        continue;
      if (match[0].split(".").every((part) => Number(part) <= 255))
        add(start, end, config.ipv4_color);
    }
  }
  for (const rule of rules) {
    if (rule.pattern) {
      const matcher = rule.pattern.matcher(text);
      for (let count = 0; count < 64 && matcher.find(); count++)
        add(matcher.start(), matcher.end(), rule.color);
    } else {
      for (const match of text.matchAll(/[a-z0-9_-]+/gi))
        if (rule.words?.has(match[0].toLowerCase()))
          add(match.index, match.index + match[0].length, rule.color);
    }
  }
  return spans.slice(0, 64);
}

export interface HighlightRow {
  cells: HighlightCell[];
  isWrapped: boolean;
}
// Match complete logical lines contained in the viewport. The caller supplies
// only the next row's wrap flag, never offscreen text. Oversized/clipped groups
// are skipped as a whole, so truncation cannot invent word/IP boundaries.
export function highlightViewport(
  rows: HighlightRow[],
  continuesAfter: boolean,
  config: Highlighting,
  rules = compileHighlightWords(config),
): HighlightSpan[][] {
  if (rows.length > 100) return [];
  const result: HighlightSpan[][] = rows.map(() => []);
  if (rows.some((r) => r.cells.length > 400)) return result;
  for (let start = 0; start < rows.length;) {
    let end = start + 1;
    while (end < rows.length && rows[end].isWrapped) end++;
    if (
      !(start === 0 && rows[start].isWrapped) &&
      !(end === rows.length && continuesAfter) &&
      end - start <= 8
    ) {
      const cells: HighlightCell[] = [],
        ranges: { offset: number; length: number }[] = [];
      for (let y = start; y < end; y++) {
        const line = rows[y].cells.slice();
        // xterm leaves a null padding cell when a wide glyph wraps early.
        const padding =
          y + 1 < end &&
          rows[y + 1].cells[0]?.width === 2 &&
          line.at(-1)?.chars === "" &&
          line.at(-1)?.width === 1;
        ranges.push({
          offset: cells.length,
          length: line.length - Number(padding),
        });
        if (padding)
          line[line.length - 1] = { ...line[line.length - 1], width: 0 };
        cells.push(...line);
      }
      for (const span of highlightBounded(cells, config, rules, 1920)) {
        for (let y = start; y < end; y++) {
          const range = ranges[y - start];
          const left = Math.max(span.x, range.offset),
            right = Math.min(span.x + span.width, range.offset + range.length);
          if (right > left)
            result[y].push({
              x: left - range.offset,
              width: right - left,
              color: span.color,
            });
        }
      }
    }
    start = end;
  }
  return result;
}
