import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  TriangleAlert,
  ChevronDown,
  ChevronRight,
  Folder,
  FolderOpen,
  KeyRound,
  File as FileIcon,
  RefreshCw,
  Search,
  X,
} from "lucide-react";
import type { Live, Profile } from "./types";
import {
  CATEGORIES,
  COMMANDS,
  asRoot,
  rootShell,
  PARAM_LABELS,
  childPath,
  commandText,
  formatKib,
  matches,
  parentPath,
  unavailable,
  validate,
  type Probe,
  type SshCommand,
} from "./ssh-commands";

type FolderSizes = {
  path: string;
  total_kib: number | null;
  filesystem: {
    name: string;
    total_kib: number;
    used_kib: number;
    available_kib: number;
    mount: string;
  } | null;
  entries: { name: string; kib: number; directory: boolean }[];
  incomplete: boolean;
  truncated: boolean;
  hidden: number;
  skipped: number;
};

export type StorageRequest = { path: string; revision: number };

// Side panel beside an SSH terminal: curated commands typed into this session's
// terminal (never broadcast, never on reconnect) and a folder-size view that
// runs `du` on a separate exec channel.
export function SshTools({
  profile,
  live,
  storageRequest,
  onClose,
  onFocusTerminal,
  onOpenInFiles,
  error,
}: {
  profile: Profile;
  live?: Live;
  storageRequest?: StorageRequest;
  onClose: () => void;
  onFocusTerminal: () => void;
  onOpenInFiles: (path: string) => void;
  error: (e: unknown) => void;
}) {
  const connected = !!live?.meta?.connected && !live.meta.finished;
  const [tab, setTab] = useState<"commands" | "storage">(
    storageRequest ? "storage" : "commands",
  );
  const [probe, setProbe] = useState<Probe | undefined>();
  const [probeState, setProbeState] = useState<
    "idle" | "checking" | "done" | "failed"
  >("idle");
  const liveId = live?.id;
  // Set after the Root shell button typed sudo -i / su -. The panel cannot see
  // whether the login succeeded or the shell was left, so the user can reset it.
  const [rootShellOpen, setRootShellOpen] = useState(false);
  // One probe per connection incarnation, only while the panel is open.
  useEffect(() => {
    if (!connected || liveId === undefined) return;
    let current = true;
    setRootShellOpen(false);
    setProbe(undefined);
    setProbeState("checking");
    void invoke<Probe>("ssh_tools_probe", { id: liveId })
      .then((p) => {
        if (!current) return;
        setProbe(p);
        setProbeState("done");
      })
      .catch(() => current && setProbeState("failed"));
    return () => {
      current = false;
    };
  }, [liveId, connected]);
  const linux = !probe || probe.os === "Linux";
  const [storagePath, setStoragePath] = useState<string | null>(
    storageRequest?.path ?? null,
  );
  const seenRequest = useRef(storageRequest?.revision ?? 0);
  useEffect(() => {
    if (!storageRequest || storageRequest.revision === seenRequest.current)
      return;
    seenRequest.current = storageRequest.revision;
    setStoragePath(storageRequest.path);
    setTab("storage");
  }, [storageRequest]);
  const openStorage = () => {
    const cwd = live?.meta?.path;
    setStoragePath((p) => p ?? (cwd?.startsWith("/") ? cwd : "/"));
    setTab("storage");
  };
  return (
    <aside className="rdp-tools ssh-tools" aria-label="SSH tools">
      <header>
        <strong>Remote tools</strong>
        <button
          className="icon"
          title="Close remote tools"
          aria-label="Close remote tools"
          onClick={onClose}
        >
          <X size={14} />
        </button>
      </header>
      <nav className="rdp-tools-tabs" role="tablist">
        <button
          role="tab"
          aria-selected={tab === "commands"}
          className={tab === "commands" ? "active" : ""}
          onClick={() => setTab("commands")}
        >
          Commands
        </button>
        <button
          role="tab"
          aria-selected={tab === "storage"}
          className={tab === "storage" ? "active" : ""}
          onClick={openStorage}
        >
          Storage
        </button>
      </nav>
      {!connected && (
        <p className="hint">Connect the session to use the remote tools.</p>
      )}
      {connected && probeState === "failed" && (
        <p className="hint" data-ssh-tools="probe-failed">
          Could not detect installed tools; all commands stay enabled.
        </p>
      )}
      {connected && !linux && (
        <p className="hint" data-ssh-tools="not-linux">
          {profile.host} reports {probe?.os || "an unknown system"}. These tools
          are for Linux hosts.
        </p>
      )}
      {tab === "commands" ? (
        <Commands
          live={live}
          probe={probe}
          rootShellOpen={rootShellOpen}
          onRootShell={setRootShellOpen}
          enabled={connected && linux && probeState !== "checking"}
          onStorage={openStorage}
          onFocusTerminal={onFocusTerminal}
          error={error}
        />
      ) : (
        connected &&
        linux &&
        probeState !== "checking" &&
        live &&
        storagePath && (
          <Storage
            key={live.id}
            id={live.id}
            path={storagePath}
            onPath={setStoragePath}
            onOpenInFiles={onOpenInFiles}
          />
        )
      )}
    </aside>
  );
}

function Commands({
  live,
  probe: detected,
  rootShellOpen,
  onRootShell,
  enabled,
  onStorage,
  onFocusTerminal,
  error,
}: {
  live?: Live;
  probe?: Probe;
  rootShellOpen: boolean;
  onRootShell: (open: boolean) => void;
  enabled: boolean;
  onStorage: () => void;
  onFocusTerminal: () => void;
  error: (e: unknown) => void;
}) {
  // Inside a root shell commands are built as for root (no sudo).
  const probe = rootShellOpen ? asRoot(detected) : detected;
  const login = rootShell(detected);
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState<string[]>([]);
  const [pending, setPending] = useState<{
    command: SshCommand;
    value: string;
    action?: string;
  } | null>(null);
  const visible = useMemo(
    () => COMMANDS.filter((c) => matches(c, query)),
    [query],
  );
  const send = (text: string) => {
    if (!live) return;
    // Only this session: commands are never mirrored to broadcast targets.
    void invoke("session_input", {
      id: live.id,
      bytes: [...new TextEncoder().encode(text + "\r")],
    })
      .then(onFocusTerminal)
      .catch(error);
  };
  const choose = (command: SshCommand) => {
    if (command.storage) return onStorage();
    if (command.param || command.danger)
      return setPending({
        command,
        value: "",
        action: command.actions?.[0].value,
      });
    send(commandText(command, { probe }));
  };
  let preview = "",
    problem: string | null = null;
  if (pending) {
    if (pending.command.param)
      problem = pending.value
        ? validate(pending.command.param, pending.value)
        : "";
    if (problem === null)
      preview = commandText(pending.command, {
        probe,
        value: pending.value,
        action: pending.action,
      });
  }
  const confirm = () => {
    if (!pending || problem !== null) return;
    send(preview);
    setPending(null);
  };
  return (
    <div className="rdp-tools-body ssh-commands">
      <label className="ssh-search">
        <Search size={13} />
        <input
          aria-label="Search commands"
          placeholder="Search commands"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </label>
      <p className="hint">
        Commands are typed into this terminal. Close editors or pagers first.
      </p>
      {rootShellOpen ? (
        <div className="ssh-root" data-ssh-root="open">
          <KeyRound size={13} />
          <span>Commands assume the root shell; sudo is left out.</span>
          <button
            title="Use after leaving the root shell or if the login failed"
            onClick={() => onRootShell(false)}
          >
            Reset
          </button>
        </div>
      ) : (
        <button
          className="ssh-root-button"
          disabled={!enabled || !login}
          title={
            login
              ? `Types ${login}; the terminal asks for the password`
              : "This account already is root"
          }
          onClick={() => {
            if (!login) return;
            send(login);
            onRootShell(true);
          }}
        >
          <KeyRound size={13} />
          Root shell{login ? ` (${login})` : ""}
        </button>
      )}
      {pending && (
        <div
          className="rdp-tools-confirm"
          role={pending.command.danger ? "alertdialog" : "dialog"}
          aria-label={pending.command.label}
        >
          <strong>{pending.command.label}</strong>
          {pending.command.param && (
            <input
              autoFocus
              aria-label={PARAM_LABELS[pending.command.param]}
              placeholder={PARAM_LABELS[pending.command.param]}
              value={pending.value}
              onChange={(e) =>
                setPending({ ...pending, value: e.target.value })
              }
              onKeyDown={(e) => {
                if (e.key === "Enter") confirm();
                if (e.key === "Escape") setPending(null);
              }}
            />
          )}
          {pending.command.actions && (
            <div className="ssh-actions" role="radiogroup">
              {pending.command.actions.map((a) => (
                <button
                  key={a.value}
                  role="radio"
                  aria-checked={pending.action === a.value}
                  className={pending.action === a.value ? "active" : ""}
                  onClick={() => setPending({ ...pending, action: a.value })}
                >
                  {a.label}
                </button>
              ))}
            </div>
          )}
          {problem && <p className="hint">{problem}</p>}
          {preview && <code className="ssh-preview">{preview}</code>}
          {pending.command.danger && (
            <p className="hint">This changes the remote system.</p>
          )}
          <div className="dialog-actions">
            <button onClick={() => setPending(null)}>Cancel</button>
            <button
              className={pending.command.danger ? "danger" : "primary"}
              disabled={!enabled || problem !== null}
              onClick={confirm}
            >
              Run
            </button>
          </div>
        </div>
      )}
      <div className="ssh-command-list">
        {CATEGORIES.map((category) => {
          const items = visible.filter((c) => c.category === category);
          if (!items.length) return null;
          const closed = collapsed.includes(category) && !query;
          return (
            <section key={category}>
              <button
                className="ssh-category"
                aria-expanded={!closed}
                onClick={() =>
                  setCollapsed((c) =>
                    c.includes(category)
                      ? c.filter((v) => v !== category)
                      : [...c, category],
                  )
                }
              >
                {closed ? (
                  <ChevronRight size={13} />
                ) : (
                  <ChevronDown size={13} />
                )}
                {category}
              </button>
              {!closed &&
                items.map((command) => {
                  const reason = unavailable(command, probe);
                  const text = command.storage
                    ? "Opens the folder-size view"
                    : command.build({ probe, value: "<value>" });
                  return (
                    <button
                      key={command.id}
                      className={`ssh-command ${command.danger ? "risky" : ""}`}
                      disabled={!enabled || !!reason}
                      title={`${command.label}
${reason ?? text}`}
                      data-command={command.id}
                      onClick={() => choose(command)}
                    >
                      <span>{command.label}</span>
                      {command.danger && (
                        <span
                          className="ssh-badge"
                          aria-label="Changes the system"
                        >
                          <TriangleAlert size={12} aria-hidden="true" />
                        </span>
                      )}
                    </button>
                  );
                })}
            </section>
          );
        })}
        {!visible.length && <p className="hint">No matching commands.</p>}
      </div>
    </div>
  );
}

function Storage({
  id,
  path,
  onPath,
  onOpenInFiles,
}: {
  id: number;
  path: string;
  onPath: (path: string) => void;
  onOpenInFiles: (path: string) => void;
}) {
  const [result, setResult] = useState<FolderSizes | null>(null);
  const [problem, setProblem] = useState("");
  const [loading, setLoading] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [reload, setReload] = useState(0);
  const request = useRef(0);
  useEffect(() => {
    const mine = ++request.current;
    setLoading(true);
    setProblem("");
    setElapsed(0);
    const started = Date.now();
    const timer = setInterval(
      () => setElapsed(Math.floor((Date.now() - started) / 1000)),
      1000,
    );
    void invoke<FolderSizes>("ssh_folder_sizes", { id, path })
      .then((r) => {
        if (mine === request.current) setResult(r);
      })
      .catch((e) => {
        if (mine !== request.current) return;
        setResult(null);
        setProblem(String(e));
      })
      .finally(() => {
        clearInterval(timer);
        if (mine === request.current) setLoading(false);
      });
    return () => clearInterval(timer);
  }, [id, path, reload]);
  // Leaving the view stops a running listing on the server.
  useEffect(
    () => () => {
      request.current++;
      void invoke("ssh_folder_sizes_cancel", { id }).catch(() => {});
    },
    [id],
  );
  const cancel = () => {
    request.current++;
    setLoading(false);
    setProblem("Cancelled");
    void invoke("ssh_folder_sizes_cancel", { id }).catch(() => {});
  };
  const shown = result?.path === path ? result : null;
  const largest = Math.max(1, ...(shown?.entries.map((e) => e.kib) ?? [1]));
  const fs = shown?.filesystem;
  const segments = path.split("/").filter(Boolean);
  return (
    <div className="rdp-tools-body ssh-storage" data-storage-path={path}>
      <nav className="ssh-crumbs" aria-label="Folder path">
        <button onClick={() => onPath("/")}>/</button>
        {segments.map((s, i) => (
          <button
            key={i}
            onClick={() => onPath("/" + segments.slice(0, i + 1).join("/"))}
          >
            {s}
          </button>
        ))}
      </nav>
      <div className="ssh-storage-actions">
        <button
          disabled={path === "/"}
          onClick={() => onPath(parentPath(path))}
        >
          Up
        </button>
        <button
          className="icon"
          title="Refresh"
          aria-label="Refresh folder sizes"
          disabled={loading}
          onClick={() => setReload((n) => n + 1)}
        >
          <RefreshCw size={13} />
        </button>
        <button onClick={() => onOpenInFiles(path)}>Open in Files</button>
      </div>
      {fs && (
        <div className="ssh-df" title={`${fs.name} mounted on ${fs.mount}`}>
          <div className="ssh-bar">
            <span
              style={{
                width: `${Math.min(100, (fs.used_kib / Math.max(1, fs.total_kib)) * 100)}%`,
              }}
            />
          </div>
          <small>
            {fs.mount}: {formatKib(fs.available_kib)} free of{" "}
            {formatKib(fs.total_kib)}
          </small>
        </div>
      )}
      {loading && (
        <div className="file-busy" data-storage="loading">
          <span>Measuring… {elapsed}s</span>
          <button onClick={cancel}>Cancel</button>
        </div>
      )}
      {problem && !loading && <p className="hint">{problem}</p>}
      {shown && !loading && (
        <>
          <small data-storage="total">
            {shown.total_kib !== null
              ? `${formatKib(shown.total_kib)} in this folder`
              : "Total unavailable"}
          </small>
          {shown.incomplete && (
            <p className="hint">
              Some folders could not be read; sizes may be incomplete.
            </p>
          )}
          {(shown.truncated || shown.hidden > 0) && (
            <p className="hint">
              The list was shortened
              {shown.hidden > 0
                ? ` (${shown.hidden} smaller entries hidden)`
                : ""}
              .
            </p>
          )}
          <ul className="ssh-sizes" aria-label="Folder sizes">
            {shown.entries.map((entry) => (
              <li key={entry.name}>
                <button
                  disabled={!entry.directory}
                  title={childPath(path, entry.name)}
                  onClick={() => onPath(childPath(path, entry.name))}
                >
                  {entry.directory ? (
                    <Folder size={13} />
                  ) : (
                    <FileIcon size={13} />
                  )}
                  <span className="ssh-size-name">{entry.name}</span>
                  <span className="ssh-size-value">{formatKib(entry.kib)}</span>
                  <span className="ssh-bar">
                    <span
                      style={{ width: `${(entry.kib / largest) * 100}%` }}
                    />
                  </span>
                </button>
              </li>
            ))}
            {!shown.entries.length && (
              <li className="hint">
                <FolderOpen size={13} /> Empty folder
              </li>
            )}
          </ul>
        </>
      )}
    </div>
  );
}
