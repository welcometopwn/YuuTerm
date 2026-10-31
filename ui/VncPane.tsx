import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Live, Meta } from "./types";
import { Modal } from "./Modal";
import {
  VncKeyboard,
  clipboardError,
  fit,
  pointer,
  packet,
  type VncInput,
  type VncStatus,
} from "./vnc-wire";
import "./vnc.css";

export function VncPane({
  live,
  visible,
  active,
  focusRequest,
  onMeta,
  onError,
  onFocus,
}: {
  live: Live;
  visible: boolean;
  active: boolean;
  focusRequest: number;
  onMeta: (id: number, meta: Meta) => void;
  onError: (error: unknown) => void;
  onFocus: () => void;
}) {
  const canvas = useRef<HTMLCanvasElement>(null),
    current = useRef<VncStatus | null>(null);
  const callbacks = useRef({ onMeta, onError });
  callbacks.current = { onMeta, onError };
  const shown = useRef(visible);
  shown.current = visible;
  const [status, setStatus] = useState<VncStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [clipboardOpen, setClipboardOpen] = useState(false),
    [clipboardText, setClipboardText] = useState("");
  const queue = useRef<VncInput[]>([]),
    sending = useRef(false),
    alive = useRef(true);
  const keyboard = useRef(new VncKeyboard());
  // Physical code of a Ctrl+V waiting for the WebView's paste event.
  const pasting = useRef<string | null>(null);
  const cursorSerial = useRef(0);
  const [box, setBox] = useState<{ width: number; height: number } | null>(
    null,
  );
  const close = () => invoke("vnc_close", { id: live.id, remove: false });
  const send = useRef<(input: VncInput) => void>(() => {});
  send.current = (input) => {
    if (!alive.current || !current.current?.connected) return;
    const last = queue.current.at(-1);
    if (
      input.kind === "pointer" &&
      last?.kind === "pointer" &&
      input.buttons === last.buttons
    )
      queue.current[queue.current.length - 1] = input;
    else queue.current.push(input);
    if (queue.current.length > 64) {
      queue.current = [];
      void close();
      callbacks.current.onError(
        "VNC input queue full; disconnected to release keys",
      );
      return;
    }
    if (sending.current) return;
    sending.current = true;
    void (async () => {
      try {
        while (alive.current && queue.current.length) {
          const commands = queue.current.splice(0, 64);
          await invoke("vnc_input", { id: live.id, commands });
        }
      } catch (error) {
        queue.current = [];
        void close();
        callbacks.current.onError(error);
      } finally {
        sending.current = false;
      }
    })();
  };
  const release = useRef(() => {});
  release.current = () => {
    keyboard.current.reset();
    pasting.current = null;
    send.current({ kind: "releaseAll" });
  };
  const sendAll = (inputs: VncInput[]) => {
    for (const input of inputs) send.current(input);
  };
  useEffect(() => {
    alive.current = true;
    let disposed = false,
      timer = 0,
      lastStatus = "";
    const poll = async () => {
      try {
        const response = await invoke<ArrayBuffer>("vnc_poll", {
          id: live.id,
          cursor: cursorSerial.current,
        });
        if (disposed) return;
        const { status, tiles, cursor } = packet(response);
        if (status.id !== live.id) throw Error("Stale VNC framebuffer");
        current.current = status;
        const signature = JSON.stringify(status);
        if (signature !== lastStatus) {
          lastStatus = signature;
          setStatus(status);
          callbacks.current.onMeta(live.id, {
            protocol: "Vnc",
            endpoint: status.endpoint,
            status: status.status,
            connected: status.connected,
            finished: status.finished,
            trust: null,
            path: "",
            files_status: "",
            files: null,
          });
        }
        const el = canvas.current;
        if (el && status.width && status.height) {
          if (el.width !== status.width || el.height !== status.height) {
            el.width = status.width;
            el.height = status.height;
          }
          const context = el.getContext("2d", { alpha: false });
          if (!context) throw Error("VNC canvas unavailable");
          for (const tile of tiles)
            context.putImageData(
              new ImageData(tile.rgba, tile.width, tile.height),
              tile.x,
              tile.y,
            );
        }
        if (el) {
          const shape = status.cursor;
          if (!shape) {
            cursorSerial.current = 0;
            el.style.cursor = "";
          } else if (cursor) {
            // The server stops drawing its pointer once it sends a shape;
            // show that shape as the local cursor instead of a second arrow.
            cursorSerial.current = shape.serial;
            if (!shape.width || !shape.height) el.style.cursor = "none";
            else {
              const image = document.createElement("canvas");
              image.width = shape.width;
              image.height = shape.height;
              image
                .getContext("2d")
                ?.putImageData(
                  new ImageData(cursor, shape.width, shape.height),
                  0,
                  0,
                );
              el.style.cursor = `url(${image.toDataURL()}) ${shape.hotspot_x} ${shape.hotspot_y}, default`;
            }
          }
        }
        // A full batch means more of the frame is waiting: fetch it at once.
        timer = window.setTimeout(
          poll,
          tiles.length === 1024
            ? 0
            : shown.current && !status.finished
              ? 16
              : 200,
        );
      } catch (error) {
        if (!disposed) {
          callbacks.current.onError(error);
          void close();
        }
      }
    };
    void poll();
    const blur = () => release.current();
    window.addEventListener("blur", blur);
    const visibility = () => {
      if (document.hidden) release.current();
    };
    document.addEventListener("visibilitychange", visibility);
    // React has no keypress prop; it carries the composed character.
    const el = canvas.current;
    const press = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      sendAll(keyboard.current.press(e));
    };
    el?.addEventListener("keypress", press);
    const surface = el?.parentElement;
    const observer = new ResizeObserver(() => {
      const r = surface!.getBoundingClientRect();
      setBox({ width: r.width, height: r.height });
    });
    if (surface) observer.observe(surface);
    return () => {
      release.current();
      alive.current = false;
      disposed = true;
      window.clearTimeout(timer);
      window.removeEventListener("blur", blur);
      document.removeEventListener("visibilitychange", visibility);
      el?.removeEventListener("keypress", press);
      observer.disconnect();
    };
  }, [live.id]);
  useEffect(() => {
    if (!visible || !active) release.current();
    else if (!status?.prompt && status?.connected) canvas.current?.focus();
  }, [active, visible, focusRequest, !!status?.prompt, status?.connected]);
  useEffect(() => {
    if (
      !visible ||
      !status?.connected ||
      !status.resize_enabled ||
      !status.resize_supported
    )
      return;
    const surface = canvas.current?.parentElement;
    if (!surface) return;
    let timer = 0;
    const request = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        const r = surface.getBoundingClientRect();
        const width = Math.max(1, Math.min(2048, Math.floor(r.width))),
          height = Math.max(1, Math.min(2048, Math.floor(r.height)));
        if (
          width !== current.current?.width ||
          height !== current.current?.height
        )
          send.current({ kind: "resize", width, height });
      }, 250);
    };
    const observer = new ResizeObserver(request);
    observer.observe(surface);
    request();
    return () => {
      observer.disconnect();
      window.clearTimeout(timer);
    };
  }, [
    visible,
    status?.connected,
    status?.resize_enabled,
    status?.resize_supported,
  ]);
  useEffect(() => {
    if (!status?.connected) {
      setClipboardOpen(false);
      setClipboardText("");
    }
  }, [status?.connected]);
  const prompt = status?.prompt;
  // Scale to the pane in both directions, keeping the aspect ratio. Sizing the
  // element itself keeps pointer mapping exact (no letterbox inside it).
  const size =
    status?.width && status.height && box
      ? fit(status.width, status.height, box.width, box.height)
      : null;
  const cancel = () => {
    void close().catch(onError);
  };
  const mouse = (
    e: React.PointerEvent<HTMLCanvasElement>,
    buttons = e.buttons,
  ) => {
    if (!current.current?.connected) return;
    const p = pointer(
      e.clientX,
      e.clientY,
      e.currentTarget.getBoundingClientRect(),
      e.currentTarget.width,
      e.currentTarget.height,
    );
    if (!p) return;
    // DOM bit order is left/right/middle; RFB is left/middle/right.
    const mask = (buttons & 1) | ((buttons & 4) >> 1) | ((buttons & 2) << 1);
    send.current({ kind: "pointer", buttons: mask, ...p });
  };
  return (
    <div className="vnc-surface" data-vnc-id={live.id}>
      {visible && status?.connected && (
        <div className="vnc-actions">
          {status.clipboard_enabled && (
            <button
              onClick={() => {
                release.current();
                setClipboardOpen(true);
              }}
            >
              Send clipboard text
            </button>
          )}
          <small>
            {status.resize_status && status.resize_status !== 0
              ? "Server refused resize · scaled"
              : status.resize_enabled && status.resize_supported
                ? "Server resize enabled"
                : "Desktop scaled"}
          </small>
        </div>
      )}
      {visible && clipboardOpen && (
        <Modal
          title="Send clipboard text"
          onClose={() => {
            setClipboardOpen(false);
            setClipboardText("");
          }}
        >
          <p>
            Paste local clipboard text here, review it, then send. No keystrokes
            are generated. Server clipboard never replaces the Windows
            clipboard. Latin-1 only, up to 16 KiB.
          </p>
          <textarea
            aria-label="Clipboard text to send"
            rows={6}
            maxLength={16384}
            value={clipboardText}
            onChange={(e) => setClipboardText(e.target.value)}
          />
          {clipboardError(clipboardText) && (
            <p role="alert">{clipboardError(clipboardText)}</p>
          )}
          <div className="dialog-actions">
            <button
              onClick={() => {
                setClipboardOpen(false);
                setClipboardText("");
              }}
            >
              Cancel
            </button>
            <button
              disabled={!!clipboardError(clipboardText)}
              onClick={() => {
                send.current({ kind: "clipboard", text: clipboardText });
                setClipboardText("");
                setClipboardOpen(false);
              }}
            >
              Send text
            </button>
          </div>
        </Modal>
      )}
      <canvas
        ref={canvas}
        width={0}
        height={0}
        tabIndex={0}
        aria-label="VNC remote desktop"
        onFocus={onFocus}
        onBlur={() => release.current()}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
        }}
        style={size ? { width: size.width, height: size.height } : undefined}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (
            current.current?.clipboard_enabled &&
            e.ctrlKey &&
            !e.altKey &&
            !e.metaKey &&
            !e.shiftKey &&
            e.key.toLowerCase() === "v"
          ) {
            // Opted-in Ctrl+V: let the WebView fire paste with the Windows
            // clipboard, send it as ClientCutText, then Ctrl+V on the server.
            sendAll(keyboard.current.flush());
            if (!e.repeat) pasting.current = e.code;
            return;
          }
          const { inputs, prevent } = keyboard.current.down(e.nativeEvent);
          if (prevent) e.preventDefault();
          sendAll(inputs);
        }}
        onKeyUp={(e) => {
          e.preventDefault();
          e.stopPropagation();
          if (pasting.current === e.code) {
            // No paste event (empty clipboard): forward the plain shortcut.
            pasting.current = null;
            send.current({ kind: "key", down: true, keysym: 0x76 });
            send.current({ kind: "key", down: false, keysym: 0x76 });
          }
          sendAll(keyboard.current.up(e.nativeEvent));
        }}
        onPaste={(e) => {
          e.preventDefault();
          e.stopPropagation();
          if (!pasting.current) return;
          pasting.current = null;
          const text = e.clipboardData
            .getData("text/plain")
            .replace(/\r\n?/g, "\n");
          const error = text ? clipboardError(text) : null;
          if (error) {
            callbacks.current.onError(`Paste not sent: ${error}`);
            return;
          }
          if (text) send.current({ kind: "clipboard", text });
          send.current({ kind: "key", down: true, keysym: 0x76 });
          send.current({ kind: "key", down: false, keysym: 0x76 });
        }}
        onPointerDown={(e) => {
          e.preventDefault();
          onFocus();
          e.currentTarget.focus();
          e.currentTarget.setPointerCapture(e.pointerId);
          mouse(e);
        }}
        onPointerMove={(e) => mouse(e)}
        onPointerUp={(e) => {
          mouse(e);
          if (e.currentTarget.hasPointerCapture(e.pointerId))
            e.currentTarget.releasePointerCapture(e.pointerId);
        }}
        onPointerCancel={() => release.current()}
        onLostPointerCapture={(e) => {
          if (!e.currentTarget.matches(":focus")) release.current();
        }}
        onWheel={(e) => {
          e.preventDefault();
          const p = pointer(
            e.clientX,
            e.clientY,
            e.currentTarget.getBoundingClientRect(),
            e.currentTarget.width,
            e.currentTarget.height,
          );
          if (p) {
            send.current({
              kind: "pointer",
              buttons: e.deltaY < 0 ? 8 : 16,
              ...p,
            });
            send.current({ kind: "pointer", buttons: 0, ...p });
          }
        }}
      />
      {!status?.width && <span>{status?.status ?? "Opening VNC…"}</span>}
      {visible && prompt?.kind === "certificate" && (
        <Modal title="Verify VNC certificate" onClose={cancel}>
          <p>
            Verify this server independently before trusting it. No credentials
            have been sent.
          </p>
          <dl className="vnc-certificate">
            <dt>Server</dt>
            <dd>{status?.endpoint}</dd>
            <dt>Subject</dt>
            <dd>{prompt.subject}</dd>
            <dt>Issuer</dt>
            <dd>{prompt.issuer}</dd>
            <dt>Valid from</dt>
            <dd>{prompt.valid_from}</dd>
            <dt>Valid until</dt>
            <dd>{prompt.valid_until}</dd>
            <dt>SHA-256</dt>
            <dd>{prompt.sha256}</dd>
          </dl>
          <div className="dialog-actions">
            <button onClick={cancel}>Cancel</button>
            <button
              className="primary"
              disabled={busy}
              onClick={() => {
                setBusy(true);
                void invoke("vnc_decide", {
                  id: live.id,
                  challenge: status!.challenge,
                  approve: true,
                })
                  .catch(onError)
                  .finally(() => setBusy(false));
              }}
            >
              Trust this certificate
            </button>
          </div>
        </Modal>
      )}
      {visible && prompt?.kind === "credentials" && (
        <Modal title="VNC authentication" onClose={cancel}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              if (busy) return;
              setBusy(true);
              void invoke("vnc_authenticate", {
                id: live.id,
                challenge: status!.challenge,
              })
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            <p>
              {status?.endpoint} · Encrypted connection. The server has been
              verified.
            </p>
            <p>
              Continue with your saved password or sign in with the Windows
              Security dialog. Choose "Remember me" to save it on this computer.
            </p>
            {prompt.auth === "Vnc" && (
              <p className="hint">
                VNC authentication supports an ASCII password of at most eight
                bytes.
              </p>
            )}
            <div className="dialog-actions">
              <button type="button" onClick={cancel}>
                Cancel
              </button>
              <button type="submit" className="primary" disabled={busy}>
                Sign in
              </button>
            </div>
          </form>
        </Modal>
      )}
    </div>
  );
}
