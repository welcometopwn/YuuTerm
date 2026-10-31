import { useEffect, useRef, useState } from "react";
import { Terminal } from "lucide-react";
import { normalizeIcon } from "./icon-normalization";

export function SessionIcon({
  icon,
  size = 16,
}: {
  icon?: string;
  size?: number;
}) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [icon]);
  return icon && !failed ? (
    <img
      className="session-icon"
      src={icon}
      width={size}
      height={size}
      alt=""
      onError={() => setFailed(true)}
    />
  ) : (
    <Terminal size={size} />
  );
}

export function IconPicker({
  value,
  onChange,
  onBusy,
}: {
  value?: string;
  onChange: (icon: string) => void;
  onBusy: (busy: boolean) => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  const live = useRef(true);
  const latest = useRef({ onChange, onBusy });
  latest.current = { onChange, onBusy };
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
      latest.current.onBusy(false);
    };
  }, []);
  return (
    <div className="icon-picker">
      <span className="icon-preview">
        <SessionIcon icon={value} size={24} />
      </span>
      <input
        hidden
        ref={input}
        type="file"
        accept="image/png,image/x-icon,image/svg+xml,.png,.ico,.svg"
        aria-label="Session PNG icon"
        onChange={async (e) => {
          const file = e.currentTarget.files?.[0];
          e.currentTarget.value = "";
          if (!file || busy) return;
          setBusy(true);
          latest.current.onBusy(true);
          setError("");
          try {
            const icon = await normalizeIcon(file);
            if (live.current) latest.current.onChange(icon);
          } catch (e) {
            if (live.current)
              setError(
                e instanceof Error ? e.message : "Cannot read this icon.",
              );
          } finally {
            if (live.current) {
              setBusy(false);
              latest.current.onBusy(false);
            }
          }
        }}
      />
      <button
        type="button"
        disabled={busy}
        onClick={() => input.current?.click()}
      >
        {busy ? "Reading…" : "Choose icon"}
      </button>
      {value && (
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            setError("");
            onChange("");
          }}
        >
          Reset icon
        </button>
      )}
      {error && (
        <small className="icon-problem" role="alert">
          {error}
        </small>
      )}
    </div>
  );
}
