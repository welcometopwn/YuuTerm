import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { WebglAddon } from "@xterm/addon-webgl";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { ArrowUp, ArrowDown, X } from "lucide-react";
import type {
  Live,
  Meta,
  Poll,
  Highlighting,
  Protocol,
  OutputLogStatus,
} from "./types";
import { terminalHighlighting } from "./terminal-highlighting";
import { TelnetLineInput } from "./TelnetLineInput";
import { fixtureTerminals } from "./fixture-terminals";
import "@xterm/xterm/css/xterm.css";
import { requiresPasteConfirmation, layoutForKey } from "./input-policy";
import { fontStack, terminalFontsReady } from "./terminal-font";
import { ContextMenu, type MenuState } from "./ContextMenu";

interface Props {
  live: Live;
  protocol: Protocol;
  fontSize: number;
  fontFamily: string;
  scrollbackLines: number;
  theme: ITheme;
  highlighting: Highlighting;
  onMeta: (id: number, m: Meta) => void;
  onLogging: (id: number, logging: OutputLogStatus | null) => void;
  onError: (error: string) => void;
  onPaste: (id: number, text: string, send: () => void) => void;
  onFocus: () => void;
  fixture: boolean;
  fixtureProbe?: boolean;
  /** GPU (WebGL) rendering; the DOM renderer is the fallback. */
  gpu: boolean;
  focusRequest: number;
  active: boolean;
  findRequest: number;
}

export function TerminalPane(props: Props) {
  const [fontReady, setFontReady] = useState(false);
  const [logging, setLogging] = useState<OutputLogStatus | null>(null);
  const loggingRef = useRef<OutputLogStatus | null>(null);
  const [contextMenu, setContextMenu] = useState<MenuState | null>(null);
  const closeContextMenu = useCallback(() => setContextMenu(null), []);
  const pasteText = useRef<(text: string) => void>(() => {});
  const linePaste = useRef<(text: string) => void>(() => {});
  const host = useRef<HTMLDivElement>(null);
  const current = useRef(props);
  current.current = props;
  const terminal = useRef<Terminal | null>(null);
  const search = useRef<SearchAddon | null>(null);
  const findInput = useRef<HTMLInputElement>(null);
  const findVisible = useRef(false);
  const [findOpen, setFindOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [matchCase, setMatchCase] = useState(false);
  const [wholeWord, setWholeWord] = useState(false);
  const [findStatus, setFindStatus] = useState("");
  const openFind = () => {
    findVisible.current = true;
    setFindOpen(true);
    findInput.current?.focus();
    findInput.current?.select();
  };
  const closeFind = () => {
    findVisible.current = false;
    setFindOpen(false);
    search.current?.clearDecorations();
    terminal.current?.clearSelection();
    terminal.current?.focus();
  };
  const runFind = (previous = false, incremental = false) => {
    if (!search.current || !terminal.current) return;
    if (!query || query.length > 256 || /[\x00-\x1f\x7f-\x9f]/.test(query)) {
      search.current.clearDecorations();
      terminal.current.clearSelection();
      setFindStatus(query ? "Use plain text" : "");
      return;
    }
    const options = {
      regex: false,
      caseSensitive: matchCase,
      wholeWord,
      incremental,
    };
    const started = current.current.fixture ? performance.now() : 0;
    const found = previous
      ? search.current.findPrevious(query, options)
      : search.current.findNext(query, options);
    if (current.current.fixture && host.current) {
      host.current.dataset.searchDurationMs = String(
        performance.now() - started,
      );
      host.current.dataset.searchSelection = terminal.current
        .getSelection()
        .slice(0, 256);
      host.current.dataset.searchSequence = String(
        Number(host.current.dataset.searchSequence ?? 0) + 1,
      );
    }
    setFindStatus(found ? "Match" : "No match");
  };
  const highlighting = useRef<ReturnType<typeof terminalHighlighting> | null>(
    null,
  );
  const refit = useRef<() => void>(() => {});
  useEffect(() => {
    let mounted = true;
    void terminalFontsReady.then((loaded) => {
      if (!mounted) return;
      if (!loaded)
        current.current.onError(
          "Bundled terminal font could not load; using a system fallback.",
        );
      setFontReady(true);
    });
    return () => {
      mounted = false;
    };
  }, []);
  useEffect(() => {
    if (!fontReady) return;
    const element = host.current!;
    const term = new Terminal({
      fontFamily: fontStack(current.current.fontFamily),
      fontSize: current.current.fontSize,
      lineHeight: 1.24,
      fontWeight: "normal",
      fontWeightBold: "bold",
      scrollback: current.current.scrollbackLines,
      cursorBlink: true,
      allowProposedApi: true, // pinned xterm marker/decoration API; no remote JS
      convertEol: false,
      theme: current.current.theme,
    });
    // Match modern shells' wide Unicode symbols before parsing any prompt.
    // Legacy widths shift zsh redraws and can leave an undeletable first letter.
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(element);
    // WebGL keeps an idle pane (blinking cursor) near 1 % of a core instead of
    // ~10 % with the DOM renderer (docs/terminal-rendering.md). Without a GPU
    // context, or when it is lost, xterm falls back to the DOM renderer.
    if (current.current.gpu) {
      try {
        const gpu = new WebglAddon();
        gpu.onContextLoss(() => gpu.dispose());
        term.loadAddon(gpu);
        element.dataset.renderer = "webgl";
      } catch {
        element.dataset.renderer = "dom";
      }
    } else element.dataset.renderer = "dom";
    if (current.current.fixture) fixtureTerminals.set(element, term);
    terminal.current = term;
    const finder = new SearchAddon({ highlightLimit: 1 });
    term.loadAddon(finder);
    search.current = finder;
    findVisible.current = false;
    setFindOpen(false);
    setQuery("");
    setFindStatus("");
    const colors = terminalHighlighting(term, element, current.current.fixture);
    highlighting.current = colors;
    colors.update(current.current.highlighting);
    // OSC52 is intentionally consumed without OS clipboard access.
    const osc = term.parser.registerOscHandler(52, () => true);
    let fixtureProbed = false;
    let stopped = false,
      finished = false,
      readyCaptured = false;
    let meta: Meta | undefined;
    let tail = Promise.resolve(),
      pendingInputs = 0;
    const id = current.current.live.id;
    const fail = (e: unknown) => {
      if (!stopped) current.current.onError(String(e));
    };
    const send = (data: string) => {
      const bytes = new TextEncoder().encode(data);
      if (bytes.length > 16384 || pendingInputs >= 32) {
        fail("Terminal input is too large or busy; nothing was queued.");
        return;
      }
      pendingInputs++;
      tail = tail
        .then(async () => {
          if (!stopped)
            await invoke("session_input", { id, bytes: Array.from(bytes) });
        })
        .catch(fail)
        .finally(() => {
          pendingInputs--;
        });
    };
    const lineMode =
      current.current.protocol === "Telnet" &&
      !!current.current.live.telnetLineEditing;
    term.options.disableStdin = lineMode;
    const input = term.onData((data) => {
      if (!lineMode) send(data);
    });
    term.attachCustomKeyEventHandler((event) => {
      // Layout keys are handled by the window listener; never send them remotely.
      if (layoutForKey(event)) return false;
      if (event.ctrlKey && !event.altKey && !event.metaKey) {
        const key = event.key.toLowerCase();
        // Leave ordinary Ctrl+C/Ctrl+V to WebView2's native clipboard events.
        // xterm would otherwise turn them into ETX / quote-next (^V).
        // With no selection Ctrl+C must still interrupt the remote process.
        if (key === "v" && !event.shiftKey) return false;
        if (key === "c" && !event.shiftKey && term.hasSelection()) return false;
        if (key === "v" && event.shiftKey) {
          if (event.type === "keydown") {
            event.preventDefault();
            event.stopPropagation();
            void navigator.clipboard
              .readText()
              .then((text) => {
                if (!stopped) pasteText.current(text);
              })
              .catch(fail);
          }
          return false;
        }
      }
      if (
        event.type === "keydown" &&
        event.ctrlKey &&
        !event.shiftKey &&
        !event.altKey &&
        event.key.toLowerCase() === "f"
      ) {
        event.preventDefault();
        event.stopPropagation();
        openFind();
        return false;
      }
      if (
        event.type === "keydown" &&
        event.ctrlKey &&
        event.shiftKey &&
        event.key.toLowerCase() === "c"
      ) {
        event.preventDefault();
        event.stopPropagation();
        if (term.hasSelection())
          void navigator.clipboard.writeText(term.getSelection()).catch(fail);
        return false;
      }
      return true;
    });
    const acceptPaste = (text: string) => {
      if (lineMode) {
        linePaste.current(text);
        return;
      }
      if (new TextEncoder().encode(text).length > 16000) {
        fail("Paste exceeds the 16000-byte limit; nothing was sent.");
        return;
      }
      const action = () => {
        if (!stopped && !finished) term.paste(text);
        else fail("Original connection has closed; paste discarded.");
      };
      if (requiresPasteConfirmation(text))
        current.current.onPaste(id, text, action);
      else action();
    };
    pasteText.current = acceptPaste;
    const onPaste = (event: ClipboardEvent) => {
      const text = event.clipboardData?.getData("text/plain");
      if (text === undefined) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      acceptPaste(text);
    };
    element.addEventListener("paste", onPaste, true);
    let size = "";
    const resize = () => {
      if (element.clientWidth < 50 || element.clientHeight < 40) return;
      fit.fit();
      const cols = Math.min(240, Math.max(10, term.cols)),
        rows = Math.min(100, Math.max(2, term.rows));
      if (term.cols !== cols || term.rows !== rows) term.resize(cols, rows);
      const next = `${cols}:${rows}`;
      if (meta?.connected && size !== next) {
        size = next;
        void invoke("session_resize", { id, rows, cols }).catch(fail);
      }
    };
    const observer = new ResizeObserver(resize);
    refit.current = resize;
    observer.observe(element);
    const poll = async () => {
      try {
        while (!stopped) {
          const p = await invoke<Poll>("session_poll", { id, waitMs: 200 });
          if (stopped) break;
          if (current.current.fixture) {
            element.dataset.logBytes = String(p.logging?.bytes ?? 0);
            element.dataset.logPath = p.logging?.path ?? "";
            element.dataset.logActive = String(!!p.logging?.active);
          }
          if (
            p.logging?.active !== loggingRef.current?.active ||
            p.logging?.finished !== loggingRef.current?.finished ||
            p.logging?.error !== loggingRef.current?.error ||
            p.logging?.path !== loggingRef.current?.path
          ) {
            loggingRef.current = p.logging;
            setLogging(p.logging);
            current.current.onLogging(id, p.logging);
          }
          if (p.meta) {
            meta = { ...p.meta, files: p.meta.files ?? meta?.files ?? [] };
            finished = meta.finished;
            current.current.onMeta(id, meta);
            resize();
            if (
              current.current.fixture &&
              current.current.fixtureProbe !== false &&
              meta.connected &&
              !fixtureProbed
            ) {
              fixtureProbed = true;
              send("ui-stream-check\r");
            }
          }
          for (const chunk of p.chunks) {
            if (stopped) break;
            await new Promise<void>((resolve) =>
              term.write(new Uint8Array(chunk), resolve),
            );
          }
          if (current.current.fixture) {
            const b = term.buffer.active;
            element.dataset.scrollbackLimit = String(term.options.scrollback);
            element.dataset.historyLength = String(b.length);
            element.dataset.terminalRows = String(term.rows);
            element.dataset.terminalCols = String(term.cols);
            element.dataset.bufferTail = Array.from(
              { length: 5 },
              (_, offset) => b.getLine(b.baseY + b.cursorY - 4 + offset),
            )
              .map(
                (line) =>
                  `${line?.isWrapped ? "" : "\n"}${line?.translateToString(true) ?? ""}`,
              )
              .join("")
              .slice(-256);
            const viewport = Array.from(
              { length: term.rows },
              (_, i) =>
                b.getLine(b.viewportY + i)?.translateToString(true) ?? "",
            ).join("\n");
            element.dataset.dispatchEcho = String(
              viewport.includes("broadcast-fixture-check"),
            );
            element.dataset.performanceEcho =
              viewport.match(/bench-\d+|bench-done/g)?.at(-1) ?? "";
          }
          if (
            current.current.fixture &&
            document.documentElement.dataset.fixtureChecks === "passed" &&
            document.documentElement.dataset.captureStarted !== "true" &&
            !readyCaptured &&
            meta?.connected &&
            (current.current.protocol === "Telnet" ||
              (meta.files?.length ?? 0) >= 2)
          ) {
            const lines = Array.from(
              { length: term.rows },
              (_, i) =>
                term.buffer.active.getLine(i)?.translateToString(true) ?? "",
            ).join("\n");
            if (lines.includes("Ready") && lines.includes("ui-stream-check")) {
              readyCaptured = true;
              document.documentElement.dataset.captureStarted = "true";
              requestAnimationFrame(() =>
                requestAnimationFrame(() => {
                  void invoke("capture_ready", { terminalReady: true }).catch(
                    fail,
                  );
                }),
              );
            }
          }
          if (finished && !p.more && (!p.logging || p.logging.finished)) break;
          if (p.more) await new Promise((resolve) => setTimeout(resolve, 0));
        }
      } catch (e) {
        fail(e);
      }
    };
    void poll();
    return () => {
      stopped = true;
      pasteText.current = () => {};
      setContextMenu(null);
      refit.current = () => {};
      observer.disconnect();
      input.dispose();
      osc.dispose();
      colors.dispose();
      search.current = null;
      highlighting.current = null;
      element.removeEventListener("paste", onPaste, true);
      term.dispose();
      terminal.current = null;
    };
  }, [props.live.id, fontReady]);
  useEffect(() => {
    if (!findOpen) return;
    // Focus after React mounts the field, rather than racing its state update.
    const frame = requestAnimationFrame(() => {
      findInput.current?.focus();
      findInput.current?.select();
    });
    return () => cancelAnimationFrame(frame);
  }, [findOpen]);
  useEffect(() => {
    if (props.findRequest > 0) openFind();
  }, [props.findRequest]);
  useEffect(() => {
    if (!findOpen) return;
    const timer = setTimeout(() => runFind(false, true), 150);
    return () => clearTimeout(timer);
  }, [findOpen, query, matchCase, wholeWord]);
  useEffect(() => {
    highlighting.current?.update(props.highlighting);
  }, [props.highlighting]);
  useEffect(() => {
    if (props.active && !document.querySelector("dialog[open]")) {
      const before = document.activeElement;
      const frame = requestAnimationFrame(() => {
        const focused = document.activeElement;
        // Startup/reconnect can schedule focus just before the user enters a
        // path field. A delayed frame must not steal that newer focus intent.
        if (
          findVisible.current ||
          !current.current.active ||
          document.querySelector("dialog[open]") ||
          focused !== before
        )
          return;
        if (
          focused instanceof HTMLElement &&
          (focused.matches('input, select, [contenteditable="true"]') ||
            (focused.tagName === "TEXTAREA" &&
              !focused.closest(".terminal-host")))
        )
          return;
        terminal.current?.focus();
      });
      return () => cancelAnimationFrame(frame);
    }
  }, [props.active, props.focusRequest, props.live.id, fontReady]);
  useEffect(() => {
    if (terminal.current) {
      terminal.current.options.fontSize = props.fontSize;
      refit.current();
    }
  }, [props.fontSize]);
  useEffect(() => {
    if (terminal.current) {
      terminal.current.options.fontFamily = fontStack(props.fontFamily);
      refit.current();
    }
  }, [props.fontFamily]);
  useEffect(() => {
    if (terminal.current)
      terminal.current.options.scrollback = props.scrollbackLines;
  }, [props.scrollbackLines]);
  useEffect(() => {
    if (terminal.current) terminal.current.options.theme = props.theme;
  }, [props.theme, fontReady]);
  return (
    <div className="terminal-shell">
      {logging?.error && (
        <div className="output-log-status failed" role="alert">
          {logging.error}
        </div>
      )}
      {contextMenu && (
        <ContextMenu menu={contextMenu} onClose={closeContextMenu} />
      )}
      {findOpen && (
        <div
          className="terminal-find"
          role="search"
          aria-label="Find in terminal"
          onMouseDown={() => current.current.onFocus()}
        >
          <input
            ref={findInput}
            type="search"
            aria-label="Find terminal text"
            placeholder="Find in terminal"
            maxLength={256}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                e.stopPropagation();
                runFind(e.shiftKey);
              }
              if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                closeFind();
              }
            }}
          />
          <button
            className="icon"
            title="Match case"
            aria-label="Match case"
            aria-pressed={matchCase}
            onClick={() => {
              setMatchCase((v) => !v);
              findInput.current?.focus();
            }}
          >
            Aa
          </button>
          <button
            className="icon"
            title="Whole word"
            aria-label="Whole word"
            aria-pressed={wholeWord}
            onClick={() => {
              setWholeWord((v) => !v);
              findInput.current?.focus();
            }}
          >
            ab
          </button>
          <span role="status">{findStatus}</span>
          <button
            className="icon"
            title="Previous match (Shift+Enter)"
            aria-label="Previous match"
            onClick={() => {
              runFind(true);
              findInput.current?.focus();
            }}
          >
            <ArrowUp size={13} />
          </button>
          <button
            className="icon"
            title="Next match (Enter)"
            aria-label="Next match"
            onClick={() => {
              runFind();
              findInput.current?.focus();
            }}
          >
            <ArrowDown size={13} />
          </button>
          <button
            className="icon"
            title="Close search (Escape)"
            aria-label="Close search"
            onClick={closeFind}
          >
            <X size={13} />
          </button>
        </div>
      )}
      {props.protocol === "Telnet" && props.live.telnetLineEditing && (
        <TelnetLineInput
          key={props.live.id}
          id={props.live.id}
          disabled={!props.live.meta?.connected || !!props.live.meta?.finished}
          onError={props.onError}
          appendRef={linePaste}
        />
      )}
      <div
        className="terminal-host"
        ref={host}
        style={{ background: props.theme.background }}
        onMouseDown={() => current.current.onFocus()}
        onContextMenu={(event) => {
          event.preventDefault();
          event.stopPropagation();
          const term = terminal.current;
          if (!term) return;
          const bounds = event.currentTarget.getBoundingClientRect();
          setContextMenu({
            x: event.clientX || bounds.left + 12,
            y: event.clientY || bounds.top + 12,
            actions: [
              {
                label: "Copy",
                disabled: !term.hasSelection(),
                run: () => {
                  void navigator.clipboard
                    .writeText(term.getSelection())
                    .catch(current.current.onError);
                },
              },
              {
                label: "Paste",
                disabled:
                  !current.current.live.meta?.connected ||
                  current.current.live.meta?.finished,
                run: () => {
                  void navigator.clipboard
                    .readText()
                    .then(pasteText.current)
                    .catch(current.current.onError);
                },
              },
              { label: "Select all", run: () => term.selectAll() },
              { label: "Find in terminal", run: openFind },
            ],
          });
        }}
      />
    </div>
  );
}
