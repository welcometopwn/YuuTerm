import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Modal } from "./Modal";
import type { Live } from "./types";
import { displayName } from "./input-policy";

type Change = {
  edit: { id: number; live_id: number; remote: string; local: string };
  revision: string;
  local_revision?: string;
};
export function EditMonitor({
  enabled,
  lives,
  autoIds,
  onAutoChange,
  error,
}: {
  enabled: boolean;
  lives: Live[];
  autoIds: number[];
  onAutoChange: (ids: number[]) => void;
  error: (e: unknown) => void;
}) {
  const [pending, setPending] = useState<Change | null>(null);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState("");
  const current = useRef({ lives, autoIds, onAutoChange, error });
  current.current = { lives, autoIds, onAutoChange, error };
  const prompt = useRef(pending);
  prompt.current = pending;
  const working = useRef(false);
  const connected = (id: number) =>
    current.current.lives.some(
      (l) => l.id === id && l.meta?.connected && !l.meta.finished,
    );
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    let remoteAt = 0;
    const poll = async () => {
      try {
        const validAuto = current.current.autoIds.filter(connected);
        if (validAuto.length !== current.current.autoIds.length)
          current.current.onAutoChange(validAuto);
        if (prompt.current && !connected(prompt.current.edit.live_id)) {
          setPending(null);
          setProblem("");
        }
        if (!working.current) {
          const changes = await invoke<Change[]>("edit_changes");
          if (!active) return;
          if (working.current) {
            timer = setTimeout(() => void poll(), 1000);
            return;
          }
          const next = changes.find((c) => connected(c.edit.live_id));
          if (prompt.current) {
            const latest = changes.find(
              (c) => c.edit.id === prompt.current?.edit.id,
            );
            if (
              latest &&
              !prompt.current.local_revision &&
              latest.revision !== prompt.current.revision
            ) {
              setPending(latest);
              setProblem("");
            } else if (!latest && !prompt.current.local_revision) {
              setPending(null);
              setProblem("");
            }
          } else if (next && !document.querySelector("dialog[open]")) {
            if (current.current.autoIds.includes(next.edit.live_id)) {
              working.current = true;
              try {
                await invoke("edit_upload", {
                  editId: next.edit.id,
                  revision: next.revision,
                });
              } catch (e) {
                if (!active) return;
                current.current.onAutoChange(
                  current.current.autoIds.filter(
                    (id) => id !== next.edit.live_id,
                  ),
                );
                setProblem(String(e));
                setPending(next);
              } finally {
                working.current = false;
              }
            } else {
              setProblem("");
              setPending(next);
            }
          } else if (
            !next &&
            !document.querySelector("dialog[open]") &&
            Date.now() >= remoteAt
          ) {
            remoteAt = Date.now() + 2000;
            const remote = await invoke<Change[]>("edit_remote_changes");
            if (
              active &&
              !prompt.current &&
              !working.current &&
              !document.querySelector("dialog[open]")
            ) {
              const changed = remote.find((c) => connected(c.edit.live_id));
              if (changed) {
                setProblem("");
                setPending(changed);
              }
            }
          }
        }
      } catch (e) {
        // Explicit recovery/reload can hold the scan lease across SFTP work.
        if (active && String(e) !== "Edit scan already running")
          current.current.error(e);
      }
      if (active) timer = setTimeout(() => void poll(), 1000);
    };
    void poll();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [enabled]);
  const decline = async () => {
    if (!pending || working.current) return;
    working.current = true;
    setBusy(true);
    try {
      await invoke(
        pending.local_revision ? "edit_remote_decide" : "edit_decline",
        {
          editId: pending.edit.id,
          revision: pending.revision,
          localRevision: pending.local_revision,
          reload: false,
        },
      );
    } catch (e) {
      current.current.error(e);
    } finally {
      working.current = false;
      setBusy(false);
      setPending(null);
      setProblem("");
    }
  };
  const upload = async (automatic: boolean) => {
    if (!pending || working.current) return;
    if (!connected(pending.edit.live_id)) {
      setProblem(
        "Original connection is no longer active. Local copy retained.",
      );
      return;
    }
    working.current = true;
    setBusy(true);
    setProblem("");
    try {
      await invoke("edit_upload", {
        editId: pending.edit.id,
        revision: pending.revision,
      });
      if (automatic && connected(pending.edit.live_id))
        current.current.onAutoChange([
          ...new Set([...current.current.autoIds, pending.edit.live_id]),
        ]);
      setPending(null);
    } catch (e) {
      setProblem(String(e));
      current.current.onAutoChange(
        current.current.autoIds.filter((id) => id !== pending.edit.live_id),
      );
    } finally {
      working.current = false;
      setBusy(false);
    }
  };
  const reload = async () => {
    if (!pending || working.current) return;
    working.current = true;
    setBusy(true);
    setProblem("");
    try {
      await invoke("edit_remote_decide", {
        editId: pending.edit.id,
        revision: pending.revision,
        localRevision: pending.local_revision,
        reload: true,
      });
      setPending(null);
    } catch (e) {
      setProblem(String(e));
    } finally {
      working.current = false;
      setBusy(false);
    }
  };
  if (!pending) return null;
  if (pending.local_revision)
    return (
      <Modal
        title="Server file changed"
        onClose={busy ? undefined : () => void decline()}
      >
        <p className="remote-target">{displayName(pending.edit.remote)}</p>
        <p>
          Load the server version into the local file opened in your editor?
        </p>
        <p className="hint">
          Review or save any unsaved editor changes first. YuuTerm cannot see
          unsaved VS Code buffers. Reload replaces the local disk file and
          retains a backup of its previous contents. VS Code may ask you to
          resolve an editor conflict.
        </p>
        {problem && (
          <p className="file-error" role="alert">
            {problem}
          </p>
        )}
        <div className="dialog-actions">
          <button
            className="primary"
            disabled={busy}
            onClick={() => void reload()}
          >
            Reload server version
          </button>
          <button disabled={busy} onClick={() => void decline()}>
            Keep local
          </button>
        </div>
      </Modal>
    );
  return (
    <Modal
      title="Upload saved changes?"
      onClose={busy ? undefined : () => void decline()}
    >
      <p className="remote-target">{displayName(pending.edit.remote)}</p>
      <p>The local file was saved. Upload this version to the server?</p>
      <p className="hint">
        Auto-upload applies to locally edited files on this connection until
        disconnect. Remote changes still block replacement. Replacing a file
        preserves basic owner/group/mode. Windows OpenSSH exposes mapped
        IDs/read-only bits; NTFS ACLs are not preserved. Extended ACLs, xattrs
        and hard-link relationships are not preserved.
      </p>
      {problem && (
        <p className="file-error" role="alert">
          {problem}
        </p>
      )}
      <div className="dialog-actions">
        <button
          className="primary"
          disabled={busy}
          onClick={() => void upload(false)}
        >
          Yes, upload
        </button>
        <button disabled={busy} onClick={() => void upload(true)}>
          Auto-upload all
        </button>
        <button disabled={busy} onClick={() => void decline()}>
          No, keep local
        </button>
      </div>
    </Modal>
  );
}
