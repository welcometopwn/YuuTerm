export const sessionSelection = (
  current: number[],
  id: number,
  ctrl: boolean,
  range?: { shift: boolean; anchor: number | null; order: number[] },
) => {
  if (range?.shift) {
    const from = range.anchor === null ? -1 : range.order.indexOf(range.anchor);
    const to = range.order.indexOf(id);
    const selected =
      from >= 0 && to >= 0
        ? range.order.slice(Math.min(from, to), Math.max(from, to) + 1)
        : [id];
    return ctrl ? [...new Set([...current, ...selected])] : selected;
  }
  return ctrl
    ? current.includes(id)
      ? current.filter((v) => v !== id)
      : [...current, id]
    : [id];
};
export const contextSelection = (current: number[], id: number) =>
  current.includes(id) ? current : [id];
