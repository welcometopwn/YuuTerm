import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Modal } from "./Modal";
import { protocolLabel, type Live, type Profile } from "./types";

type Target = {
  id: number;
  name: string;
  endpoint: string;
  protocol: Live["protocol"];
};
type Review = { targets: Target[]; text: string; appendEnter: boolean };
type Result = { id: number; queued: boolean };

export function Broadcast({
  lives,
  profiles,
  onClose,
}: {
  lives: Live[];
  profiles: Profile[];
  onClose: () => void;
}) {
  const candidates: Target[] = lives
    .filter(
      (l) =>
        l.meta?.connected &&
        !l.meta.finished &&
        ["Ssh", "Telnet", "Serial"].includes(l.protocol),
    )
    .map((l) => ({
      id: l.id,
      name: profiles.find((p) => p.id === l.profileId)?.name ?? "Terminal",
      endpoint: l.meta!.endpoint,
      protocol: l.protocol,
    }));
  const [selected, setSelected] = useState<number[]>([]);
  const [text, setText] = useState("");
  const [appendEnter, setAppendEnter] = useState(false);
  const [review, setReview] = useState<Review | null>(null);
  const [busy, setBusy] = useState(false);
  const sending = useRef(false);
  const [problem, setProblem] = useState("");
  const [done, setDone] = useState<{
    targets: Target[];
    results: Result[];
  } | null>(null);
  const available = new Set(candidates.map((t) => t.id));
  // Reconnection creates a new live ID. Filtering never adds that replacement.
  useEffect(() => {
    setSelected((previous) =>
      previous.some((id) => !available.has(id))
        ? previous.filter((id) => available.has(id))
        : previous,
    );
  }, [lives]);
  const stale = review?.targets.some((t) => !available.has(t.id)) ?? false;
  const bytes = new TextEncoder().encode(text).length + Number(appendEnter);
  const validText =
    text.trim().length > 0 &&
    bytes <= 16384 &&
    !/[\x00-\x08\x0b-\x1f\x7f-\x9f]/.test(text);
  const send = async () => {
    if (!review || stale || sending.current) return;
    sending.current = true;
    setBusy(true);
    setProblem("");
    try {
      const results = await invoke<Result[]>("terminal_dispatch", {
        ids: review.targets.map((t) => t.id),
        text: review.text,
        appendEnter: review.appendEnter,
      });
      setDone({ targets: review.targets, results });
      setSelected([]);
      setText("");
      setReview(null);
    } catch (e) {
      setProblem(
        `${String(e)}. Check each terminal before retrying; an unavailable response can leave delivery uncertain.`,
      );
    } finally {
      setBusy(false);
      sending.current = false;
    }
  };
  return (
    <Modal
      title={
        done
          ? "Dispatch result"
          : review
            ? "Review terminal dispatch"
            : "Send to terminals"
      }
      onClose={busy ? undefined : onClose}
    >
      {done ? (
        <>
          <p className="hint">
            Queued means accepted for sending, not proof of remote execution.
            Check each terminal before retrying a partial result.
          </p>
          <ul className="dispatch-results">
            {done.targets.map((t) => (
              <li key={t.id}>
                <strong>{t.name}</strong>
                <span>
                  {done.results.find((r) => r.id === t.id)?.queued
                    ? "Queued"
                    : "Not queued — disconnected or busy"}
                </span>
              </li>
            ))}
          </ul>
          <div className="dialog-actions">
            <button onClick={onClose}>Close</button>
          </div>
        </>
      ) : review ? (
        <>
          <p>
            Send once to these <strong>{review.targets.length}</strong> live
            terminals:
          </p>
          <ul className="dispatch-results">
            {review.targets.map((t) => (
              <li key={t.id}>
                <strong>{t.name}</strong>
                <span>
                  {protocolLabel(t.protocol)} · {t.endpoint}
                </span>
              </li>
            ))}
          </ul>
          <pre className="dispatch-preview">
            {review.text.replaceAll("\n", "↵\n")}
            {review.appendEnter ? "↵" : ""}
          </pre>
          <p className="warning">
            {review.appendEnter || review.text.includes("\n")
              ? "Enter/newlines can execute commands in every selected session. Existing terminal input is retained."
              : "No final Enter is appended. Existing input in each terminal is retained."}
          </p>
          {review.targets.some((t) => t.protocol === "Telnet") && (
            <p className="warning">
              Selected TELNET connections transmit this text without encryption.
            </p>
          )}
          {stale && (
            <p role="alert" className="warning">
              A selected connection ended. Go back and review the targets again;
              replacements are not selected.
            </p>
          )}
          {problem && (
            <p role="alert" className="warning">
              {problem}
            </p>
          )}
          <div className="dialog-actions">
            <button
              disabled={busy}
              onClick={() => {
                setReview(null);
                setProblem("");
              }}
            >
              Back
            </button>
            <button
              className="primary"
              disabled={busy || stale || !!problem}
              onClick={() => void send()}
            >
              {busy
                ? "Sending…"
                : `Send to ${review.targets.length} terminal${review.targets.length === 1 ? "" : "s"}`}
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="hint">
            Select live connections for a single dispatch. New or reconnected
            sessions are never added automatically.
          </p>
          <div className="dispatch-targets">
            {candidates.map((t) => (
              <label key={t.id}>
                <input
                  type="checkbox"
                  aria-label={`Select ${t.name}`}
                  checked={selected.includes(t.id)}
                  onChange={(e) =>
                    setSelected((ids) =>
                      e.target.checked
                        ? [...ids, t.id]
                        : ids.filter((id) => id !== t.id),
                    )
                  }
                />
                <span>
                  <strong>{t.name}</strong>
                  <small>
                    {protocolLabel(t.protocol)} · {t.endpoint}
                  </small>
                </span>
              </label>
            ))}
          </div>
          {!candidates.length && <p>No connected terminals available.</p>}
          <div className="dispatch-selection">
            <button
              disabled={candidates.length === 0 || candidates.length > 64}
              onClick={() => setSelected(candidates.map((t) => t.id))}
            >
              Select connected terminals
            </button>
            <button onClick={() => setSelected([])}>Clear selection</button>
            <span>{selected.length} selected</span>
          </div>
          <label>
            Text to send
            <textarea
              aria-label="Text to send"
              rows={4}
              value={text}
              maxLength={16384}
              onChange={(e) => setText(e.target.value)}
              spellCheck={false}
            />
          </label>
          <label className="dispatch-enter">
            <input
              type="checkbox"
              checked={appendEnter}
              onChange={(e) => setAppendEnter(e.target.checked)}
            />
            Append Enter
          </label>
          <small>{bytes} / 16384 bytes · up to 64 targets per dispatch</small>
          {!validText && text.length > 0 && (
            <p className="warning">
              Use nonempty text within 16 KiB. Escape and other terminal control
              characters are not allowed.
            </p>
          )}
          <div className="dialog-actions">
            <button onClick={onClose}>Cancel</button>
            <button
              className="primary"
              disabled={
                !validText || selected.length === 0 || selected.length > 64
              }
              onClick={() =>
                setReview({
                  targets: candidates.filter((t) => selected.includes(t.id)),
                  text,
                  appendEnter,
                })
              }
            >
              Review dispatch
            </button>
          </div>
        </>
      )}
    </Modal>
  );
}
