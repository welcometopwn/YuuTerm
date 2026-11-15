import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { localShellLabel, localShells } from "./types";
import type { LocalShell } from "./types";

export function LocalShellSelect({
  value,
  onChange,
  label,
}: {
  value: LocalShell;
  onChange: (shell: LocalShell) => void;
  label?: string;
}) {
  const [available, setAvailable] = useState<LocalShell[] | null>(null);
  useEffect(() => {
    let active = true;
    invoke<LocalShell[]>("local_shells")
      .then((shells) => active && setAvailable(shells))
      .catch(() => {});
    return () => {
      active = false;
    };
  }, []);
  return (
    <select
      aria-label={label}
      value={value}
      onChange={(e) => onChange(e.target.value as LocalShell)}
    >
      {localShells.map((v) => {
        const missing = !!available && !available.includes(v);
        return (
          <option key={v} value={v} disabled={missing && v !== value}>
            {localShellLabel(v)}
            {missing ? " (not installed)" : ""}
          </option>
        );
      })}
    </select>
  );
}
