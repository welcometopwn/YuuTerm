import { useEffect, useRef, useState, type MutableRefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TelnetLineDraft } from "./telnet-line-policy";
export function TelnetLineInput({
  id,
  disabled,
  onError,
  appendRef,
}: {
  id: number;
  disabled: boolean;
  onError: (message: string) => void;
  appendRef: MutableRefObject<(text: string) => void>;
}) {
  const model = useRef(new TelnetLineDraft());
  const [, paint] = useState(0);
  const render = () => paint((n) => n + 1);
  const [problem, setProblem] = useState("");
  const append = (text: string) => {
    setProblem(model.current.append(text) ?? "");
    render();
  };
  useEffect(() => {
    appendRef.current = append;
    return () => {
      appendRef.current = () => {};
    };
  });
  const submit = async () => {
    if (disabled || model.current.busy) return;
    const work = model.current.submit((text) =>
      invoke("session_input", {
        id,
        bytes: Array.from(new TextEncoder().encode(text)),
      }),
    );
    render();
    try {
      await work;
      setProblem("");
    } catch {
      onError(
        "Line submission was not confirmed; draft retained. Check the remote terminal before retrying.",
      );
    } finally {
      render();
    }
  };
  return (
    <div
      className="telnet-line-editor"
      onMouseDown={(e) => e.stopPropagation()}
    >
      <label>
        Local line (Enter sends; Escape discards)
        <input
          aria-label="TELNET local line"
          autoComplete="off"
          spellCheck={false}
          maxLength={4096}
          disabled={disabled || model.current.busy}
          value={model.current.text}
          onChange={(e) => {
            setProblem(model.current.change(e.target.value) ?? "");
            render();
          }}
          onPaste={(e) => {
            e.preventDefault();
            e.stopPropagation();
            append(e.clipboardData.getData("text/plain"));
          }}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing) return;
            if (e.key === "Enter") {
              e.preventDefault();
              e.stopPropagation();
              if (!e.repeat) void submit();
            } else if (e.key === "Escape") {
              e.preventDefault();
              e.stopPropagation();
              model.current.clear();
              setProblem("");
              render();
            }
          }}
        />
      </label>
      <button
        disabled={disabled || model.current.busy}
        onClick={() => void submit()}
      >
        Send line
      </button>
      {problem && <p role="alert">{problem}</p>}
    </div>
  );
}
