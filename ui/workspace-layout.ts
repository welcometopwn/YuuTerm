import {
  fileColumns,
  defaultColumnWidths,
  columnWidth,
  type ColumnWidths,
  type FileColumn,
  type FileSort,
} from "./file-columns.ts";

export const layoutStorageKey = "yuuterm.workspace-layout.v1";
export interface WorkspaceLayout {
  version: 1;
  sidebarWidth: number;
  sidebarOpen: boolean;
  /** Width of the RDP/SSH remote-tools side panel. */
  toolsWidth: number;
  columns: FileColumn[];
  widths: ColumnWidths;
  sort: FileSort;
}
export function defaultWorkspaceLayout(): WorkspaceLayout {
  return {
    version: 1,
    sidebarWidth: 248,
    sidebarOpen: true,
    toolsWidth: 340,
    columns: fileColumns.map((column) => column.key),
    widths: { ...defaultColumnWidths },
    sort: { column: "name", direction: "asc" },
  };
}
function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}
function isColumn(value: unknown): value is FileColumn {
  return fileColumns.some((column) => column.key === value);
}
export function normalizeWorkspaceLayout(value: unknown): WorkspaceLayout {
  const defaults = defaultWorkspaceLayout(),
    raw = record(value);
  if (raw.version !== 1) return defaults;
  const widths = record(raw.widths);
  for (const column of fileColumns) {
    const width = widths[column.key];
    if (typeof width === "number" && Number.isFinite(width))
      defaults.widths[column.key] = columnWidth(column.key, width);
  }
  if (typeof raw.sidebarWidth === "number" && Number.isFinite(raw.sidebarWidth))
    defaults.sidebarWidth = Math.max(200, Math.min(3000, raw.sidebarWidth));
  if (typeof raw.toolsWidth === "number" && Number.isFinite(raw.toolsWidth))
    defaults.toolsWidth = toolsWidth(raw.toolsWidth);
  if (typeof raw.sidebarOpen === "boolean")
    defaults.sidebarOpen = raw.sidebarOpen;
  if (
    Array.isArray(raw.columns) &&
    raw.columns.length === fileColumns.length &&
    raw.columns.every(isColumn) &&
    new Set(raw.columns).size === fileColumns.length
  )
    defaults.columns = [...raw.columns];
  const sort = record(raw.sort);
  if (
    isColumn(sort.column) &&
    (sort.direction === "asc" || sort.direction === "desc")
  )
    defaults.sort = { column: sort.column, direction: sort.direction };
  return defaults;
}
export const TOOLS_MIN_WIDTH = 240;
export const TOOLS_MAX_WIDTH = 1200;
export function toolsWidth(width: number) {
  return Math.round(
    Math.max(TOOLS_MIN_WIDTH, Math.min(TOOLS_MAX_WIDTH, width)),
  );
}
export function readWorkspaceLayout(
  storage: Pick<Storage, "getItem">,
): WorkspaceLayout {
  try {
    const raw = storage.getItem(layoutStorageKey);
    return raw && raw.length <= 4096
      ? normalizeWorkspaceLayout(JSON.parse(raw))
      : defaultWorkspaceLayout();
  } catch {
    return defaultWorkspaceLayout();
  }
}
export function moveFileColumn(
  columns: readonly FileColumn[],
  source: FileColumn,
  target: FileColumn,
): FileColumn[] {
  const from = columns.indexOf(source),
    to = columns.indexOf(target);
  if (from < 0 || to < 0 || from === to) return [...columns];
  const moved = [...columns];
  moved.splice(from, 1);
  moved.splice(to, 0, source);
  return moved;
}
