import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Monitor } from "lucide-react";
import type { Live, Meta } from "./types";
import { statusParts } from "./status-text";

function RdpStatus({ text }: { text: string }) {
  const { lead, reasons, advice, code } = statusParts(text);
  if (!reasons.length) return <span>{code ? `${lead} (${code})` : lead}</span>;
  return (
    <div className="rdp-status">
      <p>{lead}:</p>
      <ol>
        {reasons.map((r) => (
          <li key={r}>{r}</li>
        ))}
      </ol>
      {advice && <p>{advice}</p>}
      {code && <small>{code}</small>}
    </div>
  );
}

interface Status {
  id: number;
  connected: boolean;
  finished: boolean;
  status: string;
  endpoint: string;
  visible: boolean;
}
// WebView overlays that the native child window would otherwise cover. Modal
// dialogs always hide it; menus only when they actually overlap the pane.
const covered = (pane: DOMRect) => {
  if (document.querySelector("dialog[open]")) return true;
  const menus = [
    ...document.querySelectorAll<HTMLElement>(".context-menu"),
    ...document.querySelectorAll<HTMLElement>("details[open] > :not(summary)"),
  ];
  return menus.some((menu) => {
    const r = menu.getBoundingClientRect();
    return (
      r.width > 0 &&
      r.height > 0 &&
      r.left < pane.right &&
      r.right > pane.left &&
      r.top < pane.bottom &&
      r.bottom > pane.top
    );
  });
};

// Hosts the native Microsoft RDP control over this pane's rectangle. The control
// is a real child window above the WebView, which cannot draw over it. While a
// WebView overlay covers the pane, a still image of the remote desktop replaces
// the hidden control, so the desktop does not visibly disappear. Server-
// certificate and credential prompts are native.
export function RdpPane({
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
  onError: (e: unknown) => void;
  onFocus: () => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const liveId = live.id;
  const shown = useRef("");
  const started = useRef(false);
  const finished = useRef(false);
  const [still, setStill] = useState<string | null>(null);
  const stillRef = useRef<string | null>(null);
  const showStill = (url: string | null) => {
    if (stillRef.current) URL.revokeObjectURL(stillRef.current);
    stillRef.current = url;
    setStill(url);
  };
  const connected = useRef(false);
  const visibleRef = useRef(visible);
  visibleRef.current = visible;
  const meta = useRef(onMeta);
  meta.current = onMeta;
  const call = (action: string, bounds?: number[]) =>
    invoke<Status | null>("rdp_session", {
      id: live.id,
      action,
      bounds: bounds ?? null,
      scale: Math.round(window.devicePixelRatio * 100),
    });
  const report = (s: Status | null) => {
    if (!s) return;
    finished.current = s.finished;
    connected.current = s.connected;
    meta.current(live.id, {
      protocol: "Rdp",
      endpoint: s.endpoint,
      status: s.status,
      connected: s.connected,
      finished: s.finished,
      trust: null,
      path: "",
      files_status: "",
      files: null,
    });
  };
  const sync = useRef<() => void>(() => {});
  sync.current = () => {
    const el = box.current;
    if (!el) return;
    const r = el.getBoundingClientRect(),
      scale = window.devicePixelRatio;
    const bounds = [r.x, r.y, r.width, r.height].map((v) =>
      Math.round(v * scale),
    );
    const live = visibleRef.current && !finished.current;
    const overlay = live && covered(r);
    const show = live && !overlay && bounds[2] >= 64 && bounds[3] >= 64;
    const key = show ? bounds.join(",") : "hidden";
    if (key === shown.current) return;
    shown.current = key;
    if (!show) {
      if (!overlay || !connected.current) {
        showStill(null);
        void call("hide").catch(onError);
        return;
      }
      // Freeze: paint the still first, then hide the native window under it.
      void invoke<ArrayBuffer>("rdp_snapshot", { id: liveId })
        .then(
          (jpeg) =>
            new Promise<void>((resolve) => {
              showStill(
                URL.createObjectURL(new Blob([jpeg], { type: "image/jpeg" })),
              );
              requestAnimationFrame(() =>
                requestAnimationFrame(() => resolve()),
              );
            }),
        )
        .catch(() => {})
        .finally(() => {
          if (shown.current === "hidden") void call("hide").catch(onError);
        });
      return;
    }
    void call("place", bounds)
      .then(async (s) => {
        report(s);
        showStill(null);
        if (!started.current) {
          started.current = true;
          report(await call("connect"));
        }
      })
      .catch((e) => {
        shown.current = "";
        onError(e);
      });
  };
  useEffect(() => {
    const update = () => sync.current();
    const resize = new ResizeObserver(update);
    if (box.current) resize.observe(box.current);
    const overlays = new MutationObserver(update);
    overlays.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ["open", "class", "style"],
    });
    window.addEventListener("resize", update);
    const poll = window.setInterval(() => {
      if (!started.current) return;
      void call("status")
        .then((s) => {
          const wasFinished = finished.current;
          report(s);
          if (finished.current !== wasFinished) update();
        })
        .catch(() => {});
    }, 700);
    update();
    return () => {
      showStill(null);
      resize.disconnect();
      overlays.disconnect();
      window.removeEventListener("resize", update);
      window.clearInterval(poll);
    };
  }, [live.id]);
  useEffect(() => sync.current(), [visible]);
  useEffect(() => {
    if (active && visible && started.current && !finished.current)
      void call("focus").catch(() => {});
  }, [active, visible, focusRequest]);
  return (
    <div
      ref={box}
      className="rdp-surface"
      data-rdp-id={live.id}
      onMouseDown={onFocus}
    >
      {still ? (
        <img className="rdp-still" src={still} alt="" />
      ) : (
        <>
          <Monitor size={28} />
          <RdpStatus text={live.meta?.status ?? "Opening remote desktop…"} />
        </>
      )}
    </div>
  );
}
