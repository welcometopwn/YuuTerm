import { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Modal } from "./Modal";
import { GroupCombo } from "./GroupCombo";
import { groupNames } from "./session-groups";
import { normalizedGroup } from "./group-move-policy";
import type { Preferences, Profile } from "./types";

export function MoveToGroup({
  profile,
  profileIds,
  profiles,
  onSaved,
  onClose,
}: {
  profile: Profile;
  profileIds?: number[];
  profiles: Profile[];
  onSaved: (p: Preferences) => void;
  onClose: () => void;
}) {
  const [value, setValue] = useState(profile.group ?? "");
  const [busy, setBusy] = useState(false);
  const working = useRef(false);
  const [failure, setFailure] = useState("");
  let group = "",
    invalid = "";
  try {
    group = normalizedGroup(value);
  } catch (e) {
    invalid = String(e);
  }
  const submit = async () => {
    if (working.current || invalid) return;
    working.current = true;
    setBusy(true);
    setFailure("");
    try {
      onSaved(
        await invoke<Preferences>("move_profiles", {
          profileIds: profileIds ?? [profile.id],
          group,
          beforeId: null,
        }),
      );
      onClose();
    } catch (e) {
      setFailure(String(e));
    } finally {
      working.current = false;
      setBusy(false);
    }
  };
  return (
    <Modal title="Move to group" onClose={busy ? undefined : onClose}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <p>
          Move{" "}
          {profileIds && profileIds.length > 1
            ? `${profileIds.length} selected sessions`
            : profile.name}
          . Running connections stay open.
        </p>
        <label>
          Group
          <GroupCombo
            aria-label="Destination group"
            value={value}
            options={groupNames(profiles)}
            maxLength={256}
            disabled={busy}
            onChange={setValue}
          />
        </label>
        <p>
          Choose an existing name or enter a new one. Leave empty for ungrouped
          sessions. Maximum 64 UTF-8 bytes.
        </p>
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
            {busy ? "Moving…" : "Move"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
