import { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Ellipsis } from "lucide-react";
import { ContextMenu, type MenuState } from "./ContextMenu";
import { Modal } from "./Modal";
import { protocolLabel, type Preferences, type Profile } from "./types";

type Preview = {
  token: number;
  profiles: Profile[];
  format: string;
  skipped: { line: number; reason: string }[];
  duplicates: number;
};
export function SessionTransfer({
  enabled,
  count,
  onImported,
  error,
}: {
  enabled: boolean;
  count: number;
  onImported: (p: Preferences) => void;
  error: (e: unknown) => void;
}) {
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [mode, setMode] = useState<"export" | "import" | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [busy, setBusy] = useState(false);
  const working = useRef(false);
  const run = async (action: () => Promise<void>) => {
    if (working.current) return;
    working.current = true;
    setBusy(true);
    try {
      await action();
    } catch (e) {
      error(e);
    } finally {
      working.current = false;
      setBusy(false);
    }
  };
  const close = () => {
    if (working.current) return;
    if (preview)
      void invoke("profiles_import_cancel", { token: preview.token }).catch(
        error,
      );
    setPreview(null);
    setMode(null);
  };
  return (
    <>
      <button
        className="icon session-transfer-button"
        title="Session import and export"
        aria-label="Session import and export"
        disabled={busy}
        onClick={(e) => {
          e.stopPropagation();
          const r = e.currentTarget.getBoundingClientRect();
          setMenu({
            x: r.left,
            y: r.bottom,
            actions: [
              {
                label: "Import sessions",
                disabled: !enabled,
                run: () =>
                  void run(async () => {
                    const incoming = await invoke<Preview | null>(
                      "profiles_import_preview",
                    );
                    if (incoming) {
                      setPreview(incoming);
                      setMode("import");
                    }
                  }),
              },
              {
                label: "Export sessions",
                disabled: count === 0,
                run: () => setMode("export"),
              },
            ],
          });
        }}
      >
        <Ellipsis size={15} />
      </button>
      {menu && <ContextMenu menu={menu} onClose={() => setMenu(null)} />}
      {mode && (
        <Modal
          title={mode === "import" ? "Import sessions" : "Export sessions"}
          onClose={busy ? undefined : close}
        >
          {mode === "export" ? (
            <>
              <p>Export {count} saved sessions to a JSON file.</p>
              <p>
                Includes server addresses, usernames, terminal settings and
                local key/config paths. Keep the file private. Passwords,
                private-key contents and trusted host keys are excluded.
              </p>
            </>
          ) : (
            <>
              <p>
                Add {preview?.profiles.length} sessions. Existing sessions stay
                unchanged; existing sessions and duplicates are skipped. Nothing
                connects automatically.
              </p>
              {!!preview?.duplicates && (
                <p aria-label="Duplicate import entries">
                  {preview.duplicates} duplicate sessions skipped.
                </p>
              )}
              <p>
                Imported sessions have automatic logging turned off. Local
                key/config paths may need updating on this computer. Host keys
                must be verified when connecting.
              </p>
              {preview?.format === "MobaXterm" && (
                <p>
                  MobaXterm metadata only. Nested folders become groups joined
                  with /. Invalid or oversized groups are listed as skipped.
                  Commands, credentials, keys, gateways, proxies, forwarding and
                  display settings are discarded. SSH uses the Windows agent;
                  VNC requires validated TLS. Review settings before connecting.
                </p>
              )}
              <div
                className="session-import-list"
                aria-label="Sessions to import"
              >
                {preview?.profiles.map((p) => (
                  <details key={p.id}>
                    <summary>
                      {p.name}{" "}
                      <small>
                        {protocolLabel(p.protocol)} · {p.host || "Serial"}
                        {p.group ? ` · Group: ${p.group}` : " · Ungrouped"}
                      </small>
                    </summary>
                    <p>
                      {p.username ? `${p.username}@` : ""}
                      {p.host}:{p.port}
                    </p>
                    {p.protocol === "Serial" && (
                      <p>
                        {p.serial.baud_rate} baud · {p.serial.data_bits} data
                        bits · {p.serial.parity} parity · {p.serial.stop_bits}{" "}
                        stop bits · {p.serial.flow_control} flow control
                      </p>
                    )}
                    {p.protocol === "Ssh" && (
                      <p>
                        {p.ssh.auth === "WindowsAgent"
                          ? "Windows SSH agent"
                          : "Private key file"}
                        {p.ssh.key_file && <code>{p.ssh.key_file}</code>}
                        {p.ssh.config_file && <code>{p.ssh.config_file}</code>}
                        {p.ssh.config_host && (
                          <code>SSH config host: {p.ssh.config_host}</code>
                        )}
                      </p>
                    )}
                  </details>
                ))}
                {!!preview?.skipped.length && (
                  <section aria-label="Skipped import entries">
                    <h4>{preview.skipped.length} entries skipped</h4>
                    {preview.skipped.map((s) => (
                      <p key={s.line}>
                        Line {s.line}: {s.reason}
                      </p>
                    ))}
                  </section>
                )}
              </div>
            </>
          )}
          <div className="dialog-actions">
            <button disabled={busy} onClick={close}>
              Cancel
            </button>
            <button
              className="primary"
              disabled={
                busy || (mode === "import" && !preview?.profiles.length)
              }
              onClick={() =>
                void run(async () => {
                  if (mode === "export") {
                    if (await invoke<boolean>("profiles_export")) setMode(null);
                  } else if (preview) {
                    onImported(
                      await invoke<Preferences>("profiles_import_apply", {
                        token: preview.token,
                      }),
                    );
                    setPreview(null);
                    setMode(null);
                  }
                })
              }
            >
              {busy
                ? "Working…"
                : mode === "export"
                  ? "Export file"
                  : "Import sessions"}
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}
