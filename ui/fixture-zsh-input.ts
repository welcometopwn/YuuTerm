// Explicit external-host capture only. Synthetic keys through the real xterm
// input path and SSH transport; never change remote shell configuration files.
import { invoke } from "@tauri-apps/api/core";
import { fixtureTerminals } from "./fixture-terminals";

export async function checkZshInput(ids: number[]) {
  const checks: string[] = [];
  const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));
  try {
    await pause(2000);
    const host = document.querySelector<HTMLElement>(".terminal-host")!;
    const term = fixtureTerminals.get(host)!;
    const input = host.querySelector<HTMLTextAreaElement>("textarea")!;
    const line = () =>
      term.buffer.active
        .getLine(term.buffer.active.baseY + term.buffer.active.cursorY)
        ?.translateToString(false, 0, term.buffer.active.cursorX) ?? "";
    const key = async (key: string, keyCode: number, ctrlKey = false) => {
      for (const type of ["keydown", "keyup"])
        input.dispatchEvent(
          new KeyboardEvent(type, {
            key,
            keyCode,
            ctrlKey,
            bubbles: true,
            cancelable: true,
          }),
        );
      await pause(120);
    };
    input.focus();
    // Discard only this isolated session's automatic fixture probe, if present.
    await key("c", 67, true);
    await pause(800);
    for (const word of ["echo", "printf", "echo"]) {
      const before = line();
      for (const char of word)
        await key(char, char.toUpperCase().charCodeAt(0));
      if (line() !== before + word) {
        const sent = await invoke<number[][]>("capture_trace", { id: ids[0] });
        const tail = new TextDecoder()
          .decode(new Uint8Array(sent.flat()))
          .slice(-word.length);
        throw Error(
          `Typing duplicates/displaces characters; expected length ${before.length + word.length}, actual ${line().length}, input exactly once ${tail === word}`,
        );
      }
      checks.push(
        "Typed word appears exactly once at the configured zsh prompt",
      );
      for (let i = 0; i < word.length; i++) await key("Backspace", 8);
      if (line() !== before)
        throw Error(
          "Backspace leaves an undeletable character at the zsh prompt",
        );
      checks.push(
        "Backspace restores the exact prompt without a ghost character",
      );
    }
    const ownershipDeadline = Date.now() + 8000;
    while (
      !Array.from(document.querySelectorAll(".file-owner")).some(
        (entry) => entry.textContent?.trim() === "root / root",
      )
    ) {
      if (Date.now() > ownershipDeadline)
        throw Error("Real Linux SFTP listing did not resolve root / root");
      await pause(100);
    }
    checks.push("Real Linux listing resolves UID/GID 0/0 as root / root");
    await invoke("capture_report", { checks, passed: true });
    // Output from a real host is kept only in this ignored isolated capture.
    await invoke("capture_ready", { terminalReady: true });
  } catch (error) {
    checks.push("FAILED: " + String(error));
    await invoke("capture_report", { checks, passed: false });
  } finally {
    await invoke("session_disconnect", { id: ids[0] }).catch(() => {});
  }
}
