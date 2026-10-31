import { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Modal } from "./Modal";
import { normalizedGroup } from "./group-move-policy";
import type { Preferences } from "./types";

export function ManageGroup({
  group,
  remove,
  onSaved,
  onClose,
}: {
  group: string;
  remove: boolean;
  onSaved: (prefs: Preferences, name: string) => void;
  onClose: () => void;
}) {
  const [value, setValue] = useState(group);
  const [busy, setBusy] = useState(false);
  const working = useRef(false);
  const [failure, setFailure] = useState("");
  let name = "",
    invalid = "";
  if (!remove) {
    try {
      name = normalizedGroup(value);
      if (!name) invalid = "Enter a group name.";
    } catch (e) {
      invalid = String(e);
    }
  }
  const submit = async () => {
    if (working.current || invalid) return;
    working.current = true;
    setBusy(true);
    setFailure("");
    try {
      const prefs = await invoke<Preferences>("change_group", {
        group,
        newName: remove ? null : name,
      });
      onSaved(prefs, remove ? "" : name);
      onClose();
    } catch (e) {
      setFailure(String(e));
    } finally {
      working.current = false;
      setBusy(false);
    }
  };
  return (
    <Modal
      title={remove ? "Delete group" : "Rename group"}
      onClose={busy ? undefined : onClose}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        {remove ? (
          <p>
            Delete the group “{group}”? All sessions are kept and become
            ungrouped. Running connections stay open.
          </p>
        ) : (
          <>
            <label>
              Group name
              <input
                autoFocus
                aria-label="Group name"
                value={value}
                maxLength={256}
                disabled={busy}
                onChange={(e) => setValue(e.target.value)}
              />
            </label>
            <p>
              All sessions in this group move to the new name. Running
              connections stay open.
            </p>
          </>
        )}
        {(invalid || failure) && <p role="alert">{invalid || failure}</p>}
        <div className="dialog-actions">
          <button type="button" disabled={busy} onClick={onClose}>
            Cancel
          </button>
          <button
            className="primary"
            type="submit"
            disabled={busy || !!invalid}
          >
            {busy ? "Saving…" : remove ? "Delete group" : "Rename"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
