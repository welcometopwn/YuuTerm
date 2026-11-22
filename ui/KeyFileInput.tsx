import { invoke } from "@tauri-apps/api/core";
import { FolderOpen } from "lucide-react";

export function KeyFileInput({
  value,
  onChange,
  label,
}: {
  value: string;
  onChange: (path: string) => void;
  label: string;
}) {
  return (
    <span className="key-file-input">
      <input
        aria-label={label}
        value={value}
        maxLength={4096}
        onChange={(e) => onChange(e.target.value)}
      />
      <button
        type="button"
        onClick={(e) => {
          e.preventDefault();
          void invoke<string | null>("pick_key_file").then(
            (path) => path && onChange(path),
          );
        }}
      >
        <FolderOpen size={13} />
        Browse
      </button>
    </span>
  );
}
