import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Modal } from "./Modal";
import type { Live } from "./types";
import "./restore-edits.css";
type Item = {
  key: string;
  remote: string;
  server: string;
  local: string;
  profile_id: number | null;
  state: string;
  revision: string | null;
  verified: boolean;
};
type Listing = { items: Item[]; notice: string };
export function RestoreEdits({
  enabled,
  lives,
  onManual,
}: {
  enabled: boolean;
  lives: Live[];
  onManual: (id: number) => void;
}) {
  const [listing, setListing] = useState<Listing>({ items: [], notice: "" });
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState("");
  const [discard, setDiscard] = useState<string | null>(null);
  const [kept, setKept] = useState<string[]>([]);
  const [later, setLater] = useState<string[]>([]);
  // At startup, records with nothing left to upload (unchanged, missing or
  // legacy path-only copies) are tidied silently: the record goes, local files
  // stay. Only unsent changes remain, and they are offered when their original
  // SSH session is connected again, not as a startup dialog.
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    void (async () => {
      try {
        const value = await invoke<Listing>("edit_restore_list");
        const pending: Item[] = [];
        for (const item of value.items) {
          if (
            ["Unchanged", "Missing local copy", "Unverified baseline"].includes(
              item.state,
            )
          )
            await invoke("edit_restore_decide", {
              key: item.key,
              decision: "keep",
              revision: item.revision,
            }).catch(() => pending.push(item));
          else pending.push(item);
        }
        if (active) setListing({ ...value, items: pending });
      } catch (e) {
        if (active) setProblem(String(e));
      }
    })();
    return () => {
      active = false;
    };
  }, [enabled]);
  const connectedLive = (item: Item) =>
    lives.find(
      (l) =>
        l.profileId === item.profile_id &&
        l.protocol === "Ssh" &&
        l.meta?.connected &&
        !l.meta.finished,
    );
  const offered = listing.items.filter(
    (item) => !!connectedLive(item) && !later.includes(item.key),
  );
  useEffect(() => {
    if (offered.length && !open) setOpen(true);
  }, [offered.length]);
  const refresh = async (key?: string) => {
    setBusy(true);
    setProblem("");
    try {
      const value = await invoke<Listing>("edit_restore_list", { key });
      setListing((previous) =>
        key
          ? {
              ...previous,
              items: previous.items.map((row) =>
                row.key === key ? (value.items[0] ?? row) : row,
              ),
            }
          : value,
      );
    } catch (e) {
      setProblem(String(e));
    } finally {
      setBusy(false);
    }
  };
  const decide = async (
    item: Item,
    decision: "upload" | "keep" | "discard",
    id?: number,
  ) => {
    setBusy(true);
    setProblem("");
    if (id !== undefined) onManual(id);
    try {
      await invoke("edit_restore_decide", {
        key: item.key,
        decision,
        id,
        revision: item.revision,
      });
      if (decision === "keep")
        setKept((paths) => [...paths, item.local].slice(-32));
      setListing((previous) => ({
        ...previous,
        items: previous.items.filter((row) => row.key !== item.key),
      }));
    } catch (e) {
      setProblem(String(e));
    } finally {
      setBusy(false);
    }
  };
  const close = () => {
    setLater((keys) => [...keys, ...offered.map((i) => i.key)]);
    setOpen(false);
  };
  if (!enabled || !open || !offered.length) return null;
  return (
    <>
      {open && (
        <Modal
          title="Unsent changes from an earlier session"
          onClose={busy ? undefined : close}
        >
          <p>
            These files were saved in the editor during an earlier YuuTerm run
            but never uploaded. Nothing uploads without your choice.
          </p>
          <p className="hint">
            Upload checks the original server content and metadata and this
            reviewed local revision. Conflicts preserve both copies. Keep local
            copy removes only its recovery record. Discard permanently deletes
            that staged file; backups are retained. Unsaved editor buffers
            cannot be inspected.
          </p>
          <div className="restore-edits-list">
            {offered.map((item) => {
              const live = connectedLive(item);
              return (
                <section
                  key={item.key}
                  data-recovery-key={item.key}
                  data-remote={item.remote}
                >
                  <strong>{item.remote}</strong>
                  <div>{item.server}</div>
                  <p>
                    Local change state: <b>{item.state}</b>
                  </p>
                  <code>{item.local}</code>
                  {!live && (
                    <p className="hint">
                      Connect the original SSH bookmark to upload; changed
                      bookmark targets will be refused.
                    </p>
                  )}
                  <div className="dialog-actions">
                    <button
                      disabled={
                        busy || !live || !item.verified || !item.revision
                      }
                      onClick={() => void decide(item, "upload", live?.id)}
                    >
                      Upload
                    </button>
                    <button
                      disabled={busy}
                      onClick={() => void decide(item, "keep")}
                    >
                      Keep local copy
                    </button>
                    <button
                      disabled={busy || !item.revision}
                      onClick={() => {
                        if (discard === item.key) {
                          setDiscard(null);
                          void decide(item, "discard");
                        } else setDiscard(item.key);
                      }}
                    >
                      {discard === item.key ? "Confirm discard" : "Discard"}
                    </button>
                    <button
                      disabled={busy}
                      onClick={() => void refresh(item.key)}
                    >
                      Inspect local changes
                    </button>
                  </div>
                </section>
              );
            })}
          </div>
          {problem && (
            <p className="file-error" role="alert">
              {problem}
            </p>
          )}
          {!!kept.length && (
            <details>
              <summary>{kept.length} local copies retained</summary>
              {kept.map((path) => (
                <p key={path}>
                  <code>{path}</code>
                </p>
              ))}
            </details>
          )}
          <p className="hint">{listing.notice}</p>
          <div className="dialog-actions">
            <button disabled={busy} onClick={() => void refresh()}>
              Refresh recovery list
            </button>
            <button disabled={busy} onClick={close}>
              Later
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}
