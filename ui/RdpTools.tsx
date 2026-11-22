import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { X, Plug, Unplug, ExternalLink } from "lucide-react";
import type { CommandCategory, Live, Profile } from "./types";
import { Commands } from "./SshTools";
import { WINDOWS_CATALOG } from "./windows-commands";

// Side panel beside an RDP pane. Files, PowerShell and power actions run over
// an SSH session to the same host and account (OpenSSH on the remote computer);
// RDP itself offers no such channel. Without OpenSSH the panel only explains.

const PANES = ["files", "shell", "commands", "power"] as const;
type Pane = (typeof PANES)[number];
const PANE_LABELS: Record<Pane, string> = {
  files: "Files",
  shell: "PowerShell",
  commands: "Commands",
  power: "Power",
};
const WEIGHTS_KEY = "yuuterm.rdp-tools-panes.v1";
function readWeights(): Record<Pane, number> {
  let raw: Record<string, unknown> = {};
  try {
    raw = JSON.parse(localStorage.getItem(WEIGHTS_KEY) ?? "{}") ?? {};
  } catch {
    raw = {};
  }
  const weight = (value: unknown) =>
    typeof value === "number" && Number.isFinite(value) && value > 0
      ? Math.min(10, value)
      : 1;
  return {
    files: weight(raw.files),
    shell: weight(raw.shell),
    commands: weight(raw.commands),
    power: weight(raw.power),
  };
}

export function RdpTools({
  profile,
  rdpId,
  live,
  files,
  terminal,
  statistics,
  layout,
  shellPopped = false,
  error,
  onConnect,
  onDisconnect,
  onPopOut,
  onClose,
}: {
  profile: Profile;
  /** Live RDP session in the pane, if connected. */
  rdpId?: number;
  live?: Live;
  files: ReactNode;
  terminal: ReactNode;
  statistics?: ReactNode;
  layout?: CommandCategory[];
  shellPopped?: boolean;
  error: (e: unknown) => void;
  onConnect: () => Promise<void>;
  onDisconnect: () => void;
  /** Moves the PowerShell terminal into its own window. */
  onPopOut: () => void;
  onClose: () => void;
}) {
  const [ssh, setSsh] = useState<"checking" | "available" | "missing">(
    "checking",
  );
  const [open, setOpen] = useState<Pane[]>(["files"]);
  const [weights, setWeights] = useState(readWeights);
  const panes = useRef<Partial<Record<Pane, HTMLDivElement | null>>>({});
  const [busy, setBusy] = useState(false);
  const [probeError, setProbeError] = useState("");
  const [confirm, setConfirm] = useState<"restart" | "shutdown" | null>(null);
  useEffect(() => {
    localStorage.setItem(WEIGHTS_KEY, JSON.stringify(weights));
  }, [weights]);
  const toggle = (pane: Pane) =>
    setOpen((o) =>
      o.includes(pane) ? o.filter((p) => p !== pane) : [...o, pane],
    );
  const visible = PANES.filter((p) => open.includes(p));
  const share = (above: Pane, below: Pane, fraction: number) =>
    setWeights((w) => {
      const total = w[above] + w[below];
      const f = Math.min(0.9, Math.max(0.1, fraction));
      return { ...w, [above]: f * total, [below]: (1 - f) * total };
    });
  const splitter = (above: Pane, below: Pane) => (
    <div
      className="rdp-tools-splitter"
      role="separator"
      aria-orientation="horizontal"
      aria-label={`Resize ${PANE_LABELS[above]} and ${PANE_LABELS[below]}`}
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
        const f = weights[above] / (weights[above] + weights[below]);
        share(above, below, f + (e.key === "ArrowUp" ? -0.05 : 0.05));
      }}
      onPointerDown={(e) => {
        const top = panes.current[above]?.getBoundingClientRect().top;
        const bottom = panes.current[below]?.getBoundingClientRect().bottom;
        if (top === undefined || bottom === undefined || bottom <= top) return;
        const handle = e.currentTarget;
        handle.setPointerCapture(e.pointerId);
        const follow = (m: PointerEvent) =>
          share(above, below, (m.clientY - top) / (bottom - top));
        const stop = () => {
          handle.removeEventListener("pointermove", follow);
          handle.removeEventListener("pointerup", stop);
          handle.removeEventListener("pointercancel", stop);
        };
        handle.addEventListener("pointermove", follow);
        handle.addEventListener("pointerup", stop);
        handle.addEventListener("pointercancel", stop);
      }}
    />
  );
  useEffect(() => {
    let current = true;
    setSsh("checking");
    void invoke<boolean>("rdp_ssh_available", { profileId: profile.id })
      .then((ok) => current && setSsh(ok ? "available" : "missing"))
      .catch((e) => {
        if (!current) return;
        setProbeError(String(e));
        setSsh("missing");
      });
    return () => {
      current = false;
    };
  }, [profile.id, profile.host]);
  const connected = !!live?.meta?.connected && !live.meta.finished;
  const run = (command: string) =>
    live &&
    invoke("session_input", {
      id: live.id,
      bytes: [...new TextEncoder().encode(command + "\r")],
    });
  const bodies: Record<Exclude<Pane, "shell">, ReactNode> = {
    files: <div className="rdp-tools-body">{files}</div>,
    commands: (
      <Commands
        live={live}
        catalog={WINDOWS_CATALOG}
        layout={layout}
        enabled={connected}
        onFocusTerminal={() =>
          !shellPopped &&
          setOpen((o) => (o.includes("shell") ? o : [...o, "shell"]))
        }
        hint={
          shellPopped
            ? "Commands are typed into the PowerShell window."
            : "Commands are typed into the PowerShell tab."
        }
        error={error}
      />
    ),
    power: (
      <div className="rdp-tools-body rdp-tools-power">
        <p className="hint">
          Runs on {profile.host} through the tools session. Unsaved work in the
          remote desktop is lost.
        </p>
        <button disabled={!connected} onClick={() => setConfirm("restart")}>
          Restart computer
        </button>
        <button disabled={!connected} onClick={() => setConfirm("shutdown")}>
          Shut down computer
        </button>
        {confirm && (
          <div className="rdp-tools-confirm" role="alertdialog">
            <p>
              {confirm === "restart" ? "Restart" : "Shut down"} {profile.host}{" "}
              now?
            </p>
            <div className="dialog-actions">
              <button onClick={() => setConfirm(null)}>Cancel</button>
              <button
                className="primary"
                onClick={() => {
                  void run(
                    confirm === "restart"
                      ? "Restart-Computer -Force"
                      : "Stop-Computer -Force",
                  );
                  setConfirm(null);
                }}
              >
                {confirm === "restart" ? "Restart" : "Shut down"}
              </button>
            </div>
          </div>
        )}
      </div>
    ),
  };
  return (
    <aside className="rdp-tools" aria-label="Remote computer tools">
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
      <button
        disabled={!rdpId}
        title="The local Windows always keeps the physical Ctrl+Alt+Del; this sends it to the remote computer"
        onClick={() =>
          void invoke("rdp_session", {
            id: rdpId,
            action: "ctrl_alt_del",
            bounds: null,
            scale: null,
          })
        }
      >
        Send Ctrl+Alt+Del
      </button>
      {ssh === "checking" && <p className="hint">Checking for OpenSSH…</p>}
      {ssh === "missing" && (
        <p className="hint">
          {profile.host} offers no OpenSSH server on port 22. Files, PowerShell
          and restart need OpenSSH on the remote computer; the remote desktop
          itself keeps working.
          {probeError && <span data-probe-error> ({probeError})</span>}
        </p>
      )}
      {ssh === "available" && !live && (
        <>
          <p className="hint" data-rdp-tools="ssh-available">
            OpenSSH found on {profile.host}. The tools sign in over SSH as{" "}
            {profile.username || "the remote desktop account"}{" "}
            {profile.rdp?.tools_sign_in === "Agent"
              ? "with a key from the Windows SSH agent."
              : profile.rdp?.tools_sign_in === "KeyFile"
                ? "with the private-key file chosen in Edit session."
                : "with the account password; choose a key in Edit session if the computer accepts keys only."}
          </p>
          <button
            className="primary"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              void onConnect().finally(() => setBusy(false));
            }}
          >
            <Plug size={13} />
            Connect tools
          </button>
        </>
      )}
      {live && (
        <>
          <div className="rdp-tools-status">
            <span className={`dot ${connected ? "connected" : ""}`} />
            <span>
              {connected ? "Connected" : (live.meta?.status ?? "Connecting…")}
            </span>
            <button
              className="icon"
              title="Disconnect tools"
              aria-label="Disconnect tools"
              onClick={onDisconnect}
            >
              <Unplug size={14} />
            </button>
          </div>
          <nav className="rdp-tools-tabs">
            {PANES.map((pane) => {
              const shown = open.includes(pane);
              return (
                <button
                  key={pane}
                  aria-pressed={shown}
                  className={shown ? "active" : ""}
                  title={`${shown ? "Close" : "Open"} ${PANE_LABELS[pane]}`}
                  onClick={() => toggle(pane)}
                >
                  {PANE_LABELS[pane]}
                </button>
              );
            })}
          </nav>
          <div className="rdp-tools-split">
            {PANES.map((pane) => {
              const index = visible.indexOf(pane);
              return (
                <Fragment key={pane}>
                  {index > 0 && splitter(visible[index - 1], pane)}
                  {pane === "shell" ? (
                    <div
                      ref={(e) => {
                        panes.current.shell = e;
                      }}
                      className="rdp-tools-pane rdp-tools-shell"
                      data-rdp-pane="shell"
                      style={{
                        display: index >= 0 ? "flex" : "none",
                        flexGrow: weights.shell,
                      }}
                    >
                      <div className="rdp-tools-shell-bar">
                        <button
                          disabled={!connected}
                          title="Open PowerShell in its own window"
                          onClick={() => {
                            onPopOut();
                            setOpen((o) => {
                              const rest = o.filter((p) => p !== "shell");
                              return rest.length ? rest : ["files"];
                            });
                          }}
                        >
                          <ExternalLink size={13} />
                          Open in own window
                        </button>
                      </div>
                      {terminal}
                    </div>
                  ) : (
                    index >= 0 && (
                      <div
                        ref={(e) => {
                          panes.current[pane] = e;
                        }}
                        className="rdp-tools-pane"
                        data-rdp-pane={pane}
                        style={{ flexGrow: weights[pane] }}
                      >
                        {bodies[pane]}
                      </div>
                    )
                  )}
                </Fragment>
              );
            })}
            {!visible.length && <p className="hint">Choose a tool above.</p>}
          </div>
          {connected && statistics}
        </>
      )}
    </aside>
  );
}
