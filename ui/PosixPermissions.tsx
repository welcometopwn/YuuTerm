import { symbolicMode } from "./file-mode";

const WHO = ["Owner", "Group", "Other"];
const BITS = [
  [4, "Read"],
  [2, "Write"],
  [1, "Execute"],
] as const;
const SPECIAL = [
  ["Set UID", "Set user ID", 0o4000],
  ["Set GID", "Set group ID", 0o2000],
  ["Sticky", "Sticky bit", 0o1000],
] as const;

export function PosixPermissions({
  mode,
  directory,
  recursive,
  disabled,
  onMode,
  onRecursive,
}: {
  mode: string;
  directory: boolean;
  recursive: boolean;
  disabled: boolean;
  onMode: (mode: string) => void;
  onRecursive: (recursive: boolean) => void;
}) {
  const valid = /^[0-7]{3,4}$/.test(mode);
  const value = valid ? parseInt(mode, 8) : 0;
  const toggle = (bit: number, checked: boolean) =>
    onMode((checked ? value | bit : value & ~bit).toString(8).padStart(3, "0"));
  return (
    <div className="posix-permissions">
      <div className="perm-table">
        <span />
        {BITS.map(([, label]) => (
          <span key={label} className="perm-head">
            {label}
          </span>
        ))}
        {WHO.map((who, i) => (
          <div className="perm-row" key={who}>
            <span className="perm-who">{who}</span>
            {BITS.map(([bit, label]) => {
              const flag = bit << (6 - i * 3);
              return (
                <label key={label} className="perm-cell">
                  <input
                    type="checkbox"
                    aria-label={`${who} ${label.toLowerCase()}`}
                    checked={!!(value & flag)}
                    disabled={disabled}
                    onChange={(e) => toggle(flag, e.target.checked)}
                  />
                </label>
              );
            })}
          </div>
        ))}
      </div>
      <div className="perm-special">
        {SPECIAL.map(([label, title, bit]) => (
          <label key={label} title={title}>
            <input
              type="checkbox"
              aria-label={title}
              checked={!!(value & bit)}
              disabled={disabled}
              onChange={(e) => toggle(bit, e.target.checked)}
            />
            {label}
          </label>
        ))}
      </div>
      <div className="perm-mode">
        <label className="octal-mode">
          Octal
          <input
            aria-label="Octal mode"
            value={mode}
            maxLength={4}
            disabled={disabled}
            onChange={(e) => onMode(e.target.value)}
          />
        </label>
        <span className="perm-symbolic" aria-label="Symbolic mode">
          {valid ? symbolicMode(value) : "invalid"}
        </span>
      </div>
      {directory && (
        <label className="check-label perm-recursive">
          <input
            type="checkbox"
            aria-label="Apply recursively"
            checked={recursive}
            disabled={disabled}
            onChange={(e) => onRecursive(e.target.checked)}
          />
          Apply to everything inside (-R)
        </label>
      )}
    </div>
  );
}
