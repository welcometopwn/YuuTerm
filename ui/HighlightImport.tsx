import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Modal } from "./Modal";
import {
  reviewSyntax,
  type Rule,
  type SyntaxPreview as Preview,
} from "./highlight-import-policy";

export function HighlightImport({
  remaining,
  onAdd,
}: {
  remaining: number;
  onAdd: (rules: Rule[]) => void;
}) {
  const [preview, setPreview] = useState<Preview | null>(null),
    [busy, setBusy] = useState(false),
    [problem, setProblem] = useState("");
  const read = async () => {
    if (busy) return;
    setBusy(true);
    setProblem("");
    try {
      const value = await invoke<Preview | null>("highlight_import_preview");
      if (!value) return;
      setPreview(reviewSyntax(value));
    } catch (error) {
      setProblem(String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <button type="button" disabled={busy} onClick={() => void read()}>
        Import MobaXterm syntax
      </button>
      {problem && <p role="alert">{problem}</p>}
      {preview && (
        <Modal title="Review MobaXterm syntax" onClose={() => setPreview(null)}>
          <p>
            Add accepted rules after your current rules. Highlighting switches
            and IP colors stay as set. Nothing is saved until Save settings.
          </p>
          <p>
            {preview.accepted.length} accepted · {preview.rejected.length}{" "}
            rejected · {remaining} rule slots available
          </p>
          <div style={{ maxHeight: "45vh", overflow: "auto" }}>
            {preview.accepted.map((entry) => (
              <p key={entry.line}>
                <span style={{ color: entry.rule.color }}>
                  Line {entry.line} · {entry.source} · {entry.rule.color}
                </span>{" "}
                <code>{entry.rule.keywords}</code>
              </p>
            ))}
            {preview.rejected.map((entry, i) => (
              <p key={i}>
                Line {entry.line}: {entry.reason}
              </p>
            ))}
          </div>
          <p className="hint">
            Case-insensitive regex exports only. Unsupported modes/effects are
            listed. Color names map to the displayed RGB values; source palettes
            are not imported.
          </p>
          {preview.accepted.length > remaining && (
            <p role="alert">
              Remove rules from this draft first: the limit is 12. No rules will
              be silently dropped.
            </p>
          )}
          <div className="dialog-actions">
            <button onClick={() => setPreview(null)}>Cancel import</button>
            <button
              className="primary"
              disabled={
                !preview.accepted.length || preview.accepted.length > remaining
              }
              onClick={() => {
                onAdd(preview.accepted.map((e) => e.rule));
                setPreview(null);
              }}
            >
              Add to draft
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}
