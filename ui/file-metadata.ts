/** Use server account names when available, with explicit numeric fallbacks. */
export function fileOwner(
  uid: number | null,
  gid: number | null,
  owner?: string | null,
  group?: string | null,
): string {
  return `${owner ?? uid ?? "—"} / ${group ?? gid ?? "—"}`;
}

/** Decimal kilobytes rounded to hundredths; the UI tooltip retains exact bytes. */
export function fileKilobytes(size: number | null): string {
  return size === null
    ? "—"
    : (size / 1000).toLocaleString(undefined, {
        maximumFractionDigits: 2,
      });
}

export function fileModified(mtime: number | null): string {
  if (mtime === null) return "—";
  return new Date(mtime * 1000).toLocaleString(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}
