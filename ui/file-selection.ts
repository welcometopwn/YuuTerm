// Explorer-style selection for the file list: click, Ctrl+click, Shift+click.
export const MAX_SELECTION = 100;
export interface Selection {
  selected: string[];
  anchor: string | null;
}
export function selectEntry(
  names: string[],
  current: Selection,
  name: string,
  keys: { ctrl?: boolean; shift?: boolean },
): Selection {
  const from = current.anchor === null ? -1 : names.indexOf(current.anchor);
  if (keys.shift && from >= 0) {
    const to = names.indexOf(name);
    const range = names.slice(Math.min(from, to), Math.max(from, to) + 1);
    const base = keys.ctrl ? current.selected : [];
    const merged = names.filter((n) => base.includes(n) || range.includes(n));
    return { selected: merged.slice(0, MAX_SELECTION), anchor: current.anchor };
  }
  if (keys.ctrl) {
    const selected = current.selected.includes(name)
      ? current.selected.filter((n) => n !== name)
      : [...current.selected, name].slice(0, MAX_SELECTION);
    return {
      selected: names.filter((n) => selected.includes(n)),
      anchor: name,
    };
  }
  return { selected: [name], anchor: name };
}
