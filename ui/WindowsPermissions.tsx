export function WindowsPermissions({
  directory,
  mode,
  disabled,
  onChange,
}: {
  directory: boolean;
  mode: string;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <>
      <p className="hint">
        Windows OpenSSH reports mapped permissions and owner/group IDs. These
        are not NTFS ACLs. Group, execute and special POSIX bits cannot be
        edited here.
      </p>
      {directory ? (
        <p className="hint">
          Directory access must be managed using Windows security tools.
        </p>
      ) : (
        <label>
          <input
            type="checkbox"
            aria-label="Windows read-only"
            disabled={disabled}
            checked={!(parseInt(mode, 8) & 0o200)}
            onChange={(e) => onChange(e.target.checked ? "400" : "600")}
          />
          Read-only (Windows attribute mapping)
        </label>
      )}
    </>
  );
}
