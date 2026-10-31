import { test } from "node:test";
import assert from "node:assert/strict";
import { sortedFiles, columnWidth, fileColumns } from "../ui/file-columns.ts";
import { selectEntry } from "../ui/file-selection.ts";
import type { FileEntry } from "../ui/types.ts";

const entry = (name: string, extra: Partial<FileEntry> = {}): FileEntry => ({
  name,
  directory: false,
  symlink: false,
  size: null,
  permissions: null,
  uid: null,
  gid: null,
  owner: null,
  group: null,
  mtime: null,
  ...extra,
});
test("numeric sizes/dates and account names sort by values; directories first and missing values last", () => {
  const files = [
    entry("file10", { size: 10, mtime: 200, owner: "zoe" }),
    entry("file2", { size: 2, mtime: 100, owner: "alice" }),
    entry("unknown"),
    entry("folder", { directory: true }),
  ];
  const names = (
    column: "name" | "size" | "modified" | "owner",
    direction: "asc" | "desc",
  ) => sortedFiles(files, { column, direction }).map((item) => item.name);
  assert.deepEqual(names("name", "asc"), [
    "folder",
    "file2",
    "file10",
    "unknown",
  ]);
  assert.deepEqual(names("name", "desc"), [
    "folder",
    "unknown",
    "file10",
    "file2",
  ]);
  for (const column of ["size", "modified", "owner"] as const) {
    assert.deepEqual(names(column, "asc"), [
      "folder",
      "file2",
      "file10",
      "unknown",
    ]);
    assert.deepEqual(names(column, "desc"), [
      "folder",
      "file10",
      "file2",
      "unknown",
    ]);
  }
  assert.deepEqual(
    files.map((item) => item.name),
    ["file10", "file2", "unknown", "folder"],
  );
});
test("equal values have stable name ties and shift-selection follows the displayed sort", () => {
  const sorted = sortedFiles(
    [
      entry("c", { size: 10 }),
      entry("a", { size: 10 }),
      entry("b", { size: 2 }),
    ],
    { column: "size", direction: "desc" },
  );
  const names = sorted.map((item) => item.name);
  assert.deepEqual(names, ["a", "c", "b"]);
  assert.deepEqual(
    selectEntry(names, { anchor: "a", selected: ["a"] }, "c", { shift: true })
      .selected,
    ["a", "c"],
  );
});
test("every column has bounded independent resize values", () => {
  for (const column of fileColumns) {
    assert.equal(columnWidth(column.key, -1000), column.min);
    assert.equal(columnWidth(column.key, 10000), column.max);
    assert.equal(columnWidth(column.key, column.width + 16), column.width + 16);
  }
});
