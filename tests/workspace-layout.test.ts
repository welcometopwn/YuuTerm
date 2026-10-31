import { test } from "node:test";
import assert from "node:assert/strict";
import {
  defaultWorkspaceLayout,
  normalizeWorkspaceLayout,
  readWorkspaceLayout,
  moveFileColumn,
  layoutStorageKey,
} from "../ui/workspace-layout.ts";

test("sidebar and customized column arrangement survive a serialized restart", () => {
  const saved = {
    ...defaultWorkspaceLayout(),
    sidebarWidth: 760,
    sidebarOpen: false,
    toolsWidth: 520,
    columns: ["owner", "name", "size", "modified"],
    widths: { name: 196, size: 106, modified: 186, owner: 216 },
    sort: { column: "size", direction: "desc" },
  };
  const storage = {
    getItem(key: string) {
      assert.equal(key, layoutStorageKey);
      return JSON.stringify(saved);
    },
  };
  assert.deepEqual(readWorkspaceLayout(storage), saved);
});
test("damaged, future and oversized layout storage safely fall back without writing", () => {
  for (const text of [
    "{",
    "null",
    "[]",
    JSON.stringify({ version: 2, sidebarWidth: 900 }),
    " ".repeat(4097),
  ])
    assert.deepEqual(
      readWorkspaceLayout({ getItem: () => text }),
      defaultWorkspaceLayout(),
    );
  assert.deepEqual(
    readWorkspaceLayout({
      getItem() {
        throw Error("storage unavailable");
      },
    }),
    defaultWorkspaceLayout(),
  );
});
test("partial layout validation bounds sizes and refuses duplicate/unknown columns and invalid sort", () => {
  const result = normalizeWorkspaceLayout({
    version: 1,
    sidebarWidth: 1e8,
    sidebarOpen: "yes",
    toolsWidth: 5,
    widths: { name: 1e8, size: -5, owner: NaN, modified: Infinity },
    columns: ["name", "name", "size", "other"],
    sort: { column: "unknown", direction: "desc" },
  });
  assert.equal(result.sidebarWidth, 3000);
  assert.equal(result.sidebarOpen, true);
  assert.equal(result.toolsWidth, 240);
  assert.deepEqual(result.widths, {
    name: 1200,
    size: 64,
    modified: 170,
    owner: 200,
  });
  assert.deepEqual(result.columns, defaultWorkspaceLayout().columns);
  assert.deepEqual(result.sort, defaultWorkspaceLayout().sort);
});
test("moving columns preserves identities and leaves the source order unchanged", () => {
  const columns = defaultWorkspaceLayout().columns;
  const moved = moveFileColumn(columns, "owner", "name");
  assert.deepEqual(moved, ["owner", "name", "size", "modified"]);
  assert.deepEqual(columns, ["name", "size", "modified", "owner"]);
  assert.deepEqual(moveFileColumn(moved, "owner", "modified"), [
    "name",
    "size",
    "modified",
    "owner",
  ]);
  assert.deepEqual(moveFileColumn(columns, "name", "name"), columns);
});
