import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Undo2 } from "lucide-react";
import { TerminalPane, type HandOff } from "./TerminalPane";
import { Modal } from "./Modal";
import { applyChrome, themeFor, withTextColor } from "./themes";
import type { CursorStyle, Highlighting, Live, Meta, Protocol } from "./types";

type Attach = {
  title: string;
  protocol: Protocol;
  snapshot: string;
  font_size: number;
  font_family: string;
  font_color: string | null;
  scrollback_lines: number;
  cursor_style: CursorStyle;
  steady_cursor: boolean;
  theme: string;
  mocha: boolean;
  highlighting: Highlighting;
  fixture: boolean;
};

// A terminal session in its own window. The backend binds this window to
// one session; closing the window or "Back to main window" hands the screen
// content back and the main window shows the session again.
export function Popout() {
  const [attach, setAttach] = useState<Attach | null>(null);
  const [meta, setMeta] = useState<Meta | undefined>();
  const [problem, setProblem] = useState("");
  const [paste, setPaste] = useState<{ text: string; send: () => void } | null>(
    null,
  );
  const handOff = useRef<HandOff | null>(null);
  const returning = useRef(false);
  const handBack = async () => {
    if (returning.current) return;
    returning.current = true;
    try {
      const snapshot = (await handOff.current?.take()) ?? "";
      await invoke("popout_return", { snapshot });
    } catch (e) {
      returning.current = false;
      setProblem(String(e));
    }
  };
  const handBackRef = useRef(handBack);
  handBackRef.current = handBack;
  useEffect(() => {
    void invoke<Attach>("popout_attach")
      .then((a) => {
        setAttach(a);
        document.title = `${a.title} – YuuTerm`;
        applyChrome(themeFor(a.theme, a.mocha));
        if (a.fixture)
          void import("./fixture-popout").then((m) =>
            m.checkPopoutWindow(handOff, () => handBackRef.current()),
          );
      })
      .catch((e) => setProblem(String(e)));
    const unlistenClose = getCurrentWindow().onCloseRequested((event) => {
      event.preventDefault();
      void handBackRef.current();
    });
    const unlistenRecall = listen("popout-return-request", () => {
      void handBackRef.current();
    });
    return () => {
      void unlistenClose.then((u) => u());
      void unlistenRecall.then((u) => u());
    };
  }, []);
  if (!attach)
    return (
      <div className="popout-window">
        <p className="popout-status">{problem || "Opening terminal…"}</p>
      </div>
    );
  // The pop-out never knows the session ID; the pane only needs a stable key.
  const live: Live = { id: 0, profileId: 0, meta } as Live;
  const theme = themeFor(attach.theme, attach.mocha);
  return (
    <div className="popout-window">
      <header className="popout-header">
        <span className={`dot ${meta?.connected ? "connected" : ""}`} />
        <span className="popout-status">
          {problem ||
            (meta?.finished
              ? "Connection closed"
              : meta?.connected
                ? "Connected"
                : (meta?.status ?? "Connecting…"))}
        </span>
        <button onClick={() => void handBack()}>
          <Undo2 size={13} />
          Back to main window
        </button>
      </header>
      <TerminalPane
        transport="popout"
        initial={attach.snapshot}
        handOff={handOff}
        live={live}
        protocol={attach.protocol}
        fontSize={attach.font_size}
        fontFamily={attach.font_family}
        scrollbackLines={attach.scrollback_lines}
        cursorStyle={attach.cursor_style}
        steadyCursor={attach.steady_cursor}
        theme={withTextColor(theme.terminal, attach.font_color)}
        highlighting={attach.highlighting}
        onMeta={(_, m) => setMeta(m)}
        onLogging={() => {}}
        onError={setProblem}
        onPaste={(_, text, send) => setPaste({ text, send })}
        onFocus={() => {}}
        fixture={attach.fixture}
        fixtureProbe={false}
        gpu={!attach.fixture}
        focusRequest={0}
        active
        findRequest={0}
      />
      {paste && (
        <Modal title="Confirm terminal paste" onClose={() => setPaste(null)}>
          <p>
            This paste contains line breaks or control characters and may
            execute commands.
          </p>
          <pre>{JSON.stringify(paste.text)}</pre>
          <div className="dialog-actions">
            <button
              className="primary"
              onClick={() => {
                paste.send();
                setPaste(null);
              }}
            >
              Paste
            </button>
            <button onClick={() => setPaste(null)}>Cancel</button>
          </div>
        </Modal>
      )}
    </div>
  );
}
