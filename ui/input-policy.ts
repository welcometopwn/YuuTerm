export function quotedPath(path: string): string {
  if (/[\x00-\x1f\x7f-\x9f]/.test(path))
    throw new Error(
      "Paths with control characters cannot be inserted into a terminal.",
    );
  if (windowsDrivePath(path)) {
    const drivePath = path.startsWith("/") ? path.slice(1) : path;
    return "'" + drivePath.replaceAll("'", "''") + "'";
  }
  return "'" + path.replaceAll("'", "'\\''") + "'";
}
export function windowsDrivePath(path: string): boolean {
  return /^\/?[A-Za-z]:[\\/]/.test(path);
}
export function displayName(name: string): string {
  return name.replace(
    /[\x00-\x1f\x7f-\x9f]/g,
    (c) => `\\x${c.charCodeAt(0).toString(16).padStart(2, "0")}`,
  );
}
export function requiresPasteConfirmation(text: string): boolean {
  return /[\x00-\x1f\x7f-\x9f]/.test(text);
}
export function assignPane(
  panes: (number | null)[],
  focused: number,
  id: number,
): (number | null)[] {
  return panes.map((old, i) => (i === focused ? id : old === id ? null : old));
}
/** Empty visible panes take open tabs that are not shown yet, in tab order. */
export function fillPanes(
  panes: (number | null)[],
  layout: number,
  open: number[],
): (number | null)[] {
  // Panes only hold open tabs; a closed one never lingers in a hidden slot.
  const out = panes.map((id) => (id !== null && open.includes(id) ? id : null));
  while (out.length < 4) out.push(null);
  const shown = new Set(out.slice(0, layout));
  const spare = open.filter((id) => !shown.has(id));
  for (let i = 0; i < layout && spare.length; i++) {
    if (out[i] !== null) continue;
    const id = spare.shift()!;
    const old = out.indexOf(id);
    if (old >= 0) out[old] = null;
    out[i] = id;
  }
  return out;
}
/** A visible tab only takes focus; a hidden one replaces the focused pane. */
export function showTab(
  panes: (number | null)[],
  layout: number,
  focused: number,
  id: number,
): { panes: (number | null)[]; focused: number } {
  const at = panes.indexOf(id);
  if (at >= 0 && at < layout) return { panes, focused: at };
  return { panes: assignPane(panes, focused, id), focused };
}
/** Put a tab into this pane; one shown in another pane swaps places. */
export function swapPane(
  panes: (number | null)[],
  index: number,
  id: number,
): (number | null)[] {
  const out = [...panes];
  const at = out.indexOf(id);
  if (at >= 0) out[at] = out[index];
  out[index] = id;
  return out;
}
// F1/F2/F3 select the single, two and four pane layouts.
export function layoutForKey(e: {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}): 1 | 2 | 4 | null {
  if (e.ctrlKey || e.altKey || e.shiftKey || e.metaKey) return null;
  return e.key === "F1" ? 1 : e.key === "F2" ? 2 : e.key === "F3" ? 4 : null;
}
