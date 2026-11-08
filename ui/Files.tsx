import {
  useState,
  useEffect,
  useRef,
  useMemo,
  type CSSProperties,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Folder,
  File as FileIcon,
  ArrowUp,
  RefreshCw,
  Play,
  Upload,
  MoreHorizontal,
  X,
  Link2,
  FolderPlus,
  FilePlus,
  Download,
  Trash2,
  Columns2,
} from "lucide-react";
import { WindowsPermissions } from "./WindowsPermissions";
import { PosixPermissions } from "./PosixPermissions";
import { Modal } from "./Modal";
import { ContextMenu } from "./ContextMenu";
import type { Live, FileEntry } from "./types";
import { quotedPath, displayName, windowsDrivePath } from "./input-policy";
import { selectEntry } from "./file-selection";
import { FileNavigation, FileClickPairs } from "./file-navigation";
import { fileKilobytes, fileModified, fileOwner } from "./file-metadata";
import {
  fileColumns,
  columnWidth,
  sortedFiles,
  type FileColumn,
} from "./file-columns";
import {
  useWorkspaceLayout,
  updateWorkspaceLayout,
} from "./use-workspace-layout";
import { moveFileColumn, type WorkspaceLayout } from "./workspace-layout";
interface Properties {
  windows_permissions?: boolean;
  size: number | null;
  permissions: number | null;
  uid: number | null;
  gid: number | null;
  mtime: number | null;
  directory: boolean;
  symlink: boolean;
}
interface Result {
  bytes: number;
  changed?: number;
  skipped?: number;
  properties: Properties | null;
  resolved?: { path: string; directory: boolean } | null;
}
interface Edit {
  id: number;
  live_id: number;
  remote: string;
  local: string;
}
interface Target {
  entry: FileEntry;
  path: string;
  id: number;
}
type Dialog = {
  kind: "rename" | "delete" | "permissions" | "properties";
  target: Target;
  properties?: Properties;
};
function FileCell({ entry, column }: { entry: FileEntry; column: FileColumn }) {
  switch (column) {
    case "name":
      return (
        <span className="file-name">
          {entry.symlink ? (
            <Link2 size={14} aria-label="Symbolic link" />
          ) : entry.directory ? (
            <Folder size={14} />
          ) : (
            <FileIcon size={14} />
          )}
          <span>{displayName(entry.name)}</span>
        </span>
      );
    case "size":
      return (
        <span
          className="file-size"
          aria-label="Size in KB"
          title={
            entry.size === null
              ? "Unknown size"
              : `${entry.size} bytes; 1 KB = 1000 bytes`
          }
        >
          {entry.directory ? "—" : fileKilobytes(entry.size)}
        </span>
      );
    case "modified":
      return (
        <span
          className="file-modified"
          aria-label="Last modified"
          title={
            entry.mtime === null
              ? "Unknown modification time"
              : new Date(entry.mtime * 1000).toISOString()
          }
        >
          {fileModified(entry.mtime)}
        </span>
      );
    case "owner":
      return (
        <span
          className="file-owner"
          aria-label="Owner / group"
          title={`${fileOwner(entry.uid, entry.gid, entry.owner, entry.group)}\nUID: ${entry.uid ?? "Unknown"} / GID: ${entry.gid ?? "Unknown"}`}
        >
          {fileOwner(entry.uid, entry.gid, entry.owner, entry.group)}
        </span>
      );
  }
}
export function Files({
  live,
  error,
  onEditOpened,
  onFocusTerminal,
  autoUpload,
  onAskUploads,
  expandable = false,
  expanded = false,
  onResize,
  onFolderSizes,
}: {
  onEditOpened: () => void;
  onFocusTerminal: () => void;
  autoUpload: boolean;
  onAskUploads: () => void;
  expandable?: boolean;
  expanded?: boolean;
  onResize?: (expanded: boolean) => void;
  /** Opens the SSH tools storage view for a folder; absent where unsupported. */
  onFolderSizes?: (path: string) => void;
  live?: Live;
  error: (e: unknown) => void;
}) {
  const layout = useWorkspaceLayout();
  const sort = layout.sort,
    columnWidths = layout.widths;
  const saveLayout = (patch: Partial<Omit<WorkspaceLayout, "version">>) => {
    try {
      updateWorkspaceLayout(patch);
    } catch (e) {
      error(`Could not save file layout: ${e}`);
    }
  };
  const orderedColumns = layout.columns.map((key) =>
    fileColumns.find((column) => column.key === key)!,
  );
  const columnReorder = useRef<FileColumn | null>(null);
  const columnDrag = useRef<{
    column: FileColumn;
    x: number;
    width: number;
  } | null>(null);
  const entries = useMemo(
    () => sortedFiles(live?.meta?.files ?? [], sort),
    [live?.meta?.files, sort.column, sort.direction],
  );
  const resizeColumn = (column: FileColumn, width: number) =>
    saveLayout({
      widths: { ...columnWidths, [column]: columnWidth(column, width) },
    });
  const columnStyle = {
    "--file-grid": layout.columns
      .map((column) => `${columnWidths[column]}px`)
      .join(" "),
    "--file-table-width": `${Object.values(columnWidths).reduce((sum, width) => sum + width, 39)}px`,
  } as CSSProperties;
  const [path, setPath] = useState(""),
    [dialog, setDialog] = useState<Dialog | null>(null),
    [name, setName] = useState(""),
    [mode, setMode] = useState(""),
    [recursive, setRecursive] = useState(false),
    [busy, setBusy] = useState(false),
    [problem, setProblem] = useState(""),
    [edits, setEdits] = useState<Edit[]>([]);
  const [recoverable, setRecoverable] = useState<Edit[]>([]);
  const [recovering, setRecovering] = useState<Edit | null>(null);
  // Files kept from a previous connection are offered one at a time in a
  // dialog after reconnecting ("Not now" keeps the local copy).
  const [notNow, setNotNow] = useState<number[]>([]);
  const [fileMenu, setFileMenu] = useState<{
    target: Target | null;
    id: number;
    directory: string;
    x: number;
    y: number;
  } | null>(null);
  const [newFile, setNewFile] = useState<{
    id: number;
    directory: string;
    kind?: "file" | "directory";
  } | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const anchor = useRef<string | null>(null);
  const [bulkDelete, setBulkDelete] = useState<Target[] | null>(null);
  const operation = useRef(false);
  const stopBatch = useRef(false);
  const current = useRef(live);
  current.current = live;
  const navigationError = useRef(error);
  navigationError.current = error;
  const navigation = useRef<FileNavigation | null>(null);
  const clickPairs = useRef(new FileClickPairs());
  if (!navigation.current)
    navigation.current = new FileNavigation(
      (id, path) => invoke<string>("session_browse", { id, path }),
      (message) => navigationError.current(String(message)),
    );
  useEffect(() => {
    navigation.current!.sync(
      live?.id ?? null,
      live?.meta?.path ?? "",
      !!live?.meta?.connected && !live?.meta?.finished,
    );
  }, [live?.id, live?.meta?.path, live?.meta?.connected, live?.meta?.finished]);
  useEffect(() => () => navigation.current!.dispose(), []);
  useEffect(() => {
    setPath(live?.meta?.path ?? "");
    setSelected([]);
    anchor.current = null;
    setBulkDelete(null);
  }, [live?.id, live?.meta?.path]);
  useEffect(() => {
    let active = true;
    if (live) {
      void invoke<Edit[]>("edit_list", { id: live.id })
        .then((v) => {
          if (active) setEdits(v);
        })
        .catch((e) => {
          if (active) error(e);
        });
      if (live.meta?.connected && !live.meta.finished)
        void invoke<Edit[]>("edit_recoverable", { id: live.id })
          .then((v) => {
            if (active) setRecoverable(v);
          })
          .catch((e) => {
            if (active) error(e);
          });
    }
    return () => {
      active = false;
    };
  }, [live?.id, live?.meta?.connected]);
  const menu = (target: Target, x: number, y: number) => {
    setProblem("");
    setFileMenu({
      target,
      x,
      y,
      id: target.id,
      directory: current.current?.meta?.path ?? "",
    });
  };
  useEffect(() => {
    setFileMenu(null);
    setDialog(null);
    setRecovering(null);
    setNewFile(null);
  }, [live?.id]);
  const connected = !!live?.meta?.connected;
  useEffect(() => {
    if (!connected || recovering) return;
    const next = recoverable.find((e) => !notNow.includes(e.id));
    if (next) setRecovering(next);
  }, [connected, recovering, recoverable, notNow]);
  // Native file drops: the backend keeps the paths and sends a single-use
  // token with the drop position. Upload into the current remote directory
  // when the drop landed on this panel.
  const panel = useRef<HTMLDivElement>(null);
  const dropState = useRef({ live, connected });
  dropState.current = { live, connected };
  useEffect(() => {
    const pending = listen<{ token: number; x: number; y: number }>(
      "files-dropped",
      (event) => {
        const { live, connected } = dropState.current;
        const rect = panel.current?.getBoundingClientRect();
        const scale = window.devicePixelRatio;
        const x = event.payload.x / scale,
          y = event.payload.y / scale;
        if (
          !live?.meta?.path ||
          !connected ||
          !rect ||
          rect.width === 0 ||
          x < rect.left ||
          x > rect.right ||
          y < rect.top ||
          y > rect.bottom
        )
          return;
        const directory = live.meta.path;
        void invoke<{ uploaded: number; bytes: number; skipped: string[] }>(
          "file_upload_dropped",
          { id: live.id, path: directory, token: event.payload.token },
        )
          .then((r) => {
            error(
              `Uploaded ${r.uploaded} file${r.uploaded === 1 ? "" : "s"} (${r.bytes} bytes)` +
                (r.skipped.length ? `; skipped: ${r.skipped.join(", ")}` : ""),
            );
            void invoke("session_browse", { id: live.id, path: directory });
          })
          .catch(error);
      },
    );
    return () => {
      void pending.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);
  const join = (name: string) =>
    `${live?.meta?.path.replace(/\/$/, "") ?? ""}/${name}`;
  const browse = (path: string) => {
    if (live && connected && !busy)
      navigation.current!.navigate(live.id, { kind: "path", path });
  };
  const refreshRef = useRef(() => {});
  refreshRef.current = () => browse(live?.meta?.path ?? ".");
  useEffect(() => {
    const onRefresh = () => refreshRef.current();
    window.addEventListener("yuuterm:refresh-files", onRefresh);
    return () => window.removeEventListener("yuuterm:refresh-files", onRefresh);
  }, []);
  const browseParent = () => {
    if (live && connected && !busy)
      navigation.current!.navigate(live.id, { kind: "parent" });
  };
  const create = (kind: "file" | "directory") => {
    if (!live?.meta?.path || !connected || busy) return;
    setName("");
    setProblem("");
    setNewFile({ id: live.id, directory: live.meta.path, kind });
  };
  const creating = useRef(false);
  useEffect(() => {
    creating.current = false;
  }, [newFile]);
  const submitNew = async () => {
    if (!newFile || creating.current || busy) return;
    if (!name.trim()) {
      setNewFile(null);
      setProblem("");
      return;
    }
    if (name.includes("/") || name === "." || name === "..") {
      setProblem("Names cannot contain / or be . or ..");
      return;
    }
    creating.current = true;
    let created = false;
    try {
      const result = await run<Result>(
        newFile.id,
        "file_action",
        {
          action: {
            kind: newFile.kind === "directory" ? "mkdir" : "create",
            path: newFile.directory,
            name,
          },
        },
        false,
      );
      if (result !== undefined) {
        created = true;
        setNewFile(null);
        error(
          newFile.kind === "directory" ? "Directory created" : "File created",
        );
      }
    } finally {
      // After success the row unmounts; a late blur must not create twice.
      if (!created) creating.current = false;
    }
  };
  const targets = (live?.meta?.files ?? [])
    .filter((e) => selected.includes(e.name))
    .map((entry) => ({ entry, path: join(entry.name), id: live!.id }));
  const names = entries.map((e) => e.name);
  // Functional update so rapid clicks never act on a stale selection.
  const pick = (name: string, keys: { ctrl?: boolean; shift?: boolean }) =>
    setSelected((prev) => {
      const next = selectEntry(
        names,
        { selected: prev, anchor: anchor.current },
        name,
        keys,
      );
      anchor.current = next.anchor;
      return next.selected;
    });
  const clearSelection = () => {
    setSelected([]);
    anchor.current = null;
  };
  const canDownload =
    !busy &&
    connected &&
    targets.length > 0 &&
    !targets.some((t) => t.entry.symlink);
  const requestDelete = () => {
    if (busy || !connected || !targets.length) return;
    setProblem("");
    setBulkDelete(targets);
  };
  const downloadSelected = async (items: Target[] = targets) => {
    if (!live || operation.current || !items.length) return;
    operation.current = true;
    try {
      const result = await run<{
        completed: number;
        problem: string | null;
        cancelled: boolean;
        skipped: number;
      }>(live.id, "files_download", { paths: items.map((t) => t.path) });
      if (result && !result.cancelled) {
        const skipped = result.skipped
          ? ` · ${result.skipped} symlinks/special files skipped`
          : "";
        if (result.problem)
          setProblem(
            `${result.completed} downloaded${skipped}. ${result.problem}`,
          );
        else {
          setSelected([]);
          error(
            `Downloaded ${result.completed} ${result.completed === 1 ? "item" : "items"}${skipped}`,
          );
        }
      }
    } finally {
      operation.current = false;
    }
  };
  const deleteSelected = async () => {
    if (!bulkDelete || operation.current) return;
    operation.current = true;
    const snapshot = bulkDelete;
    stopBatch.current = false;
    setBusy(true);
    setProblem("");
    let completed = 0;
    try {
      for (const target of snapshot) {
        if (stopBatch.current)
          throw new Error("Cancelled; remaining items were retained.");
        if (
          current.current?.id !== target.id ||
          !current.current.meta?.connected
        )
          throw new Error("Original connection is no longer active.");
        await invoke("file_action", {
          id: target.id,
          action: { kind: "delete", path: target.path },
        });
        completed++;
        setBulkDelete(
          (items) => items?.filter((t) => t.path !== target.path) ?? null,
        );
        setSelected((s) => s.filter((name) => name !== target.entry.name));
      }
      setBulkDelete(null);
      error(`Deleted ${completed} items`);
    } catch (e) {
      setProblem(`${completed} deleted; stopped. ${String(e)}`);
    } finally {
      setBusy(false);
      operation.current = false;
    }
  };
  const run = async <T,>(
    id: number,
    command: string,
    args: object,
    notify = true,
  ): Promise<T | undefined> => {
    if (current.current?.id !== id || !current.current.meta?.connected) {
      setProblem("Original connection is no longer active.");
      return;
    }
    setBusy(true);
    setProblem("");
    try {
      return await invoke<T>(command, { id, ...args });
    } catch (e) {
      setProblem(String(e));
      if (notify) error(e);
      return undefined;
    } finally {
      setBusy(false);
    }
  };
  const action = async (d: Dialog, kind: string, extra: object = {}) => {
    const result = await run<Result>(d.target.id, "file_action", {
      action: { kind, path: d.target.path, ...extra },
    });
    if (result !== undefined) {
      setDialog(null);
      if (kind === "permissions") {
        if (result.skipped) {
          error(
            `Permissions applied to ${result.changed ?? 0} items · ${result.skipped} symlinks/special files skipped`,
          );
        }
        return;
      }
      error("File operation completed");
    }
  };
  const inspect = async (
    target: Target,
    kind: "permissions" | "properties",
  ) => {
    const result = await run<Result>(target.id, "file_action", {
      action: { kind: "properties", path: target.path },
    });
    if (result?.properties) {
      setMode(
        ((result.properties.permissions ?? 0) & 0o7777)
          .toString(8)
          .padStart(3, "0"),
      );
      setRecursive(false);
      setDialog({ kind, target, properties: result.properties });
    }
  };
  const resolved = async (target: Target): Promise<Target | undefined> => {
    if (!target.entry.symlink) return target;
    const result = await run<Result>(target.id, "file_action", {
      action: { kind: "resolve", path: target.path },
    });
    if (!result?.resolved) return;
    return {
      ...target,
      path: result.resolved.path,
      entry: {
        ...target.entry,
        directory: result.resolved.directory,
        symlink: false,
      },
    };
  };
  const activate = async (target: Target) => {
    const destination = await resolved(target);
    if (!destination) return;
    if (destination.entry.directory) {
      browse(destination.path);
      setDialog(null);
    } else await open(destination);
  };
  const containingDirectory = async (target: Target) => {
    const result = await run<Result>(target.id, "file_action", {
      action: { kind: "resolve", path: target.path },
    });
    if (result?.resolved) {
      browse(
        result.resolved.directory
          ? result.resolved.path
          : result.resolved.path.slice(
              0,
              result.resolved.path.lastIndexOf("/"),
            ) || "/",
      );
      setDialog(null);
    }
  };
  const nano = async (target: Target) => {
    const destination = await resolved(target);
    if (!destination || destination.entry.directory) return;
    const result = await run<null>(target.id, "session_nano", {
      path: destination.path,
    });
    if (result !== undefined) {
      setDialog(null);
      onFocusTerminal();
    }
  };
  const open = async (target: Target, choose = false) => {
    const result = await run<Edit | null>(target.id, "edit_open", {
      path: target.path,
      choose,
    });
    if (result) {
      setEdits((a) => [...a.filter((e) => e.id !== result.id), result]);
      setDialog(null);
      onEditOpened();
    }
  };
  const transfer = async (id: number, path: string, upload: boolean) => {
    const result = await run<{ cancelled: boolean; bytes: number }>(
      id,
      "file_transfer",
      { path, upload },
    );
    if (result && !result.cancelled) {
      setDialog(null);
      error(`${upload ? "Uploaded" : "Downloaded"} ${result.bytes} bytes`);
    }
  };
  const copy = (text: string) => {
    void navigator.clipboard
      .writeText(text)
      .then(() => {
        setDialog(null);
        error("File path copied");
      })
      .catch((e) => setProblem(String(e)));
  };
  const insert = (target: Target) => {
    try {
      const bytes = Array.from(
        new TextEncoder().encode(quotedPath(target.path)),
      );
      void run<null>(target.id, "session_input", { bytes }).then((result) => {
        if (result !== undefined) {
          setDialog(null);
          onFocusTerminal();
        }
      });
    } catch (e) {
      setProblem(String(e));
    }
  };
  return (
    <div
      ref={panel}
      className={`files-panel ${expanded ? "files-expanded" : ""}`}
      data-watched-edits={edits.length}
      onMouseDown={(e) => {
        if (
          e.button === 3 &&
          connected &&
          !busy &&
          !(e.target as HTMLElement).closest(
            "input, textarea, [contenteditable=true]",
          )
        ) {
          e.preventDefault();
          e.stopPropagation();
          browseParent();
        }
      }}
      onMouseUp={(e) => {
        if (e.button === 3) e.preventDefault();
      }}
      onAuxClick={(e) => {
        if (e.button === 3) {
          e.preventDefault();
          e.stopPropagation();
        }
      }}
      onContextMenu={(e) => {
        e.stopPropagation();
        if (
          (e.target as HTMLElement).closest(
            "input, textarea, [contenteditable=true]",
          )
        )
          return;
        e.preventDefault();
        if (document.querySelector("dialog[open]")) return;
        setFileMenu({
          target: null,
          id: live?.id ?? 0,
          directory: live?.meta?.path ?? "",
          x: e.clientX,
          y: e.clientY,
        });
      }}
    >
      {live ? (
        <>
          <div className="file-host">
            {live.meta?.endpoint ?? "Connecting…"}
          </div>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              browse(path);
            }}
          >
            <input
              aria-label="Remote path"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              maxLength={4096}
            />
            <button
              className="icon"
              title="Open path"
              aria-label="Open path"
              disabled={!connected || busy}
            >
              <Play size={12} />
            </button>
          </form>
          <div className="file-tools">
            {expandable && (
              <button
                className="icon"
                title={
                  expanded
                    ? "Narrow file browser"
                    : "Widen file browser to show details"
                }
                aria-label="Widen file browser"
                aria-pressed={expanded}
                onClick={() => {
                  onResize?.(!expanded);
                }}
              >
                <Columns2 />
              </button>
            )}
            <button
              className="icon"
              title="Parent directory"
              aria-label="Parent directory"
              disabled={!connected || busy}
              onClick={browseParent}
            >
              <ArrowUp />
            </button>
            <button
              className="icon"
              title="Refresh files"
              aria-label="Refresh files"
              disabled={!connected || busy}
              onClick={() => browse(live.meta?.path ?? ".")}
            >
              <RefreshCw />
            </button>
            <button
              className="icon"
              title="Upload file"
              aria-label="Upload file"
              disabled={!connected || busy || !live.meta?.path}
              onClick={() =>
                void transfer(live.id, live.meta?.path ?? ".", true)
              }
            >
              <Upload />
            </button>
            {autoUpload && (
              <button
                className="auto-upload-on"
                title="Saved files upload without asking on this connection. Click to ask again."
                aria-label="Ask before uploads"
                onClick={onAskUploads}
              >
                Auto-upload on
              </button>
            )}
            <button
              className="icon"
              title="New directory"
              aria-label="New directory"
              disabled={!connected || busy || !live.meta?.path}
              onClick={() => create("directory")}
            >
              <FolderPlus />
            </button>
            <button
              className="icon"
              title="New file"
              aria-label="New file"
              disabled={!connected || busy || !live.meta?.path}
              onClick={() => create("file")}
            >
              <FilePlus />
            </button>
            <button
              className="icon"
              aria-label="Download selected files"
              title="Download selected files and directories"
              disabled={!canDownload}
              onClick={() => void downloadSelected()}
            >
              <Download />
            </button>
            <button
              className="icon"
              aria-label="Delete selected items"
              title="Delete selected items (Del)"
              disabled={busy || !connected || !targets.length}
              onClick={requestDelete}
            >
              <Trash2 />
            </button>
            {busy && (
              <button
                className="icon"
                title="Cancel file operation"
                aria-label="Cancel file operation"
                onClick={() =>
                  void invoke("file_cancel", { id: live.id }).catch(error)
                }
              >
                <X />
              </button>
            )}
          </div>
          <div className="file-status" role="status">
            {live.meta?.files_status ?? "Waiting for SSH"}
            {selected.length > 0 && ` · ${selected.length} selected`}
          </div>
          {problem && !recovering && (
            <div className="file-error" role="alert">
              {problem}
            </div>
          )}
          <div
            className="file-list"
            style={columnStyle}
            onClick={(e) => {
              if (e.target === e.currentTarget) clearSelection();
            }}
            onKeyDown={(e) => {
              if (
                (e.target as HTMLElement).closest(
                  "input, textarea, [contenteditable=true]",
                )
              )
                return;
              if (e.key === "Delete") {
                e.preventDefault();
                requestDelete();
              } else if (e.key === "Escape" && selected.length) {
                e.preventDefault();
                clearSelection();
              } else if (e.key.toLowerCase() === "a" && e.ctrlKey) {
                e.preventDefault();
                if (names.length) {
                  anchor.current = names[0];
                  setSelected(names.slice(0, 100));
                }
              }
            }}
          >
            <div className="file-columns">
              {orderedColumns.map((column) => (
                <div
                  className="file-column"
                  key={column.key}
                  data-column={column.key}
                >
                  <button
                    type="button"
                    className="file-sort"
                    draggable
                    onDragStart={(event) => {
                      columnReorder.current = column.key;
                      event.dataTransfer.effectAllowed = "move";
                      event.dataTransfer.setData(
                        "application/x-yuuterm-column",
                        column.key,
                      );
                    }}
                    onDragEnd={() => {
                      columnReorder.current = null;
                    }}
                    onDragOver={(event) => {
                      if (columnReorder.current) {
                        event.preventDefault();
                        event.dataTransfer.dropEffect = "move";
                      }
                    }}
                    onDrop={(event) => {
                      const source = columnReorder.current;
                      if (!source) return;
                      event.preventDefault();
                      event.stopPropagation();
                      columnReorder.current = null;
                      saveLayout({
                        columns: moveFileColumn(
                          layout.columns,
                          source,
                          column.key,
                        ),
                      });
                    }}
                    onKeyDown={(event) => {
                      if (
                        !event.altKey ||
                        !["ArrowLeft", "ArrowRight"].includes(event.key)
                      )
                        return;
                      event.preventDefault();
                      event.stopPropagation();
                      const index = layout.columns.indexOf(column.key),
                        target =
                          layout.columns[
                            index + (event.key === "ArrowRight" ? 1 : -1)
                          ];
                      if (target)
                        saveLayout({
                          columns: moveFileColumn(
                            layout.columns,
                            column.key,
                            target,
                          ),
                        });
                    }}
                    data-sort-column={column.key}
                    data-direction={
                      sort.column === column.key ? sort.direction : undefined
                    }
                    aria-label={`Sort by ${column.label}${sort.column === column.key ? `, currently ${sort.direction === "asc" ? "ascending" : "descending"}` : ""}`}
                    aria-pressed={sort.column === column.key}
                    title={`Sort by ${column.label}; ${sort.column === column.key && sort.direction === "asc" ? "descending" : "ascending"}. Drag to reorder; Alt+Left/Right moves the column.`}
                    onClick={() =>
                      saveLayout({
                        sort: {
                          column: column.key,
                          direction:
                            sort.column === column.key &&
                            sort.direction === "asc"
                              ? "desc"
                              : "asc",
                        },
                      })
                    }
                  >
                    <span>{column.label}</span>
                    {sort.column === column.key && (
                      <span
                        aria-label={
                          sort.direction === "asc" ? "Ascending" : "Descending"
                        }
                      >
                        {sort.direction === "asc" ? "↑" : "↓"}
                      </span>
                    )}
                  </button>
                  <div
                    className="file-column-resizer"
                    role="separator"
                    aria-label={`Resize ${column.label} column`}
                    aria-orientation="vertical"
                    aria-valuemin={column.min}
                    aria-valuemax={column.max}
                    aria-valuenow={columnWidths[column.key]}
                    tabIndex={0}
                    title="Drag to resize; double-click to reset"
                    onPointerDown={(event) => {
                      if (event.button !== 0) return;
                      event.preventDefault();
                      event.stopPropagation();
                      columnDrag.current = {
                        column: column.key,
                        x: event.clientX,
                        width:
                          event.currentTarget.parentElement!.getBoundingClientRect()
                            .width,
                      };
                      event.currentTarget.setPointerCapture(event.pointerId);
                    }}
                    onPointerMove={(event) => {
                      const drag = columnDrag.current;
                      if (
                        drag?.column === column.key &&
                        event.currentTarget.hasPointerCapture(event.pointerId)
                      )
                        resizeColumn(
                          column.key,
                          drag.width + event.clientX - drag.x,
                        );
                    }}
                    onPointerUp={(event) => {
                      if (
                        event.currentTarget.hasPointerCapture(event.pointerId)
                      )
                        event.currentTarget.releasePointerCapture(
                          event.pointerId,
                        );
                      columnDrag.current = null;
                    }}
                    onLostPointerCapture={() => {
                      columnDrag.current = null;
                    }}
                    onDoubleClick={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                      resizeColumn(column.key, column.width);
                    }}
                    onKeyDown={(event) => {
                      if (
                        !["ArrowLeft", "ArrowRight", "Home", "End"].includes(
                          event.key,
                        )
                      )
                        return;
                      event.preventDefault();
                      event.stopPropagation();
                      resizeColumn(
                        column.key,
                        event.key === "Home"
                          ? column.min
                          : event.key === "End"
                            ? column.max
                            : columnWidths[column.key] +
                              (event.key === "ArrowRight" ? 16 : -16),
                      );
                    }}
                  />
                </div>
              ))}
              <span />
            </div>
            {live.meta?.path && (
              <div className="file-row directory parent-directory">
                <button
                  aria-label="Go to parent directory"
                  disabled={!connected || busy}
                  onClick={(event) => {
                    if (
                      clickPairs.current.continued(
                        `${live.id}:${live.meta?.path}/..`,
                        event.detail,
                        event.button,
                      )
                    )
                      browseParent();
                  }}
                  onDoubleClick={(event) => {
                    if (event.detail <= 2) browseParent();
                  }}
                >
                  <span
                    className="file-name"
                    style={{ gridColumn: layout.columns.indexOf("name") + 1 }}
                  >
                    <Folder size={14} />
                    <span>..</span>
                  </span>
                </button>
              </div>
            )}
            {newFile && newFile.id === live.id && (
              <form
                className={`file-row file-new ${newFile.kind === "directory" ? "directory" : ""}`}
                onSubmit={(e) => {
                  e.preventDefault();
                  void submitNew();
                }}
              >
                {newFile.kind === "directory" ? (
                  <Folder size={14} />
                ) : (
                  <FileIcon size={14} />
                )}
                <input
                  autoFocus
                  aria-label="Filename"
                  placeholder={
                    newFile.kind === "directory"
                      ? "New directory name"
                      : "New file name"
                  }
                  value={name}
                  maxLength={255}
                  readOnly={busy}
                  onChange={(e) => {
                    setName(e.target.value);
                    setProblem("");
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") {
                      e.preventDefault();
                      e.stopPropagation();
                      setNewFile(null);
                      setProblem("");
                    }
                  }}
                  onBlur={() => {
                    // Explorer-style: leaving the field commits a typed name.
                    if (!problem) void submitNew();
                  }}
                />
              </form>
            )}
            {entries.map((entry) => {
              const target = { entry, path: join(entry.name), id: live.id };
              return (
                <div
                  className={`file-row ${entry.directory ? "directory" : ""} ${selected.includes(entry.name) ? "file-selected" : ""}`}
                  key={entry.name}
                  data-name={entry.name}
                >
                  <button
                    aria-pressed={selected.includes(entry.name)}
                    disabled={!connected || busy}
                    onClick={(e) => {
                      pick(entry.name, {
                        ctrl: e.ctrlKey || e.metaKey,
                        shift: e.shiftKey,
                      });
                      if (
                        clickPairs.current.continued(
                          `${target.id}:${target.path}`,
                          e.detail,
                          e.button,
                        )
                      )
                        void activate(target);
                    }}
                    title={`${entry.name}\n${entry.size ?? "Unknown size"} bytes · ${entry.permissions?.toString(8) ?? "Unknown mode"}`}
                    onDoubleClick={(event) => {
                      if (event.detail <= 2) void activate(target);
                    }}
                    onMouseDown={(e) => {
                      if (e.button === 1) e.preventDefault();
                    }}
                    onAuxClick={(e) => {
                      if (e.button === 1) {
                        e.preventDefault();
                        e.stopPropagation();
                        insert(target);
                      }
                    }}
                    onContextMenu={(e) => {
                      e.stopPropagation();
                      e.preventDefault();
                      if (!selected.includes(entry.name)) pick(entry.name, {});
                      menu(target, e.clientX, e.clientY);
                    }}
                  >
                    {layout.columns.map((column) => (
                      <FileCell key={column} entry={entry} column={column} />
                    ))}
                  </button>
                  <button
                    className="icon file-more"
                    aria-label={`Actions for ${displayName(entry.name)}`}
                    disabled={!connected || busy}
                    onClick={(e) => {
                      const box = e.currentTarget.getBoundingClientRect();
                      menu(
                        target,
                        e.detail ? e.clientX : box.left,
                        e.detail ? e.clientY : box.bottom,
                      );
                    }}
                  >
                    <MoreHorizontal size={14} />
                  </button>
                </div>
              );
            })}
          </div>
        </>
      ) : (
        <div className="side-empty">
          Connect an SSH session
          <br />
          to browse its files.
        </div>
      )}
      {recovering && live && (
        <Modal
          title="Resume local edit"
          onClose={
            busy
              ? undefined
              : () => {
                  setNotNow((a) => [...a, recovering.id]);
                  setRecovering(null);
                }
          }
        >
          <p className="remote-target">{displayName(recovering.remote)}</p>
          <p>
            This file was open for editing when the connection ended. Resume it
            on the new connection? The server file must still match its previous
            content; then your saved local changes ask before upload. Automatic
            uploads are turned off.
          </p>
          <p className="hint">
            If the server file changed, recovery stops and retains your local
            copy. Open the server version separately to compare. This recovery
            applies within the running app; restarting the app does not restore
            watched edits yet.
          </p>
          {problem && (
            <p role="alert" className="file-error">
              {problem}
            </p>
          )}
          <div className="dialog-actions">
            <button
              className="primary"
              disabled={busy || !connected}
              onClick={() => {
                onAskUploads();
                void run<Edit>(
                  live.id,
                  "edit_resume",
                  {
                    editId: recovering.id,
                  },
                  false,
                ).then((result) => {
                  if (!result) return;
                  setEdits((a) => [...a, result]);
                  setRecoverable((a) => a.filter((e) => e.id !== result.id));
                  setRecovering(null);
                  onEditOpened();
                });
              }}
            >
              Verify and resume
            </button>
            <button
              disabled={busy}
              onClick={() =>
                void invoke("edit_reopen", { editId: recovering.id }).catch(
                  error,
                )
              }
            >
              Open local copy
            </button>
            <button
              disabled={busy}
              onClick={() => {
                setNotNow((a) => [...a, recovering.id]);
                setRecovering(null);
              }}
            >
              Not now
            </button>
          </div>
        </Modal>
      )}
      {bulkDelete && (
        <Modal
          title="Delete selected items"
          onClose={busy ? undefined : () => setBulkDelete(null)}
        >
          <p>
            Permanently delete these {bulkDelete.length} items? Directories must
            be empty; deletion is never recursive. Processing stops on the first
            error.
          </p>
          <ul className="bulk-items">
            {bulkDelete
              .filter((t) => selected.includes(t.entry.name))
              .map((t) => (
                <li key={t.path}>{displayName(t.path)}</li>
              ))}
          </ul>
          {problem && (
            <p className="file-error" role="alert">
              {problem}
            </p>
          )}
          <div className="dialog-actions">
            {busy && (
              <button
                onClick={() => {
                  stopBatch.current = true;
                  void invoke("file_cancel", { id: bulkDelete[0]?.id }).catch(
                    error,
                  );
                }}
              >
                Cancel remaining
              </button>
            )}
            <button disabled={busy} onClick={() => setBulkDelete(null)}>
              Cancel
            </button>
            <button
              className="danger"
              disabled={busy || !connected}
              onClick={() => void deleteSelected()}
            >
              Delete selected permanently
            </button>
          </div>
        </Modal>
      )}
      {fileMenu && (
        <ContextMenu
          compact
          menu={{
            x: fileMenu.x,
            y: fileMenu.y,
            actions: !fileMenu.target
              ? [
                  {
                    label: "New file",
                    disabled: busy || !connected || !fileMenu.directory,
                    run: () => {
                      setName("");
                      setProblem("");
                      setNewFile({
                        id: fileMenu.id,
                        directory: fileMenu.directory,
                        kind: "file",
                      });
                    },
                  },
                  {
                    label: "New directory",
                    disabled: busy || !connected || !fileMenu.directory,
                    run: () => {
                      setName("");
                      setProblem("");
                      setNewFile({
                        id: fileMenu.id,
                        directory: fileMenu.directory,
                        kind: "directory",
                      });
                    },
                  },
                  ...(onFolderSizes
                    ? [
                        {
                          label: "Show folder sizes",
                          disabled:
                            !connected || !fileMenu.directory.startsWith("/"),
                          run: () => onFolderSizes(fileMenu.directory),
                        },
                      ]
                    : []),
                ]
              : selected.length > 1 &&
                  selected.includes(fileMenu.target.entry.name)
                ? [
                    {
                      label: `Download ${selected.length} files`,
                      disabled: !canDownload,
                      run: () => void downloadSelected(),
                    },
                    {
                      label: `Delete ${selected.length} items`,
                      disabled: busy || !connected,
                      run: requestDelete,
                    },
                    {
                      label: "Copy file paths",
                      run: () => copy(targets.map((t) => t.path).join("\n")),
                    },
                  ]
                : [
                    {
                      label: "Open",
                      disabled: busy || !connected,
                      run: () => void activate(fileMenu.target!),
                    },
                    ...(!fileMenu.target!.entry.directory
                      ? [
                          {
                            label: "Open with",
                            disabled: busy || !connected,
                            run: () =>
                              void resolved(fileMenu.target!).then((t) => {
                                if (t && !t.entry.directory)
                                  return open(t, true);
                              }),
                          },
                          {
                            label: "Open in nano",
                            disabled:
                              busy ||
                              !connected ||
                              windowsDrivePath(fileMenu.target!.path),
                            run: () => void nano(fileMenu.target!),
                          },
                          {
                            label: "Download",
                            disabled: busy || !connected,
                            run: () =>
                              void transfer(
                                fileMenu.target!.id,
                                fileMenu.target!.path,
                                false,
                              ),
                          },
                        ]
                      : fileMenu.target!.entry.symlink
                        ? []
                        : [
                            {
                              label: "Download",
                              disabled: busy || !connected,
                              run: () =>
                                void downloadSelected([fileMenu.target!]),
                            },
                            ...(onFolderSizes
                              ? [
                                  {
                                    label: "Show folder sizes",
                                    disabled:
                                      !connected ||
                                      !fileMenu.target!.path.startsWith("/"),
                                    run: () =>
                                      onFolderSizes(fileMenu.target!.path),
                                  },
                                ]
                              : []),
                          ]),
                    {
                      label: "Open containing directory",
                      disabled: busy || !connected,
                      run: () => void containingDirectory(fileMenu.target!),
                    },
                    {
                      label: "Rename",
                      disabled: busy || !connected,
                      run: () => {
                        setName(fileMenu.target!.entry.name);
                        setDialog({ kind: "rename", target: fileMenu.target! });
                      },
                    },
                    {
                      label: "Delete",
                      disabled: busy || !connected,
                      run: () =>
                        setDialog({ kind: "delete", target: fileMenu.target! }),
                    },
                    {
                      label: "Copy file path",
                      run: () => copy(fileMenu.target!.path),
                    },
                    {
                      label: "Copy path to terminal",
                      disabled: busy || !connected,
                      run: () => insert(fileMenu.target!),
                    },
                    {
                      label: "Properties",
                      disabled: busy || !connected,
                      run: () => void inspect(fileMenu.target!, "properties"),
                    },
                    {
                      label: "Permissions",
                      disabled:
                        busy || !connected || fileMenu.target!.entry.symlink,
                      run: () => void inspect(fileMenu.target!, "permissions"),
                    },
                  ],
          }}
          onClose={() => setFileMenu(null)}
        />
      )}
      {dialog && (
        <Modal
          title={
            (
              {
                rename: "Rename",
                delete: "Delete permanently",
                permissions: "Permissions",
                properties: "Properties",
              } as const
            )[dialog.kind]
          }
          onClose={busy ? undefined : () => setDialog(null)}
          className={
            dialog.kind === "permissions" ? "permissions-dialog" : undefined
          }
        >
          <p className="remote-target">{displayName(dialog.target.path)}</p>
          {dialog.kind === "rename" && (
            <>
              <label>
                New name
                <input
                  autoFocus
                  value={name}
                  maxLength={255}
                  onChange={(e) => setName(e.target.value)}
                />
              </label>
              <p className="hint">
                An existing destination is never overwritten.
              </p>
              <div className="dialog-actions">
                <button
                  className="primary"
                  disabled={busy || !name || name === dialog.target.entry.name}
                  onClick={() => void action(dialog, "rename", { name })}
                >
                  Rename
                </button>
              </div>
            </>
          )}
          {dialog.kind === "delete" && (
            <>
              <p>
                This permanently removes the selected remote item. Directories
                must be empty; deletion is never recursive.
              </p>
              <div className="dialog-actions">
                <button disabled={busy} onClick={() => setDialog(null)}>
                  Cancel
                </button>
                <button
                  className="danger"
                  disabled={busy}
                  onClick={() => void action(dialog, "delete")}
                >
                  Delete permanently
                </button>
              </div>
            </>
          )}
          {dialog.kind === "properties" && dialog.properties && (
            <dl className="properties">
              <dt>Size</dt>
              <dd>{dialog.properties.size ?? "Unknown"} bytes</dd>
              <dt>Mode</dt>
              <dd>{dialog.properties.permissions?.toString(8) ?? "Unknown"}</dd>
              <dt>
                {dialog.properties.windows_permissions
                  ? "Mapped owner / group"
                  : "Owner / group"}
              </dt>
              <dd>
                {dialog.properties.uid ?? "Unknown"} /{" "}
                {dialog.properties.gid ?? "Unknown"}
              </dd>
              <dt>Modified</dt>
              <dd>
                {dialog.properties.mtime
                  ? new Date(dialog.properties.mtime * 1000).toLocaleString()
                  : "Unknown"}
              </dd>
              <dt>Type</dt>
              <dd>
                {dialog.properties.symlink
                  ? "Symbolic link"
                  : dialog.properties.directory
                    ? "Directory"
                    : "File"}
              </dd>
            </dl>
          )}
          {dialog.kind === "permissions" && (
            <>
              {dialog.properties?.windows_permissions ? (
                <WindowsPermissions
                  directory={dialog.properties.directory}
                  mode={mode}
                  disabled={busy}
                  onChange={setMode}
                />
              ) : (
                <PosixPermissions
                  mode={mode}
                  directory={!!dialog.properties?.directory}
                  recursive={recursive}
                  disabled={busy}
                  onMode={setMode}
                  onRecursive={setRecursive}
                />
              )}
              <p className="hint">
                {recursive
                  ? "Files and folders inside get the same mode. Symlinks and special files are skipped, never followed."
                  : "Applies to this item only. Entries reported as symlinks are refused."}
              </p>
              <div className="dialog-actions">
                <button
                  className="primary"
                  disabled={
                    busy ||
                    !/^[0-7]{3,4}$/.test(mode) ||
                    (dialog.properties?.windows_permissions &&
                      dialog.properties.directory)
                  }
                  onClick={() =>
                    void action(dialog, "permissions", {
                      mode: parseInt(mode, 8),
                      recursive: recursive && !!dialog.properties?.directory,
                    })
                  }
                >
                  Apply permissions
                </button>
              </div>
            </>
          )}
          {busy && (
            <div className="file-busy">
              Working…{" "}
              <button
                onClick={() =>
                  void invoke("file_cancel", { id: dialog.target.id }).catch(
                    error,
                  )
                }
              >
                Cancel operation
              </button>
            </div>
          )}
          {problem && (
            <p role="alert" className="file-error">
              {problem}
            </p>
          )}
        </Modal>
      )}
    </div>
  );
}
