import { useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { X, Plug, Unplug } from "lucide-react";
import type { Live, Profile } from "./types";

// Side panel beside an RDP pane. Files, PowerShell and power actions run over
// an SSH session to the same host and account (OpenSSH on the remote computer);
// RDP itself offers no such channel. Without OpenSSH the panel only explains.
export function RdpTools({
  profile,
  rdpId,
  live,
  files,
  terminal,
  onConnect,
  onDisconnect,
  onClose,
}: {
  profile: Profile;
  /** Live RDP session in the pane, if connected. */
  rdpId?: number;
  live?: Live;
  files: ReactNode;
  terminal: ReactNode;
  onConnect: () => Promise<void>;
  onDisconnect: () => void;
  onClose: () => void;
}) {
  const [ssh, setSsh] = useState<"checking" | "available" | "missing">(
    "checking",
  );
  const [tab, setTab] = useState<"files" | "shell" | "power">("files");
  const [busy, setBusy] = useState(false);
  const [probeError, setProbeError] = useState("");
  const [confirm, setConfirm] = useState<"restart" | "shutdown" | null>(null);
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
            OpenSSH found on {profile.host}. The tools sign in over SSH with the
            same account and saved password as the remote desktop.
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
            <span>{live.meta?.status ?? "Connecting…"}</span>
            <button
              className="icon"
              title="Disconnect tools"
              aria-label="Disconnect tools"
              onClick={onDisconnect}
            >
              <Unplug size={14} />
            </button>
          </div>
          <nav className="rdp-tools-tabs" role="tablist">
            {(
              [
                ["files", "Files"],
                ["shell", "PowerShell"],
                ["power", "Power"],
              ] as const
            ).map(([key, label]) => (
              <button
                key={key}
                role="tab"
                aria-selected={tab === key}
                className={tab === key ? "active" : ""}
                onClick={() => setTab(key)}
              >
                {label}
              </button>
            ))}
          </nav>
          <div className="rdp-tools-body" hidden={tab !== "files"}>
            {files}
          </div>
          {/* The terminal stays mounted: it also polls the session. */}
          <div
            className="rdp-tools-body rdp-tools-shell"
            style={{ display: tab === "shell" ? "flex" : "none" }}
          >
            {terminal}
          </div>
          {tab === "power" && (
            <div className="rdp-tools-body rdp-tools-power">
              <p className="hint">
                Runs on {profile.host} through the tools session. Unsaved work
                in the remote desktop is lost.
              </p>
              <button
                disabled={!connected}
                onClick={() => setConfirm("restart")}
              >
                Restart computer
              </button>
              <button
                disabled={!connected}
                onClick={() => setConfirm("shutdown")}
              >
                Shut down computer
              </button>
              {confirm && (
                <div className="rdp-tools-confirm" role="alertdialog">
                  <p>
                    {confirm === "restart" ? "Restart" : "Shut down"}{" "}
                    {profile.host} now?
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
          )}
        </>
      )}
    </aside>
  );
}
