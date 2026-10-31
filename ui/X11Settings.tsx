import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./x11.css";
export function X11Settings({
  enabled,
  onChange,
}: {
  enabled: boolean;
  onChange: (value: boolean) => void;
}) {
  const [probe, setProbe] = useState<{ available: boolean; message: string }>();
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    let active = true;
    setProbe(undefined);
    invoke<{ available: boolean; message: string }>("x11_probe")
      .then((value) => {
        if (active) setProbe(value);
      })
      .catch(() => {
        if (active)
          setProbe({
            available: false,
            message:
              "X11 detection is unavailable in this build. Forwarding cannot be enabled.",
          });
      });
    return () => {
      active = false;
    };
  }, [revision]);
  return (
    <fieldset className="x11-settings">
      <legend>X11 applications</legend>
      <label className="check-label">
        <input
          type="checkbox"
          aria-label="Forward X11 applications"
          checked={enabled}
          disabled={!enabled && !probe?.available}
          onChange={(e) => {
            if (!e.target.checked || probe?.available)
              onChange(e.target.checked);
          }}
        />
        Show remote Linux apps on this computer (X11)
      </label>
      <p role="status">
        {probe?.available
          ? "Ready. YuuTerm handles setup automatically when you connect."
          : probe
            ? "X11 is unavailable. Check the details below."
            : "Checking availability…"}
      </p>
      <button
        type="button"
        disabled={!probe}
        onClick={() => setRevision((value) => value + 1)}
      >
        Check again
      </button>
      <details className="advanced">
        <summary>How X11 works and security</summary>
        <small>{probe?.message}</small>
        <small>
          Enable only for computers you trust. Remote apps share access to the
          local X11 desktop. Disconnecting SSH closes their connection.
        </small>
      </details>
    </fieldset>
  );
}
