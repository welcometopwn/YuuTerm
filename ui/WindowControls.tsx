import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, Copy, X } from "lucide-react";

export function WindowControls() {
  const [maximized, setMaximized] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const window = getCurrentWindow();
    const sync = async () => {
      const value = await window.isMaximized();
      if (!disposed) setMaximized(value);
    };
    // Register before reading, so a resize during setup cannot leave a stale icon.
    void window
      .onResized(() => void sync().catch(() => {}))
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
        return sync();
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const run = async (action: "minimize" | "toggleMaximize" | "close") => {
    try {
      setError("");
      // close() follows the same native close-request path as Alt+F4, including
      // the existing pending-output drain and shutdown guards.
      await getCurrentWindow()[action]();
    } catch {
      setError("Window action failed. Try Alt+Space or Alt+F4.");
    }
  };

  return (
    <div className="window-controls" aria-label="Window controls">
      {error && (
        <span className="window-error" role="alert">
          {error}
        </span>
      )}
      <button
        title="Minimize"
        aria-label="Minimize"
        onClick={() => void run("minimize")}
      >
        <Minus size={14} />
      </button>
      <button
        title={maximized ? "Restore" : "Maximize"}
        aria-label={maximized ? "Restore" : "Maximize"}
        onClick={() => void run("toggleMaximize")}
      >
        {maximized ? <Copy size={12} /> : <Square size={12} />}
      </button>
      <button
        className="window-close"
        title="Close window"
        aria-label="Close window"
        onClick={() => void run("close")}
      >
        <X size={16} />
      </button>
    </div>
  );
}
