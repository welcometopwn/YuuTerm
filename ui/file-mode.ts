/** `ls -l` style permission string, e.g. 0o4755 → "rwsr-xr-x". */
export function symbolicMode(mode: number) {
  let text = "";
  for (const [shift, special, mark] of [
    [6, 0o4000, "s"],
    [3, 0o2000, "s"],
    [0, 0o1000, "t"],
  ] as const) {
    const bits = (mode >> shift) & 7;
    const execute = !!(bits & 1);
    text += bits & 4 ? "r" : "-";
    text += bits & 2 ? "w" : "-";
    text +=
      mode & special
        ? execute
          ? mark
          : mark.toUpperCase()
        : execute
          ? "x"
          : "-";
  }
  return text;
}
