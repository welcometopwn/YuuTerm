import type { FileEntry } from "./types";
import { fileOwner } from "./file-metadata.ts";

export const fileColumns = [
  { key: "name", label: "Name", width: 180, min: 100, max: 1200 },
  { key: "size", label: "Size (KB)", width: 90, min: 64, max: 800 },
  { key: "modified", label: "Last modified", width: 170, min: 100, max: 800 },
  { key: "owner", label: "Owner / group", width: 200, min: 110, max: 1000 },
] as const;
export type FileColumn = (typeof fileColumns)[number]["key"];
export type FileSort = { column: FileColumn; direction: "asc" | "desc" };
export type ColumnWidths = Record<FileColumn, number>;
export const defaultColumnWidths: ColumnWidths = Object.fromEntries(
  fileColumns.map((column) => [column.key, column.width]),
) as ColumnWidths;

export function columnWidth(column: FileColumn, width: number): number {
  const limits = fileColumns.find((item) => item.key === column)!;
  return Math.min(limits.max, Math.max(limits.min, width));
}

const collator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: "base",
});
function value(entry: FileEntry, column: FileColumn): string | number | null {
  switch (column) {
    case "name":
      return entry.name;
    case "size":
      return entry.directory ? null : entry.size;
    case "modified":
      return entry.mtime;
    case "owner":
      return entry.owner == null &&
        entry.group == null &&
        entry.uid == null &&
        entry.gid == null
        ? null
        : fileOwner(entry.uid, entry.gid, entry.owner, entry.group);
  }
}

/** Directories stay first and missing metadata stays last in either direction. */
export function sortedFiles(
  entries: readonly FileEntry[],
  sort: FileSort,
): FileEntry[] {
  return [...entries].sort((a, b) => {
    if (a.directory !== b.directory) return a.directory ? -1 : 1;
    const left = value(a, sort.column),
      right = value(b, sort.column);
    if (left == null && right != null) return 1;
    if (right == null && left != null) return -1;
    const difference =
      left == null || right == null
        ? 0
        : typeof left === "number" && typeof right === "number"
          ? left - right
          : collator.compare(String(left), String(right));
    return (
      (sort.direction === "asc" ? difference : -difference) ||
      collator.compare(a.name, b.name) ||
      a.name.localeCompare(b.name)
    );
  });
}
