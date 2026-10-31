// Capture scope "local-terminal": a Windows PowerShell bookmark started in
// C:\Windows through the real pseudo console. Typed input runs, the pane
// resizes the console, and "exit 7" ends the session with its exit code.
import { invoke } from "@tauri-apps/api/core";
import { terminalText } from "./fixture-terminals";

export async function checkLocalTerminal(ids: number[]) {
  const checks: string[] = [];
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  // Read from the buffer: the WebGL renderer has no DOM text rows.
  const text = () => terminalText();
  const until = async (test: () => boolean, name: string, ms = 20000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      if (test()) {
        checks.push(name);
        return;
      }
      await sleep(200);
    }
    throw Error(name);
  };
  const type = (s: string) =>
    invoke("session_input", {
      id: ids[0],
      bytes: [...new TextEncoder().encode(s)],
    });
  try {
    await until(
      () => text().includes("PS C:\\Windows>"),
      "Windows PowerShell starts in the bookmark's directory",
    );
    // The shared fixture probe types "ui-stream-check" first; start clean.
    await type("Clear-Host\r");
    await sleep(800);
    await type("Write-Output ('yuuterm' + '-local-' + (6*7))\r");
    await until(
      () => text().includes("yuuterm-local-42"),
      "Typed input runs and its output reaches the pane",
    );
    await type("$Host.UI.RawUI.WindowSize.Width\r");
    await until(() => {
      const width = Number(
        text()
          .split("\n")
          .map((l) => l.trim())
          .find((l) => /^\d{2,3}$/.test(l)),
      );
      return width >= 40;
    }, "The console follows the pane size (width reported by the shell)");
    await sleep(300);
    await type("Clear-Host; Write-Host 'YuuTerm 0.2.0' -ForegroundColor Cyan; Write-Host 'Windows remote workspace' -ForegroundColor White; Write-Host ''; Write-Host 'SSH / SFTP    RDP    VNC    TELNET    SERIAL'; Write-Host 'Tabs, split panes, file transfers and X11 forwarding'; Write-Host ''; Write-Host 'Local terminal demonstration' -ForegroundColor DarkGray\r");
    await until(() => text().includes("Local terminal demonstration"), "The local demonstration renders");
    await sleep(700);
    await invoke("capture_still", { name: "local-terminal" });
    await sleep(1500);
    if (
      document.querySelector<HTMLElement>(".terminal-host")?.dataset
        .renderer !== "webgl"
    )
      throw Error("The terminal did not use the WebGL renderer");
    checks.push("The pane renders with WebGL");
    await type("exit 7\r");
    await until(
      () =>
        !!document
          .querySelector(".pane")
          ?.textContent?.includes("exited with code 7"),
      "Exit ends the session and shows the shell's exit code",
    );
    // Reconnecting must not surface errors from the previous connection.
    const button = (label: string) =>
      [...document.querySelectorAll<HTMLButtonElement>("button")].find(
        (b) =>
          b.getAttribute("aria-label") === label ||
          b.textContent?.trim() === label,
      );
    const connected = () =>
      !!document.querySelector(".pane .dot.connected") ||
      !!document.querySelector(".session-row .dot.connected");
    const notice = () =>
      document.querySelector(".app-notice")?.textContent ?? "";
    for (const round of [1, 2, 3]) {
      button("Reconnect")!.click();
      await until(connected, `Reconnect ${round} connects again`);
      await sleep(1500);
      button("Disconnect")!.click();
      await until(() => !!button("Reconnect"), `Disconnect ${round} ends it`);
    }
    await sleep(1000);
    if (/no longer exists/i.test(notice()))
      throw Error(`Reconnect showed an error: ${notice()}`);
    checks.push("Reconnect and disconnect show no stale-connection error");
    // The sidebar button opens an unsaved PowerShell 7 in one click.
    button("Open local terminal")!.click();
    const localTab = () =>
      [...document.querySelectorAll(".tab")].find((t) =>
        t.textContent?.includes("Local terminal"),
      );
    await until(() => !!localTab(), "The local terminal button opens a tab");
    await until(
      () =>
        !!document.querySelector(
          ".pane:not([hidden]) .dot.connected, .session-row .dot.connected",
        ) && text().includes("PS "),
      "It starts a PowerShell prompt",
    );
    await until(
      () =>
        [...document.querySelectorAll(".session-row")].some((r) =>
          r.textContent?.includes("Local terminal"),
        ),
      "The unsaved local terminal is listed with the quick sessions",
    );
    // A local session needs no host to be saved from the editor.
    button("New session")!.click();
    await until(
      () => !!document.querySelector("dialog[open] select"),
      "The session editor opens",
    );
    const protocol = [
      ...document.querySelectorAll<HTMLSelectElement>("dialog[open] select"),
    ].find((s) => [...s.options].some((o) => o.value === "Local"))!;
    protocol.value = "Local";
    protocol.dispatchEvent(new Event("change", { bubbles: true }));
    const name = document.querySelector<HTMLInputElement>(
      'dialog[open] input[placeholder="A name you recognize"]',
    )!;
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!;
    setter.call(name, "Saved shell");
    name.dispatchEvent(new Event("input", { bubbles: true }));
    await until(
      () =>
        [
          ...document.querySelectorAll<HTMLButtonElement>(
            "dialog[open] button",
          ),
        ].some((b) => b.textContent?.trim() === "Save session" && !b.disabled),
      "Save session is enabled for a local terminal without a host",
    );
    button("Cancel")!.click();
    await invoke("capture_report", { checks, passed: true });
  } catch (error) {
    checks.push(`FAILED: ${String(error)}`);
    checks.push(`Pane text: ${text().slice(-300)}`);
    await invoke("capture_report", { checks, passed: false });
  }
}
