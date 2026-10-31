import type { Profile } from "./types";
export function LogRotationSettings({
  value,
  onChange,
}: {
  value: Profile["logging"];
  onChange: (value: Profile["logging"]) => void;
}) {
  const policy = value.rotation;
  return (
    <>
      <label className="check-label">
        <input
          type="checkbox"
          aria-label="Rotate output logs"
          disabled={!value.enabled}
          checked={!!policy}
          onChange={(e) =>
            onChange({
              ...value,
              rotation: e.target.checked
                ? { max_mib: 16, keep_files: 4 }
                : null,
            })
          }
        />
        Rotate logs by size
      </label>
      {policy && (
        <>
          <label>
            Maximum MiB per file
            <input
              aria-label="Log rotation MiB"
              type="number"
              min={1}
              max={64}
              disabled={!value.enabled}
              value={policy.max_mib}
              onChange={(e) =>
                onChange({
                  ...value,
                  rotation: { ...policy, max_mib: Number(e.target.value) },
                })
              }
            />
          </label>
          <label>
            Files to keep (including active)
            <input
              aria-label="Log rotation kept files"
              type="number"
              min={2}
              max={10}
              disabled={!value.enabled}
              value={policy.keep_files}
              onChange={(e) =>
                onChange({
                  ...value,
                  rotation: { ...policy, keep_files: Number(e.target.value) },
                })
              }
            />
          </label>
          <p className="hint">
            Older owned archives are deleted. With no custom path, reconnects
            append to current.log in this session’s log directory. Archives use
            a separate .yuuterm-rotated folder; unknown contents stop recording.
            Applies on next connection.
          </p>
        </>
      )}
    </>
  );
}
