// Synthetic DOM checks inside the actual WebView2 app, only on the isolated fixture.
// These do not establish physical Windows keyboard/clipboard or AltGr/IME behavior.
import { invoke } from "@tauri-apps/api/core";
import type { Preferences, Bootstrap } from "./types";
import { fixtureTerminals } from "./fixture-terminals";
export async function checkTelnetFixture(
  ids: number[],
  applyPreferences: (prefs: Preferences) => void,
) {
  const checks: string[] = [];
  const until = async (
    predicate: () => boolean | Promise<boolean>,
    name: string,
  ) => {
    const end = Date.now() + 6000;
    while (Date.now() < end) {
      if (await predicate()) {
        checks.push(name);
        return;
      }
      await new Promise((r) => setTimeout(r, 40));
    }
    throw new Error(name);
  };
  try {
    await until(
      () =>
        document.querySelectorAll(".terminal-host .xterm-helper-textarea")
          .length === 4,
      "Four genuine TCP TELNET terminal renderers are mounted",
    );
    await until(
      () => document.querySelectorAll(".plaintext-warning").length === 4,
      "Every TELNET pane retains its unencrypted-connection warning",
    );
    await until(async () => {
      for (const id of ids) {
        const trace = await invoke<number[][]>("capture_trace", { id });
        if (
          !new TextDecoder()
            .decode(new Uint8Array(trace.flat()))
            .includes("ui-stream-check")
        )
          return false;
      }
      return true;
    }, "Fixture input traverses all four independent TELNET sessions");
    const otherCounts = await Promise.all(
      ids
        .slice(1)
        .map((id) =>
          invoke<number[][]>("capture_trace", { id }).then((v) => v.length),
        ),
    );
    const terminal = document.querySelector<HTMLTextAreaElement>(
      ".pane.focused .xterm-helper-textarea",
    )!;
    terminal.focus();
    terminal.value = "Grüße € @";
    terminal.dispatchEvent(
      new InputEvent("input", {
        data: "Grüße € @",
        inputType: "insertText",
        bubbles: true,
      }),
    );
    await until(async () => {
      const trace = await invoke<number[][]>("capture_trace", { id: ids[0] });
      return new TextDecoder()
        .decode(new Uint8Array(trace.flat()))
        .includes("Grüße € @");
    }, "Committed Unicode text reaches the focused TELNET input path (synthetic event)");
    const after = await Promise.all(
      ids
        .slice(1)
        .map((id) =>
          invoke<number[][]>("capture_trace", { id }).then((v) => v.length),
        ),
    );
    if (after.some((v, i) => v !== otherCounts[i]))
      throw new Error("TELNET input leaked to another pane");
    checks.push("Focused TELNET input does not broadcast");
    for (const name of ["Single pane", "Two panes", "Four panes"]) {
      document.querySelector<HTMLElement>(".view-menu summary")!.click();
      Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
        .find((b) => b.textContent?.trim() === name)!
        .click();
      await until(
        () =>
          !!document.querySelector(
            `.workspace.layout-${({ "Single pane": 1, "Two panes": 2, "Four panes": 4 } as Record<string, number>)[name]}`,
          ),
        `${name} retains existing TELNET connections`,
      );
    }
    if (document.querySelector('dialog[aria-label="Verify SSH host key"]'))
      throw new Error("TELNET requested SSH trust");
    checks.push("TELNET never asks for an SSH host key");
    document
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Show remote files"]',
      )!
      .click();
    await until(
      () =>
        !!Array.from(document.querySelectorAll(".side-empty")).find((e) =>
          e.textContent?.includes("SSH sessions"),
        ),
      "TELNET shows an honest unavailable-file-browser state",
    );
    if (
      await invoke<number>("session_start", { profileId: 1 }).then(
        () => true,
        () => false,
      )
    )
      throw new Error("Duplicate TELNET connection accepted");
    checks.push("Duplicate bookmark connection is refused");
    applyPreferences(
      await invoke<Preferences>("capture_bookmark_protocol", {
        profileId: 1,
        protocol: "Ssh",
      }),
    );
    await until(() => {
      const pane = document.querySelector<HTMLElement>(
        '.pane[data-profile-protocol="Ssh"]',
      );
      return (
        pane?.dataset.liveProtocol === "Telnet" &&
        !!pane.querySelector(".plaintext-warning")
      );
    }, "Editing the bookmark to SSH cannot hide a running TELNET connection warning");
    await until(
      () =>
        !!Array.from(document.querySelectorAll(".side-empty")).find((e) =>
          e.textContent?.includes("SSH sessions"),
        ),
      "File actions stay unavailable for the live TELNET transport after a bookmark edit",
    );
    applyPreferences(
      await invoke<Preferences>("capture_bookmark_protocol", {
        profileId: 1,
        protocol: "Telnet",
      }),
    );
    await until(
      () => !document.querySelector('.pane[data-profile-protocol="Ssh"]'),
      "Fixture metadata restored without reconnecting",
    );
    await invoke("capture_report", { checks, passed: true });
    document.documentElement.dataset.fixtureChecks = "passed";
  } catch (error) {
    checks.push("FAILED: " + String(error));
    await invoke("capture_report", { checks, passed: false });
  }
}
export async function checkFixture(ids: number[]) {
  const checks: string[] = [];
  const pause = () => new Promise<void>((r) => setTimeout(r, 40));
  async function until(
    predicate: () => boolean | Promise<boolean>,
    label: string,
  ) {
    const end = Date.now() + 6000;
    while (Date.now() < end) {
      if (await predicate()) {
        checks.push(label);
        return;
      }
      await pause();
    }
    throw new Error(label);
  }
  const button = (text: string) =>
    Array.from(document.querySelectorAll<HTMLButtonElement>("button")).find(
      (b) => b.textContent?.trim() === text,
    );
  const trace = () => invoke<number[][]>("capture_trace", { id: ids[0] });
  const contains = async (text: string) => {
    const value = new TextDecoder().decode(
      new Uint8Array((await trace()).flat()),
    );
    return value.includes(text);
  };
  function paste(target: HTMLElement, text: string) {
    const clipboardData = new DataTransfer();
    clipboardData.setData("text/plain", text);
    const event = new ClipboardEvent("paste", {
      clipboardData,
      bubbles: true,
      cancelable: true,
    });
    target.dispatchEvent(event);
    return event.defaultPrevented;
  }
  try {
    const scope = await invoke<string>("capture_scope");
    if (scope === "telnet-external") {
      const { checkTelnetExternal } = await import("./fixture-telnet-external");
      await checkTelnetExternal(ids, checks);
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      );
      await invoke("capture_ready", { terminalReady: true });
      return;
    }
    if (scope === "file-navigation") {
      const { checkFileNavigation } = await import("./fixture-file-navigation");
      await checkFileNavigation(ids, until, checks);
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      await invoke("capture_ready", { terminalReady: true });
      return;
    }
    if (
      scope === "windows-openssh" ||
      scope === "linux-openssh" ||
      scope === "linux-ssh-tools"
    ) {
      if (scope === "linux-ssh-tools") {
        const { checkSshTools } = await import("./fixture-ssh-tools");
        try {
          await checkSshTools(ids, checks);
        } catch (e) {
          // Report the failing step and end the run instead of idling to the
          // runner deadline.
          // capture_report refuses entries over 400 characters.
          checks.push(("FAILED: " + String(e)).slice(0, 390));
          await invoke("capture_report", { checks, passed: false });
          return;
        }
      } else if (scope === "windows-openssh") {
        const { checkWindowsOpenSsh } =
          await import("./fixture-windows-openssh");
        await checkWindowsOpenSsh(ids, checks);
      } else {
        const { checkLinuxOpenSsh } = await import("./fixture-linux-openssh");
        await checkLinuxOpenSsh(ids, checks);
      }
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      );
      await invoke("capture_ready", { terminalReady: true });
      return;
    }
    if (scope === "restart-prepare" || scope === "restart-restore") {
      const { checkRestart } = await import("./fixture-restart-checks");
      await checkRestart(ids, until, checks, scope === "restart-prepare");
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      return;
    }
    if (scope === "qol") {
      const { checkQol } = await import("./fixture-qol-checks");
      await checkQol(ids, until, checks);
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      return;
    }
    if (scope === "icons") {
      const { checkIcons } = await import("./fixture-icons-checks");
      await checkIcons(ids, until, checks);
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      return;
    }
    if (scope === "pollwait") {
      const { checkPollWait } = await import("./fixture-pollwait-checks");
      await checkPollWait(ids, checks);
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      return;
    }
    if (scope === "performance") {
      const { measureWorkspace } = await import("./fixture-performance");
      await measureWorkspace(ids);
      return;
    }
    if (scope === "transfer") {
      const { checkTransfer } = await import("./fixture-transfer-checks");
      await checkTransfer(ids, until, checks);
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      return;
    }
    if (scope === "logging") {
      const { checkLogging } = await import("./fixture-logging-checks");
      await checkLogging(ids, until, checks);
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      return;
    }
    if (scope === "history") {
      const { checkHistory } = await import("./fixture-history-checks");
      await checkHistory(ids, until, checks);
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      return;
    }
    if (scope === "filemenus") {
      const { checkFileMenus } = await import("./fixture-file-menu-checks");
      await checkFileMenus(ids, until, checks);
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      return;
    }
    if (scope === "recovery") {
      const { checkRecovery } = await import("./fixture-recovery-checks");
      await checkRecovery(ids, until, checks);
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      return;
    }
    if (scope === "editor") {
      const { checkEditor } = await import("./fixture-editor-checks");
      await checkEditor(ids, checks);
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      return;
    }
    if (scope === "files") {
      const { checkFileWorkflow } = await import("./fixture-file-checks");
      await checkFileWorkflow(ids, until, checks);
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      return;
    }
    if (scope === "session") {
      await until(
        () =>
          document.querySelectorAll(".terminal-host .xterm-rows").length ===
            4 && document.querySelectorAll(".stat-memory strong").length === 4,
        "Four isolated live SSH panes and RAM bars are ready",
      );
      await until(() => {
        const cell = document.querySelector<HTMLElement>(".stat-memory")!;
        const text = cell.querySelector("strong")!;
        return (
          /3[.,]00 GiB \/ 4[.,]00 GiB/.test(text.textContent ?? "") &&
          Math.abs(
            cell.getBoundingClientRect().width -
              text.getBoundingClientRect().width -
              36,
          ) < 2 &&
          Math.abs(
            text.getBoundingClientRect().left -
              cell.querySelector("svg")!.getBoundingClientRect().right -
              5,
          ) < 1
        );
      }, "RAM uses two decimal unit-labelled values and only the normal icon gap");
      document
        .querySelectorAll<HTMLButtonElement>(".side-tabs button")[0]
        .click();
      await pause();
      await checkSessionTabs(until);
      const { checkX11Settings } = await import("./fixture-x11-checks");
      await checkX11Settings(until, checks);
      const { checkWorkspaceActions } =
        await import("./fixture-workspace-actions");
      await checkWorkspaceActions(until, checks);
      const { checkSshPasswordEditor } = await import("./fixture-ssh-password");
      await checkSshPasswordEditor(until, checks);
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      return;
    }

    await until(
      () =>
        document.querySelectorAll(".terminal-host .xterm-helper-textarea")
          .length === 4,
      "Four real terminal renderers mounted",
    );
    await until(async () => {
      for (const id of ids) {
        const values = await invoke<number[][]>("capture_trace", { id });
        if (
          !new TextDecoder()
            .decode(new Uint8Array(values.flat()))
            .includes("ui-stream-check")
        )
          return false;
      }
      return true;
    }, "All four fixture input probes reach their own live sessions");
    const otherCounts = await Promise.all(
      ids
        .slice(1)
        .map((id) =>
          invoke<number[][]>("capture_trace", { id }).then(
            (values) => values.length,
          ),
        ),
    );
    const terminal = document.querySelector<HTMLTextAreaElement>(
      ".pane.focused .xterm-helper-textarea",
    )!;
    const path = document.querySelector<HTMLInputElement>(
      'input[aria-label="Remote path"]',
    )!;
    path.focus();
    const before = (await trace()).length;
    const event = new KeyboardEvent("keydown", {
      key: "v",
      code: "KeyV",
      keyCode: 86,
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    path.dispatchEvent(event);
    if (event.defaultPrevented || paste(path, "/fixture/copied-path"))
      throw new Error("Path editing was intercepted");
    await pause();
    if ((await trace()).length !== before)
      throw new Error("Path clipboard bytes leaked to terminal");
    if (document.activeElement !== path)
      throw new Error("Path focus was stolen by another control");
    checks.push(
      "Path Ctrl+V/paste events remain local; focus is retained (synthetic DOM)",
    );
    terminal.focus();
    // Ctrl+V must reach WebView2's native paste event, never SSH's quote-next (^V).
    const pasteKey = new KeyboardEvent("keydown", {
      key: "v",
      code: "KeyV",
      keyCode: 86,
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    terminal.dispatchEvent(pasteKey);
    if (pasteKey.defaultPrevented)
      throw Error("Ctrl+V was swallowed before native paste");
    await pause();
    if ((await trace()).flat().includes(22))
      throw Error("Ctrl+V sent quote-next to SSH");
    checks.push(
      "Ctrl+V allows native clipboard paste without sending quote-next to SSH",
    );
    const host = terminal.closest<HTMLElement>(".terminal-host")!;
    const renderer = fixtureTerminals.get(host)!;
    renderer.select(0, 0, 7);
    const copyKey = new KeyboardEvent("keydown", {
      key: "c",
      code: "KeyC",
      keyCode: 67,
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    const beforeCopy = (await trace()).flat().length;
    terminal.dispatchEvent(copyKey);
    if (copyKey.defaultPrevented)
      throw Error("Selected Ctrl+C was swallowed before native copy");
    const selected = renderer.getSelection();
    const copied = new DataTransfer();
    terminal.dispatchEvent(
      new ClipboardEvent("copy", {
        clipboardData: copied,
        bubbles: true,
        cancelable: true,
      }),
    );
    if (!selected || copied.getData("text/plain") !== selected)
      throw Error("Native copy event lost the terminal selection");
    await pause();
    if ((await trace()).flat().length !== beforeCopy)
      throw Error("Selected Ctrl+C leaked into SSH");
    renderer.clearSelection();
    checks.push(
      "Selected Ctrl+C copies terminal text without interrupting SSH",
    );
    terminal.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "c",
        code: "KeyC",
        keyCode: 67,
        ctrlKey: true,
        bubbles: true,
        cancelable: true,
      }),
    );
    await until(
      () => contains("\x03"),
      "Terminal Ctrl+C reaches SSH as ETX (synthetic key event)",
    );
    terminal.value = "€@";
    terminal.dispatchEvent(
      new InputEvent("input", {
        data: "€@",
        inputType: "insertText",
        bubbles: true,
      }),
    );
    await until(
      () => contains("€@"),
      "Committed Unicode text reaches SSH (not an OS AltGr/IME test)",
    );
    paste(terminal, "paste-ui");
    await until(
      () => contains("paste-ui"),
      "Single-line paste traverses terminal input IPC",
    );
    const count = (await trace()).length;
    paste(terminal, "review-ui\nsecond-line");
    await until(
      () =>
        !!document.querySelector('dialog[aria-label="Confirm terminal paste"]'),
      "Multiline paste opens confirmation",
    );
    if ((await trace()).length !== count)
      throw new Error("Multiline paste sent before confirmation");
    button("Cancel")!.click();
    await until(
      () => !document.querySelector("dialog[open]"),
      "Cancelled paste sends no data",
    );
    if ((await trace()).length !== count)
      throw new Error("Cancelled paste sent data");
    if (scope === "clipboard") {
      // A shell redraw addresses cells using modern wcwidth. With xterm's
      // legacy width for lightning, the first e is left behind as "eecho".
      const write = (text: string) =>
        new Promise<void>((resolve) => renderer.write(text, resolve));
      renderer.unicode.activeVersion = "6";
      await write("\r\n\u26a1 > e\r\x1b[5Cecho");
      const legacyLine = renderer.buffer.active
        .getLine(renderer.buffer.active.baseY + renderer.buffer.active.cursorY)
        ?.translateToString(true);
      if (legacyLine !== "\u26a1 > eecho")
        throw Error(
          "Legacy-width control failed to reproduce the exact symptom",
        );
      checks.push(
        "Legacy Unicode-width control reproduces eecho from a single echoed e plus shell redraw",
      );
      renderer.unicode.activeVersion = "11";
      await write("\r\n\u26a1 > e\r\x1b[5Cecho");
      const currentLine = () =>
        renderer.buffer.active
          .getLine(
            renderer.buffer.active.baseY + renderer.buffer.active.cursorY,
          )
          ?.translateToString(true);
      if (currentLine() !== "\u26a1 > echo")
        throw Error("Unicode prompt redraw duplicates the first letter");
      await write("\b\b\b\b\x1b[K");
      if (currentLine()?.trimEnd() !== "\u26a1 >")
        throw Error(
          `Unicode prompt deletion leaves a ghost character: ${JSON.stringify(currentLine())}`,
        );
      checks.push(
        "Modern Unicode prompt redraw and deletion leave no duplicated or undeletable first letter",
      );
      await invoke("capture_report", { checks, passed: true });
      document.documentElement.dataset.fixtureChecks = "passed";
      return;
    }
    for (const name of ["Single pane", "Two panes", "Four panes"]) {
      document.querySelector<HTMLElement>(".view-menu summary")!.click();
      button(name)!.click();
      const layout = { "Single pane": 1, "Two panes": 2, "Four panes": 4 }[
        name
      ];
      await until(
        () => !!document.querySelector(`.workspace.layout-${layout}`),
        `${name} layout selected without reconnect`,
      );
    }
    if (
      await invoke<number>("session_start", { profileId: 1 }).then(
        () => true,
        () => false,
      )
    )
      throw new Error("Duplicate connection was allowed");
    checks.push("Existing bookmark connection cannot be duplicated");
    document
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Actions for read me.txt"]',
      )!
      .click();
    await until(() => !!button("Permissions"), "File context actions open");
    button("Permissions")!.click();
    await until(
      () => !!document.querySelector('input[aria-label="Octal mode"]'),
      "Permissions fetched from real SFTP server",
    );
    document
      .querySelector<HTMLInputElement>('input[aria-label="Group read"]')!
      .click();
    button("Apply permissions")!.click();
    await until(async () => {
      const result = await invoke<{ properties: { permissions: number } }>(
        "file_action",
        {
          id: ids[0],
          action: { kind: "properties", path: "/fixture/read me.txt" },
        },
      ).catch(() => null);
      return result?.properties?.permissions === 0o100640;
    }, "Permission checkbox writes 0640 through SFTP");
    await until(
      () => !document.querySelector("dialog[open]"),
      "Permission dialog closes after success",
    );
    document
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Actions for read me.txt"]',
      )!
      .click();
    await until(
      () => !!button("Permissions"),
      "File menu remains usable after mutation",
    );
    const afterCounts = await Promise.all(
      ids
        .slice(1)
        .map((id) =>
          invoke<number[][]>("capture_trace", { id }).then(
            (values) => values.length,
          ),
        ),
    );
    if (otherCounts.some((n, i) => n !== afterCounts[i]))
      throw new Error("Input leaked into another pane");
    checks.push(
      "Focused input/paste never broadcasts to the other three SSH connections",
    );
    const hosts = () =>
      Array.from(document.querySelectorAll<HTMLElement>(".terminal-host"));
    await until(
      () => hosts().every((h) => Number(h.dataset.highlightSpans) > 0),
      "Keyword/IPv4 decorations appear in all four real SSH renderers",
    );
    document
      .querySelector<HTMLButtonElement>('button[aria-label="Settings"]')!
      .click();
    await until(
      () =>
        !!document.querySelector('.highlight-settings input[type="checkbox"]'),
      "Highlighting controls open in Appearance",
    );
    document.getElementById("settings-tab-highlighting")!.click();
    await pause();
    const highlighting = Array.from(
      document.querySelectorAll<HTMLInputElement>(
        '.highlight-settings input[type="checkbox"]',
      ),
    )[0];
    highlighting.click();
    await until(
      () => hosts().every((h) => h.dataset.highlightSpans === undefined),
      "Turning highlighting off clears decorations without reconnecting",
    );
    highlighting.click();
    await until(
      () => hosts().every((h) => Number(h.dataset.highlightSpans) > 0),
      "Turning highlighting back on restores visible terminal rules",
    );
    // Keep rendering checks active while the settings dialog is open.
    await invoke("session_input", {
      id: ids[0],
      bytes: Array.from(
        new TextEncoder().encode("\x1b[?1049hERROR alternate screen"),
      ),
    });
    await until(
      () => hosts()[0].dataset.highlightSpans === undefined,
      "Alternate-screen applications receive no highlighting decorations",
    );
    await invoke("session_input", {
      id: ids[0],
      bytes: Array.from(new TextEncoder().encode("\x1b[?1049l")),
    });
    await until(
      () => Number(hosts()[0].dataset.highlightSpans) > 0,
      "Returning to normal output restores highlighting",
    );
    for (let i = 0; i < 4 && document.querySelector("dialog[open]"); i++) {
      Array.from(
        document.querySelectorAll<HTMLButtonElement>(
          'dialog[open] button[aria-label="Close dialog"]',
        ),
      )
        .at(-1)!
        .click();
      await pause();
    }
    await checkAppearance(until, checks);
    document.querySelector<HTMLButtonElement>(".sidebar .new-session")!.click();
    await until(
      () => !!document.querySelector("dialog[open] select"),
      "Connection editor opens without replacing live SSH sessions",
    );
    const field = (label: string) =>
      Array.from(document.querySelectorAll("dialog[open] label"))
        .find((l) => l.textContent?.trim().startsWith(label))!
        .querySelector<HTMLInputElement | HTMLSelectElement>("input,select")!;
    const setValue = (label: string, value: string) => {
      const el = field(label);
      const prototype =
        el instanceof HTMLSelectElement
          ? HTMLSelectElement.prototype
          : HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(el, value);
      el.dispatchEvent(
        new Event(el instanceof HTMLSelectElement ? "change" : "input", {
          bubbles: true,
        }),
      );
    };
    setValue("Protocol", "Serial");
    await until(
      () =>
        document
          .querySelector("dialog[open]")
          ?.textContent?.includes("Baud rate") === true,
      "SERIAL settings appear with explicit control-line notice",
    );
    await until(
      () =>
        field("Baud rate").value === "115200" &&
        field("Data bits").value === "8" &&
        field("Stop bits").value === "1",
      "SERIAL defaults are 115200 / 8-N-1",
    );
    setValue("Session name", "Serial console");
    await pause();
    setValue("COM device", "COM7");
    await pause();
    setValue("Baud rate", "9600");
    await until(
      () => field("Baud rate").value === "9600",
      "SERIAL baud rate is editable in draft metadata",
    );
    setValue("Flow control", "Hardware");
    await until(
      () => field("Assert RTS").disabled,
      "RTS/CTS disables manual RTS to retain driver handshake ownership",
    );
    button("Refresh COM ports")!.click();
    await until(
      () =>
        !!document
          .querySelector("dialog[open] small[role=status]")
          ?.textContent?.match(/found|failed/),
      "COM enumeration returns an explicit result without connecting",
    );
    await until(
      () => document.querySelectorAll(".terminal-host").length === 4,
      "Editing SERIAL settings retains all four SSH terminal renderers",
    );
    document
      .querySelector<HTMLButtonElement>(
        'dialog[open] button[aria-label="Close dialog"]',
      )!
      .click();
    await pause();
    document
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Send to terminals"]',
      )!
      .click();
    const choices = () =>
      Array.from(
        document.querySelectorAll<HTMLInputElement>(
          '.dispatch-targets input[type="checkbox"]',
        ),
      ).sort((a, b) =>
        (a.getAttribute("aria-label") ?? "").localeCompare(
          b.getAttribute("aria-label") ?? "",
        ),
      );
    await until(
      () => choices().length === 4 && choices().every((c) => !c.checked),
      "Dispatch starts with no enrolled live terminals",
    );
    choices()[0].click();
    await pause();
    choices()[2].click();
    await pause();
    const dispatchText = document.querySelector<HTMLTextAreaElement>(
      'textarea[aria-label="Text to send"]',
    )!;
    Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!.call(dispatchText, "echo broadcast-fixture-check");
    dispatchText.dispatchEvent(new Event("input", { bubbles: true }));
    await until(
      () => button("Review dispatch")?.disabled === false,
      "Two explicit recipients and valid text enable review",
    );
    const beforeDispatch = await Promise.all(
      ids.map((id) =>
        invoke<number[][]>("capture_trace", { id }).then((t) => t.length),
      ),
    );
    button("Review dispatch")!.click();
    await until(
      () => document.querySelectorAll(".dispatch-results li").length === 2,
      "Dispatch review lists exactly the two selected live endpoints",
    );
    button("Back")!.click();
    await pause();
    const afterReview = await Promise.all(
      ids.map((id) =>
        invoke<number[][]>("capture_trace", { id }).then((t) => t.length),
      ),
    );
    checkEqual(
      beforeDispatch,
      afterReview,
      "Review and Back send no terminal input",
      checks,
    );
    document.querySelector<HTMLInputElement>(".dispatch-enter input")!.click();
    await pause();
    button("Review dispatch")!.click();
    await until(
      () =>
        document
          .querySelector(".dispatch-preview")
          ?.textContent?.endsWith("↵") === true,
      "Review explicitly shows the appended Enter",
    );
    const sendButton = button("Send to 2 terminals")!;
    sendButton.click();
    sendButton.click();
    await until(
      () =>
        document.querySelector('dialog[aria-label="Dispatch result"]') !== null,
      "Reviewed input is queued once and displays per-target results",
    );
    const afterDispatch = await Promise.all(
      ids.map((id) => invoke<number[][]>("capture_trace", { id })),
    );
    checkEqual(
      afterDispatch.map((t, i) => t.length - beforeDispatch[i]),
      [1, 0, 1, 0],
      "Double-click dispatch reaches only selected live sessions once",
      checks,
    );
    checkEqual(
      afterDispatch[0].at(-1),
      Array.from(new TextEncoder().encode("echo broadcast-fixture-check\r")),
      "Dispatch preserves text and appends exactly one CR",
      checks,
    );
    await until(
      () => [0, 2].every((i) => hosts()[i].dataset.dispatchEcho === "true"),
      "Both selected SSH fixture servers echo the dispatched text through real terminal output",
    );
    checkEqual(
      [hosts()[1].dataset.dispatchEcho, hosts()[3].dataset.dispatchEcho],
      ["false", "false"],
      "Unselected SSH terminals receive no dispatch echo",
      checks,
    );
    const guardBefore = await invoke<number[][]>("capture_trace", {
      id: ids[0],
    });
    for (const bad of [
      { ids: [ids[0], 999999], text: "must-not-send" },
      { ids: [ids[0], ids[0]], text: "must-not-send" },
      { ids: [ids[0]], text: "\x1b[31m" },
    ]) {
      let refused = false;
      try {
        await invoke("terminal_dispatch", { ...bad, appendEnter: true });
      } catch {
        refused = true;
      }
      if (!refused) throw new Error("Unsafe dispatch was accepted");
    }
    checkEqual(
      (await invoke<number[][]>("capture_trace", { id: ids[0] })).length,
      guardBefore.length,
      "Stale IDs, duplicate IDs and control text are refused before any queuing",
      checks,
    );
    button("Close")!.click();
    await pause();
    document
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Send to terminals"]',
      )!
      .click();
    await until(
      () => choices().length === 4 && choices().every((c) => !c.checked),
      "Closing and reopening dispatch resets all recipient selections",
    );
    button("Select connected terminals")!.click();
    await pause();
    await invoke("session_disconnect", { id: ids[3] });
    await until(
      () => choices().length === 3 && choices().every((c) => c.checked),
      "A disconnected live ID is removed from explicit recipients",
    );
    const fourth = document.querySelector<HTMLElement>(
      `section[data-live-id="${ids[3]}"]`,
    )!;
    await until(
      () =>
        Array.from(fourth.querySelectorAll("button")).some(
          (b) => b.textContent?.trim() === "Reconnect",
        ),
      "Ended fixture terminal offers explicit reconnect",
    );
    Array.from(fourth.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "Reconnect")!
      .click();
    await until(
      () =>
        choices().length === 4 &&
        choices().filter((c) => c.checked).length === 3,
      "A reconnected terminal receives a fresh ID and stays unselected",
    );
    const reconnectedChoice = document.querySelector<HTMLInputElement>(
      '.dispatch-targets input[aria-label="Select SSH fixture 4"]',
    )!;
    if (reconnectedChoice.checked)
      throw new Error("Reconnected target enrolled automatically");
    // Retain a review image; this final command is not dispatched.
    const finalText = document.querySelector<HTMLTextAreaElement>(
      'textarea[aria-label="Text to send"]',
    )!;
    Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!.call(finalText, "hostname");
    finalText.dispatchEvent(new Event("input", { bubbles: true }));
    await pause();
    button("Review dispatch")!.click();
    await until(
      () => document.querySelectorAll(".dispatch-results li").length === 3,
      "Final review retains only the three explicitly selected original live IDs",
    );
    // Close the dispatch review before exercising the local terminal search.
    document
      .querySelector<HTMLButtonElement>(
        'dialog[open] button[aria-label="Close dialog"]',
      )!
      .click();
    await pause();
    await checkLocalSearch(hosts(), until, checks);
    document
      .querySelector<HTMLButtonElement>('button[aria-label="Close search"]')!
      .click();
    document.querySelector<HTMLButtonElement>(".sidebar .new-session")!.click();
    await until(
      () =>
        Array.from(document.querySelectorAll("dialog[open] summary")).some(
          (el) => el.textContent === "Agent and OpenSSH identity rules",
        ),
      "SSH editor exposes explicit OpenSSH identity policy settings",
    );
    Array.from(document.querySelectorAll<HTMLElement>("dialog[open] summary"))
      .find((el) => el.textContent === "Agent and OpenSSH identity rules")!
      .click();
    await until(
      () =>
        Array.from(document.querySelectorAll("dialog[open] label")).some((el) =>
          el.textContent?.trim().startsWith("OpenSSH identity host"),
        ) &&
        Array.from(document.querySelectorAll("dialog[open] label")).some((el) =>
          el.textContent?.trim().startsWith("SSH identity config file"),
        ),
      "SSH identity host override and optional config path are visible without connecting",
    );
    await until(
      () =>
        ["400", "700", "italic 400", "italic 700"].every((style) =>
          document.fonts.check(
            `${style} 14px "YuuTerm JetBrains Mono"`,
            "Aa\ue0b0\uf120",
          ),
        ),
      "All four bundled terminal font faces load inside native WebView2",
    );
    document
      .querySelector<HTMLButtonElement>(
        'dialog[open] button[aria-label="Close dialog"]',
      )!
      .click();
    await pause();
    // Menu actions run only against isolated profiles and real loopback sessions.
    const sessionsTab = Array.from(
      document.querySelectorAll<HTMLButtonElement>(".side-tabs button"),
    ).find((b) => b.textContent === "Sessions")!;
    sessionsTab.click();
    await pause();
    const row = document.querySelector<HTMLElement>(".session-row")!;
    row.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: 120,
        clientY: 200,
      }),
    );
    await until(
      () =>
        !!document.querySelector('[role="menu"] button') &&
        Array.from(document.querySelectorAll('[role="menuitem"]')).some(
          (el) => el.textContent === "Edit session",
        ),
      "Session right-click exposes application actions without starting a connection",
    );
    Array.from(
      document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
    )
      .find((b) => b.textContent === "Edit session")!
      .click();
    await until(
      () =>
        !!document.querySelector('dialog[aria-label="Edit session"]') &&
        !document.querySelector('[role="menu"]'),
      "Edit opens the profile editor; no redundant Rename action is offered",
    );
    document
      .querySelector<HTMLButtonElement>(
        'dialog[open] button[aria-label="Close dialog"]',
      )!
      .click();
    await pause();
    const terminalHost = document.querySelector<HTMLElement>(".terminal-host")!;
    terminalHost.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: innerWidth - 4,
        clientY: innerHeight - 4,
      }),
    );
    await pause();
    const menu = document.querySelector<HTMLElement>('[role="menu"]')!;
    const rect = menu.getBoundingClientRect();
    if (rect.right > innerWidth || rect.bottom > innerHeight)
      throw new Error("Context menu escaped viewport");
    menu.querySelector<HTMLButtonElement>("button:last-child")!.click();
    await until(
      () =>
        !!document.querySelector(".terminal-find") &&
        !document.querySelector('[role="menu"]'),
      "Terminal menu stays within the viewport and opens local search",
    );
    document
      .querySelector<HTMLButtonElement>('button[aria-label="Close search"]')!
      .click();
    await pause();
    terminalHost.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: 440,
        clientY: 220,
      }),
    );
    await until(
      () =>
        Array.from(document.querySelectorAll('[role="menuitem"]'))
          .map((b) => b.textContent)
          .join(",") === "Copy,Paste,Select all,Find in terminal",
      "Terminal right-click offers clipboard, selection and search actions instead of browser navigation",
    );
    document
      .querySelector<HTMLElement>('[role="menu"]')!
      .dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    row.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: 100,
        clientY: 250,
      }),
    );
    await pause();
    if (
      Array.from(document.querySelectorAll('[role="menuitem"]')).some(
        (el) => el.textContent === "Rename",
      )
    )
      throw new Error("Redundant rename remains");
    Array.from(
      document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
    )
      .find((b) => b.textContent === "Duplicate session")!
      .click();
    await until(
      () =>
        document.querySelectorAll(".session-row").length === 5 &&
        !!Array.from(document.querySelectorAll(".session-info")).find((el) =>
          el.textContent?.includes("(copy)"),
        ),
      "Duplicate creates a fifth saved fixture profile with a copy name",
    );
    await until(
      () =>
        document.querySelectorAll("section[data-live-id]").length === 4 &&
        !document.querySelector("dialog[open]"),
      "Duplicating neither starts a connection nor opens an editor",
    );
    const copy = Array.from(
      document.querySelectorAll<HTMLButtonElement>(".session-select"),
    ).find((el) => el.textContent?.includes("(copy)"))!;
    copy.click();
    await until(
      () =>
        !!document.querySelector('section[data-profile-id="5"]') &&
        !document
          .querySelector('section[data-profile-id="5"]')
          ?.getAttribute("data-live-id") &&
        document.querySelectorAll("section[data-live-id]").length === 4,
      "Single click opens the saved profile without connecting",
    );
    copy.dispatchEvent(
      new MouseEvent("dblclick", { bubbles: true, cancelable: true }),
    );
    copy.dispatchEvent(
      new MouseEvent("dblclick", { bubbles: true, cancelable: true }),
    );
    await until(
      () =>
        !!document
          .querySelector('section[data-profile-id="5"][data-live-id]')
          ?.textContent?.includes("Connected") &&
        document.querySelectorAll("section[data-live-id]").length === 5,
      "Repeated double-click opens one real SSH connection for the copied profile",
    );
    const copyLiveId = document
      .querySelector('section[data-profile-id="5"]')!
      .getAttribute("data-live-id");
    copy.dispatchEvent(
      new MouseEvent("dblclick", { bubbles: true, cancelable: true }),
    );
    await pause();
    checkEqual(
      document
        .querySelector('section[data-profile-id="5"]')!
        .getAttribute("data-live-id"),
      copyLiveId,
      "Double-click on an active connection retains its live ID",
      checks,
    );
    document
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Close SSH fixture 1 (copy)"]',
      )!
      .click();
    await until(
      () => document.querySelectorAll("section[data-live-id]").length === 4,
      "Closing the double-clicked fixture leaves the four original connections running",
    );
    row.querySelector<HTMLButtonElement>(".session-select")!.click();
    await pause();
    const main = document.querySelector<HTMLElement>("main.main")!;
    const originalWidth = main.getBoundingClientRect().width;
    const toggle = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Toggle sidebar"]',
    )!;
    toggle.click();
    await until(
      () =>
        main.getBoundingClientRect().width > originalWidth + 200 &&
        main.getBoundingClientRect().left === 0,
      "Hiding sidebar expands the workspace into the released grid column",
    );
    toggle.click();
    await until(
      () =>
        Math.abs(main.getBoundingClientRect().width - originalWidth) < 2 &&
        document.querySelectorAll("section[data-live-id]").length === 4,
      "Showing sidebar restores its width without replacing live sessions",
    );
    await until(
      () => document.querySelectorAll(".session-statistics").length === 4,
      "All four SSH panes start host statistics automatically without a pulse button",
    );
    await until(
      () =>
        document
          .querySelector(".session-statistics")
          ?.textContent?.includes("/ 64%") === true &&
        !!document.querySelector(
          '.session-statistics span[title^="RAM used: 75%"]',
        ),
      "Opt-in statistics traverse a separate real SSH exec channel",
    );
    await until(
      () =>
        document.querySelector(".stat-cpu strong")?.textContent === "20%" &&
        document.querySelector(".stat-download strong")?.textContent !== "—",
      "Second sample computes CPU and interface rates from successive fixture counters",
    );
    await until(
      () =>
        document
          .querySelector(".stat-host")
          ?.getAttribute("title")
          ?.includes("Debian GNU/Linux 13") === true &&
        document.querySelector(".stat-host")?.textContent === "fixture-host" &&
        document.querySelector(".stat-uptime")?.textContent === "3d 0h" &&
        document
          .querySelector(".session-statistics")
          ?.previousElementSibling?.classList.contains("terminal-shell") ===
          true,
      "Bottom statistics show hostname, OS/kernel hover details and three-day uptime",
    );
    await until(
      () =>
        !document.querySelector(".workspace-bar, .view-label, footer") &&
        !document.body.textContent?.includes("Connection metadata only") &&
        document.querySelector(".pane-title")?.textContent?.trim() ===
          "Connected",
      "Redundant workspace labels and saved-locally footer are removed; status stays with the live pane",
    );
    await until(() => {
      const up =
        document.querySelector(".stat-upload")?.getAttribute("title") ?? "";
      const down =
        document.querySelector(".stat-download")?.getAttribute("title") ?? "";
      const disk =
        document.querySelector(".stat-disk")?.getAttribute("title") ?? "";
      return (
        up.includes("Total sent:") &&
        up.includes("eth0") &&
        down.includes("Total received:") &&
        disk.includes("/dev/fixture  1000  600  350  64%  /")
      );
    }, "Rate tooltips show cumulative interface totals and disk hover retains df fields");
    const cpuHover = document.querySelector<HTMLElement>(".stat-cpu")!;
    cpuHover.focus();
    await until(
      () =>
        document.querySelectorAll(".cpu-process-tooltip tbody tr").length ===
          5 &&
        document
          .querySelector(".cpu-process-tooltip")
          ?.textContent?.includes("fixture-worker") === true,
      "CPU focus fetches the five typed process rows through an independent SSH exec channel",
    );
    cpuHover.blur();
    await until(
      () => !document.querySelector(".cpu-process-tooltip"),
      "CPU process tooltip closes on blur",
    );
    const bar = document.querySelector<HTMLElement>(".session-statistics")!;
    await until(() => {
      const host = bar.querySelector<HTMLElement>(".stat-host")!;
      const text = host.querySelector("strong")!;
      const measured = text.getBoundingClientRect().width;
      return (
        Math.abs(host.getBoundingClientRect().width - measured - 36) < 2 &&
        getComputedStyle(bar.querySelector(".stat-latency")!).width !==
          "78px" &&
        getComputedStyle(bar.querySelector(".stat-uptime")!).width !== "80px"
      );
    }, "Hostname fits its frozen connection name while request time and uptime use compact widths");
    await until(() => {
      const cell = bar.querySelector<HTMLElement>(".stat-memory")!;
      const text = cell.querySelector("strong")!;
      return (
        (text.textContent?.match(/GiB/g)?.length ?? 0) === 2 &&
        /3[.,]00 GiB \/ 4[.,]00 GiB/.test(text.textContent ?? "") &&
        Math.abs(
          cell.getBoundingClientRect().width -
            text.getBoundingClientRect().width -
            36,
        ) < 2 &&
        Math.abs(
          text.getBoundingClientRect().left -
            cell.querySelector("svg")!.getBoundingClientRect().right -
            5,
        ) < 1
      );
    }, "RAM fits both two-decimal unit-labelled values with only the normal icon gap");
    const metrics = Array.from(
      bar.querySelectorAll<HTMLElement>(":scope > span"),
    );
    const geometry = () =>
      metrics.map((el) => {
        const r = el.getBoundingClientRect();
        return [r.left, r.top, r.width, r.height];
      });
    const stable = geometry();
    const cpuValue = bar.querySelector<HTMLElement>(".stat-cpu .cpu-value")!;
    const previousCpu = cpuValue.textContent;
    for (const value of ["9", "10"]) {
      cpuValue.textContent = value;
      if (JSON.stringify(geometry()) !== JSON.stringify(stable))
        throw new Error(
          "Statistics cells shifted when CPU digit count changed",
        );
    }
    const twoDigitWidth = bar
      .querySelector(".stat-cpu")!
      .getBoundingClientRect().width;
    cpuValue.textContent = "100";
    const threeDigitWidth = bar
      .querySelector(".stat-cpu")!
      .getBoundingClientRect().width;
    if (
      threeDigitWidth <= twoDigitWidth ||
      threeDigitWidth - twoDigitWidth > 10
    )
      throw new Error("CPU cell did not expand by one digit at 100 percent");
    cpuValue.textContent = previousCpu;
    checks.push(
      "CPU keeps two-digit values stable and expands by one character only at 100 percent",
    );
    await checkStatisticsSettings(until, checks);
    await until(
      () =>
        Array.from(document.querySelectorAll(".stat-download strong")).every(
          (el) => el.textContent?.includes("MB/s"),
        ) &&
        Array.from(document.querySelectorAll(".stat-cpu strong")).every(
          (el) => el.textContent === "20%",
        ),
      "Restored default-on panes resume interval CPU and MB/s sampling",
    );
    await checkSessionTabs(until);
    await invoke("capture_report", { checks, passed: true });
    document.documentElement.dataset.fixtureChecks = "passed";
  } catch (e) {
    checks.push("FAILED: " + String(e));
    await invoke("capture_report", { checks, passed: false });
  }
}

function checkEqual(
  actual: unknown,
  expected: unknown,
  label: string,
  checks: string[],
) {
  if (JSON.stringify(actual) !== JSON.stringify(expected))
    throw new Error(
      `${label}; actual ${JSON.stringify(actual)}; expected ${JSON.stringify(expected)}`.slice(
        0,
        350,
      ),
    );
  checks.push(label);
}

async function checkAppearance(
  until: (
    predicate: () => boolean | Promise<boolean>,
    label: string,
  ) => Promise<void>,
  checks: string[],
) {
  const initial = (await invoke<Bootstrap>("bootstrap")).prefs;
  const field = (label: string) =>
    document.querySelector<HTMLInputElement>(`dialog [aria-label="${label}"]`)!;
  const change = (label: string, value: string) => {
    const el = field(label);
    const prototype =
      el.tagName === "SELECT"
        ? HTMLSelectElement.prototype
        : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(el, value);
    el.dispatchEvent(
      new Event(el.tagName === "SELECT" ? "change" : "input", {
        bubbles: true,
      }),
    );
  };
  const font = () =>
    getComputedStyle(
      document.querySelector<HTMLElement>(".terminal-host .xterm-rows")!,
    )
      .fontFamily.split(",")[0]
      .replaceAll('"', "")
      .trim();
  const button = (text: string) =>
    Array.from(
      document.querySelectorAll<HTMLButtonElement>("dialog button"),
    ).find((b) => b.textContent?.trim() === text)!;
  const open = async () => {
    document
      .querySelector<HTMLButtonElement>('button[aria-label="Settings"]')!
      .click();
    const end = Date.now() + 3000;
    while (
      !document.querySelector('dialog[aria-label="Settings"]') &&
      Date.now() < end
    )
      await new Promise((r) => setTimeout(r, 20));
    if (!document.querySelector('dialog[aria-label="Settings"]'))
      throw new Error("Appearance did not open");
  };
  await open();
  document.getElementById("settings-tab-highlighting")!.click();
  await new Promise((r) => setTimeout(r, 40));
  await until(() => {
    const v4 = field("IPv4 highlight color").getBoundingClientRect();
    const v6 = field("IPv6 highlight color").getBoundingClientRect();
    const words = field("Rule 1 color").getBoundingClientRect();
    return (
      Math.abs(v4.left - words.left) < 1 &&
      Math.abs(v6.left - words.left) < 1 &&
      words.top - v6.bottom >= 18 &&
      field("IPv4 highlight color").value === "#89b4fa" &&
      field("IPv6 highlight color").value === "#89b4fa"
    );
  }, "IPv4 and IPv6 have aligned independent blue color controls with space before word rules");
  document.getElementById("settings-tab-appearance")!.click();
  await new Promise((r) => setTimeout(r, 40));
  change("Terminal font", "Consolas");
  change("Default terminal font size", "18");
  await until(
    () =>
      font() === "Consolas" &&
      getComputedStyle(
        document.querySelector<HTMLElement>(".terminal-host .xterm-rows")!,
      ).fontSize === "18px",
    "Appearance previews the selected font and size in the existing terminal",
  );
  document.getElementById("settings-tab-highlighting")!.click();
  await new Promise((r) => setTimeout(r, 40));
  field("Highlight IPv6").click();
  change("Rule 1 type", "regex");
  change("Rule 1 words", "[");
  await until(
    () =>
      button("Save settings").disabled &&
      !!document.querySelector(".appearance-error"),
    "Invalid regex drafts explain the error and disable Save",
  );
  document
    .querySelector<HTMLButtonElement>(
      'dialog button[aria-label="Close dialog"]',
    )!
    .click();
  await until(
    () =>
      !document.querySelector("dialog[open]") &&
      font() === initial.font_family &&
      getComputedStyle(
        document.querySelector<HTMLElement>(".terminal-host .xterm-rows")!,
      ).fontSize === `${initial.font_size}px`,
    "Appearance X discards previewed font and size immediately",
  );
  const afterX = (await invoke<Bootstrap>("bootstrap")).prefs;
  checkEqual(
    [afterX.font_family, afterX.font_size, afterX.highlighting],
    [initial.font_family, initial.font_size, initial.highlighting],
    "Closing Appearance does not change the saved backend preferences",
    checks,
  );
  await open();
  document.getElementById("settings-tab-appearance")!.click();
  await new Promise((r) => setTimeout(r, 40));
  change("Terminal font", "Consolas");
  document.getElementById("settings-tab-highlighting")!.click();
  await new Promise((r) => setTimeout(r, 40));
  field("Highlight IPv6").click();
  change("Rule 1 type", "regex");
  change("Rule 1 words", "WARN[0-9]+|ERROR");
  change("IPv6 highlight color", "#ffffff");
  await new Promise((r) => setTimeout(r, 50));
  button("Save settings").click();
  await until(async () => {
    const p = (await invoke<Bootstrap>("bootstrap")).prefs;
    return (
      !document.querySelector("dialog[open]") &&
      p.font_family === "Consolas" &&
      !p.highlighting.ipv6 &&
      p.highlighting.ipv6_color === "#ffffff" &&
      p.highlighting.ipv4_color === initial.highlighting.ipv4_color &&
      p.highlighting.rules[0].regex === true
    );
  }, "Explicit Save commits font, IPv6 selection and regex mode to the isolated backend");
  await open();
  await until(
    () =>
      field("Terminal font").value === "Consolas" &&
      !field("Highlight IPv6").checked &&
      field("Rule 1 type").value === "regex" &&
      field("IPv6 highlight color").value === "#ffffff",
    "Saved appearance selections are restored when reopening the dialog",
  );
  change("Default terminal font size", "22");
  button("Cancel").click();
  await until(
    () =>
      !document.querySelector("dialog[open]") &&
      getComputedStyle(
        document.querySelector<HTMLElement>(".terminal-host .xterm-rows")!,
      ).fontSize === `${initial.font_size}px`,
    "Appearance Cancel discards edits just like X",
  );
  await open();
  change("Terminal font", initial.font_family);
  document.getElementById("settings-tab-highlighting")!.click();
  await new Promise((r) => setTimeout(r, 40));
  change("IPv6 highlight color", initial.highlighting.ipv6_color);
  document.getElementById("settings-tab-highlighting")!.click();
  await new Promise((r) => setTimeout(r, 40));
  field("Highlight IPv6").click();
  change("Rule 1 type", "words");
  change("Rule 1 words", initial.highlighting.rules[0].keywords);
  await new Promise((r) => setTimeout(r, 50));
  button("Save settings").click();
  await until(
    () =>
      !document.querySelector("dialog[open]") && font() === initial.font_family,
    "Restoring appearance keeps all original SSH connections running",
  );
}

async function checkLocalSearch(
  hosts: HTMLElement[],
  until: (
    predicate: () => boolean | Promise<boolean>,
    label: string,
  ) => Promise<void>,
  checks: string[],
) {
  const liveIds = Array.from(
    document.querySelectorAll<HTMLElement>("section[data-live-id]"),
  ).map((el) => Number(el.dataset.liveId));
  await until(async () => {
    for (const id of liveIds) {
      const trace = await invoke<number[][]>("capture_trace", { id });
      if (
        !new TextDecoder()
          .decode(new Uint8Array(trace.flat()))
          .includes("ui-stream-check")
      )
        return false;
    }
    return true;
  }, "Reconnected terminal finishes its isolated input probe before local search checks");
  const before = await Promise.all(
    liveIds.map((id) =>
      invoke<number[][]>("capture_trace", { id }).then((t) => t.length),
    ),
  );
  hosts[0]
    .querySelector<HTMLTextAreaElement>(".xterm-helper-textarea")!
    .dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "f",
        code: "KeyF",
        ctrlKey: true,
        bubbles: true,
        cancelable: true,
      }),
    );
  await until(
    () =>
      document.querySelector(".terminal-find input") === document.activeElement,
    "Ctrl+F opens and focuses the local terminal search field",
  );
  const query = document.querySelector<HTMLInputElement>(
    ".terminal-find input",
  )!;
  const setQuery = (text: string) => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(query, text);
    query.dispatchEvent(new Event("input", { bubbles: true }));
  };
  const status = () =>
    document.querySelector('.terminal-find [role="status"]')?.textContent;
  setQuery("ERROR");
  await until(
    () => status() === "Match",
    "Literal search finds text decoded from the real SSH output",
  );
  setQuery("error");
  await new Promise((r) => setTimeout(r, 40));
  document
    .querySelector<HTMLButtonElement>('button[aria-label="Match case"]')!
    .click();
  await until(
    () => status() === "No match",
    "Case-sensitive search distinguishes lowercase from uppercase output",
  );
  document
    .querySelector<HTMLButtonElement>('button[aria-label="Match case"]')!
    .click();
  await until(
    () => status() === "Match",
    "Turning case sensitivity off restores the matching output",
  );
  setQuery(".*");
  await until(
    () => status() === "No match",
    "Search treats regex syntax as literal text",
  );
  setQuery("ERROR");
  await until(
    () => status() === "Match",
    "Search restores an actual terminal match",
  );
  query.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "Enter",
      bubbles: true,
      cancelable: true,
    }),
  );
  query.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "Enter",
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    }),
  );
  query.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    }),
  );
  await until(
    () =>
      !document.querySelector(".terminal-find") &&
      document.activeElement ===
        hosts[0].querySelector(".xterm-helper-textarea"),
    "Search Enter/Shift+Enter navigate locally; Escape closes and returns terminal focus",
  );
  checkEqual(
    await Promise.all(
      liveIds.map((id) =>
        invoke<number[][]>("capture_trace", { id }).then((t) => t.length),
      ),
    ),
    before,
    "Search query, navigation and Escape send no input to any SSH connection",
    checks,
  );
  document
    .querySelector<HTMLButtonElement>(
      'section[data-live-id] button[aria-label="Find in terminal"]',
    )!
    .click();
  await until(
    () =>
      document.querySelector(".terminal-find input") === document.activeElement,
    "Pane search button reopens the same local search field",
  );
}

// Session editing moved from an inline icon to the context menu.
async function editFirstFixtureSession() {
  Array.from(document.querySelectorAll<HTMLButtonElement>(".side-tabs button"))
    .find((button) => button.textContent?.trim() === "Sessions")!
    .click();
  await new Promise((resolve) => setTimeout(resolve, 80));
  document.querySelector<HTMLElement>(".session-row")!.dispatchEvent(
    new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 100,
      clientY: 200,
    }),
  );
  await new Promise((resolve) => setTimeout(resolve, 80));
  Array.from(document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'))
    .find((button) => button.textContent?.trim() === "Edit session")!
    .click();
}

async function checkStatisticsSettings(
  until: (
    predicate: () => boolean | Promise<boolean>,
    label: string,
  ) => Promise<void>,
  checks: string[],
) {
  const initial = (await invoke<Bootstrap>("bootstrap")).prefs;
  const liveIds = Array.from(
    document.querySelectorAll<HTMLElement>("section[data-live-id]"),
  ).map((p) => p.dataset.liveId);
  const field = (label: string) =>
    document.querySelector<HTMLInputElement>(`dialog [aria-label="${label}"]`)!;
  const button = (text: string) =>
    Array.from(
      document.querySelectorAll<HTMLButtonElement>("dialog button"),
    ).find((b) => b.textContent?.trim() === text)!;
  const pause = () => new Promise((r) => setTimeout(r, 60));
  const openGlobal = async () => {
    document
      .querySelector<HTMLButtonElement>('button[aria-label="Settings"]')!
      .click();
    await pause();
    document.getElementById("settings-tab-statistics")!.click();
    await pause();
  };
  const unit = (value: string) => {
    const select = field("Network rate unit");
    Object.getOwnPropertyDescriptor(
      HTMLSelectElement.prototype,
      "value",
    )!.set!.call(select, value);
    select.dispatchEvent(new Event("change", { bubbles: true }));
  };
  const edit = async () => {
    await editFirstFixtureSession();
    await pause();
    document.getElementById("session-tab-statistics")!.click();
    await pause();
  };
  await openGlobal();
  await until(
    () =>
      field("Show host statistics by default").checked &&
      field("Network rate unit").value === "Megabytes" &&
      document.querySelectorAll('[role="tab"]').length === 3,
    "Global Settings has three sections, statistics enabled by default and MB/s selected",
  );
  unit("Kilobytes");
  await until(
    () =>
      Array.from(document.querySelectorAll(".stat-download strong")).some((e) =>
        e.textContent?.includes("kB/s"),
      ),
    "Network units preview in existing live panes without reconnecting",
  );
  field("Show host statistics by default").click();
  await until(
    () => !document.querySelector(".session-statistics"),
    "Global statistics draft can hide inherited bars",
  );
  document
    .querySelector<HTMLButtonElement>(
      'dialog button[aria-label="Close dialog"]',
    )!
    .click();
  await until(
    async () =>
      document.querySelectorAll(".session-statistics").length === 4 &&
      JSON.stringify(
        (await invoke<Bootstrap>("bootstrap")).prefs.statistics,
      ) === JSON.stringify(initial.statistics),
    "Settings X restores default statistics and leaves committed units untouched",
  );
  await openGlobal();
  field("Show host statistics by default").click();
  unit("Megabits");
  await pause();
  button("Save settings").click();
  await until(
    async () =>
      !document.querySelector("dialog[open]") &&
      !document.querySelector(".session-statistics") &&
      !(await invoke<Bootstrap>("bootstrap")).prefs.statistics.enabled,
    "Explicit Save persists global statistics off",
  );
  await edit();
  if (field("Show session host statistics").checked)
    throw new Error("Session did not inherit global off");
  field("Show session host statistics").click();
  await pause();
  button("Save session").click();
  await until(
    async () =>
      !document.querySelector("dialog[open]") &&
      document.querySelectorAll(".session-statistics").length === 1 &&
      (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0]
        .statistics_enabled === true,
    "A saved session override enables its bar even with the global default off",
  );
  await until(
    () =>
      document
        .querySelector(".stat-download strong")
        ?.textContent?.includes("Mbit/s") === true,
    "Saved megabit units apply to a session override",
  );
  await edit();
  field("Show session host statistics").click();
  await pause();
  button("Save session").click();
  await until(
    () =>
      !document.querySelector("dialog[open]") &&
      !document.querySelector(".session-statistics"),
    "Session override can explicitly disable statistics",
  );
  await openGlobal();
  field("Show host statistics by default").click();
  unit("Megabytes");
  await pause();
  button("Save settings").click();
  await until(
    () =>
      !document.querySelector("dialog[open]") &&
      document.querySelectorAll(".session-statistics").length === 3,
    "Explicit session off survives enabling the global default",
  );
  await edit();
  button("Use global setting").click();
  await pause();
  button("Save session").click();
  await until(
    async () =>
      !document.querySelector("dialog[open]") &&
      document.querySelectorAll(".session-statistics").length === 4 &&
      (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0]
        .statistics_enabled === null,
    "Resetting a session to global inheritance restores its statistics bar",
  );
  checkEqual(
    Array.from(
      document.querySelectorAll<HTMLElement>("section[data-live-id]"),
    ).map((p) => p.dataset.liveId),
    liveIds,
    "Global and session statistics changes retain all original live connection IDs",
    checks,
  );
}

async function checkSessionTabs(
  until: (
    predicate: () => boolean | Promise<boolean>,
    label: string,
  ) => Promise<void>,
) {
  const initial = (await invoke<Bootstrap>("bootstrap")).prefs;
  const profile = initial.profiles[0];
  const pane = document.querySelector<HTMLElement>(
    `section[data-profile-id="${profile.id}"]`,
  )!;
  const liveId = pane.dataset.liveId;
  const size = () =>
    getComputedStyle(pane.querySelector<HTMLElement>(".xterm-rows")!).fontSize;
  const button = (label: string) =>
    Array.from(
      document.querySelectorAll<HTMLButtonElement>("dialog button"),
    ).find((el) => el.textContent?.trim() === label)!;
  const pause = () => new Promise((r) => setTimeout(r, 60));
  const open = async () => {
    await editFirstFixtureSession();
    await pause();
  };
  const choose = (value: string) => {
    const input = document.querySelector<HTMLInputElement>(
      'input[aria-label="Session font size"]',
    )!;
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, value === "global" ? "" : value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  };
  await open();
  await until(
    () =>
      document
        .getElementById("session-tab-connection")
        ?.getAttribute("aria-selected") === "true" &&
      document.querySelectorAll('dialog [role="tab"]').length === 3,
    "SSH editor groups Connection, Terminal and Statistics into separate tabs",
  );
  document
    .getElementById("session-tab-connection")!
    .dispatchEvent(
      new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
    );
  await until(
    () =>
      document
        .getElementById("session-tab-terminal")
        ?.getAttribute("aria-selected") === "true" &&
      document.activeElement?.id === "session-tab-terminal" &&
      !document.getElementById("session-panel-terminal")?.hidden &&
      document.getElementById("session-panel-connection")?.hidden === true,
    "Session tabs support keyboard focus and hide inactive panels",
  );
  choose("18");
  await pause();
  document.getElementById("session-tab-connection")!.click();
  await pause();
  document.getElementById("session-tab-terminal")!.click();
  await pause();
  await until(
    () =>
      document.querySelector<HTMLInputElement>(
        'input[aria-label="Session font size"]',
      )?.value === "18" && size() === `${initial.font_size}px`,
    "Font-size draft survives tab changes without changing live terminals before Save",
  );
  button("Cancel").click();
  await until(
    async () =>
      !document.querySelector("dialog[open]") &&
      (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0].font_size ===
        profile.font_size,
    "Cancelling session tabs discards the font-size override draft",
  );
  await open();
  document.getElementById("session-tab-terminal")!.click();
  await pause();
  choose("18");
  await pause();
  button("Save session").click();
  await until(
    async () =>
      !document.querySelector("dialog[open]") &&
      size() === "18px" &&
      (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0].font_size ===
        18 &&
      pane.dataset.liveId === liveId,
    "Saving the font-size field updates the original live terminal without reconnecting",
  );
  await open();
  document.getElementById("session-tab-terminal")!.click();
  await pause();
  await until(
    () =>
      document.querySelector<HTMLInputElement>(
        'input[aria-label="Session font size"]',
      )?.value === "18",
    "Reopening the session restores the saved font-size field",
  );
  choose("global");
  await pause();
  button("Save session").click();
  await until(
    async () =>
      !document.querySelector("dialog[open]") &&
      size() === `${initial.font_size}px` &&
      (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0].font_size ===
        null &&
      pane.dataset.liveId === liveId,
    "Use global setting restores inherited terminal size while retaining the connection",
  );
  await open();
  document.getElementById("session-tab-terminal")!.click();
  await pause();
}
