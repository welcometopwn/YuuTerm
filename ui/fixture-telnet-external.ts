import { invoke } from "@tauri-apps/api/core";
import { terminalText } from "./fixture-terminals";
import type { Bootstrap } from "./types";

// Synthetic DOM/xterm input in the actual native app, against a real telnetd.
// No password is read by this module. The host runner answers native CredUI.
export async function checkTelnetExternal(ids: number[], checks: string[]) {
  const profile = (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0];
  const pane = () =>
    document.querySelector<HTMLElement>(
      `.pane[data-profile-id="${profile.id}"]`,
    )!;
  const id = () => Number(pane()?.dataset.liveId);
  const host = () => pane()?.querySelector<HTMLElement>(".terminal-host")!;
  const rows = () => terminalText(pane());
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const until = async (f: () => boolean | Promise<boolean>, label: string) => {
    const end = Date.now() + 30000;
    while (Date.now() < end) {
      if (await f()) {
        checks.push(label);
        return;
      }
      await sleep(50);
    }
    // Never include terminal output, endpoint or account in failure reports.
    throw Error(label);
  };
  const trace = () => invoke<number[][]>("capture_trace", { id: id() });
  const send = (text: string) =>
    invoke("session_input", {
      id: id(),
      bytes: Array.from(new TextEncoder().encode(text)),
    });
  const input = (text: string) => {
    const textarea = host().querySelector<HTMLTextAreaElement>(
      ".xterm-helper-textarea",
    )!;
    textarea.focus();
    textarea.value = text;
    textarea.dispatchEvent(
      new InputEvent("input", {
        data: text,
        inputType: "insertText",
        bubbles: true,
      }),
    );
  };
  const line = () =>
    pane()?.querySelector<HTMLInputElement>('[aria-label="TELNET local line"]');
  const setLine = async (value: string) => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(line(), value);
    line()!.dispatchEvent(new Event("input", { bubbles: true }));
    await sleep(50);
  };
  const enterLine = () =>
    line()!.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
  const button = (text: string) =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent?.trim() === text,
    )!;
  const login = async () => {
    await until(
      () => /login:\s*$/i.test(rows()),
      "Real telnetd presents its OS login prompt",
    );
    if (line()) {
      await setLine(profile.username);
      enterLine();
    } else input(profile.username + "\r");
    await until(
      () => /password:\s*$/i.test(rows()),
      "Real login requests a password after the username",
    );
    await invoke("capture_telnet_password", { id: id() });
    await until(
      () => /[#$]\s*$/m.test(rows()),
      "Native one-shot password entry reaches the real OS shell",
    );
    await send(
      "unset HISTFILE; PS1='YT_PROMPT> '; printf '\\033[3J\\033[2J\\033[H'; printf 'YT_%s\\n' READY\r",
    );
    await until(
      () => rows().includes("YT_READY") && rows().includes("YT_PROMPT>"),
      "Shell readiness is confirmed by executed output, not login typeahead",
    );
  };
  if (ids.length !== 1 || ids[0] !== id())
    throw Error("Expected one external TELNET incarnation");
  await until(
    () => !!host()?.dataset.terminalCols,
    "Native xterm renderer is mounted",
  );
  const warning = pane().querySelector<HTMLElement>(".plaintext-warning");
  if (
    !warning ||
    warning.getBoundingClientRect().height <= 0 ||
    getComputedStyle(warning).visibility !== "visible"
  )
    throw Error("Plaintext warning is not visible");
  checks.push(
    "Permanent plaintext TELNET warning is visible in the native pane",
  );
  if ((await trace()).length)
    throw Error("Automatic fixture input preceded login");
  checks.push("External scope sends no automatic fixture probe before login");
  await login();

  input("printf 'YT_UTF8:%s\\n' 'Ω 日'\r");
  await until(
    () => rows().includes("YT_UTF8:Ω 日"),
    "Synthetic committed UTF-8 input (Ω 日) round-trips through the real shell and xterm",
  );
  const size = () =>
    `${host().dataset.terminalRows} ${host().dataset.terminalCols}`;
  await send("printf 'YT_SIZE:'; stty size\r");
  await until(
    () => rows().includes("YT_SIZE:" + size()),
    "Initial xterm dimensions reach real stty via NAWS",
  );
  const original = size();
  host().style.maxWidth = "620px";
  host().style.maxHeight = "270px";
  await until(
    () => size() !== original,
    "Resizing the actual terminal pane changes xterm dimensions",
  );
  await sleep(300);
  await send("printf 'YT_RESIZED:'; stty size\r");
  await until(
    () => rows().includes("YT_RESIZED:" + size()),
    "Pane ResizeObserver delivers the new dimensions to real stty",
  );
  host().style.maxWidth = "";
  host().style.maxHeight = "";
  await sleep(300);

  input(
    "printf 'YT_%s\\n' SLEEPING; sleep 30; printf 'YT_%s\\n' SLEEP_FINISHED\r",
  );
  await until(
    () => rows().includes("YT_SLEEPING"),
    "Real shell starts the interruptible sleep",
  );
  const textarea = host().querySelector<HTMLTextAreaElement>(
    ".xterm-helper-textarea",
  )!;
  textarea.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "c",
      code: "KeyC",
      ctrlKey: true,
      keyCode: 67,
      bubbles: true,
      cancelable: true,
    }),
  );
  textarea.dispatchEvent(
    new KeyboardEvent("keyup", {
      key: "c",
      code: "KeyC",
      ctrlKey: true,
      keyCode: 67,
      bubbles: true,
    }),
  );
  input("printf 'YT_%s\\n' INTERRUPTED\r");
  await until(
    () => rows().includes("YT_INTERRUPTED"),
    "Synthetic Ctrl+C interrupts sleep and subsequent native input executes",
  );
  if (rows().includes("YT_SLEEP_FINISHED"))
    throw Error("Sleep finished instead of being interrupted");

  await send(
    "i=0; while [ \"$i\" -lt 20000 ]; do i=$((i+1)); printf 'YT_FLOOD_%s\\n' \"$i\"; if [ $((i % 1000)) -eq 0 ]; then sleep .1; fi; done; printf 'YT_%s\\n' FLOOD_DONE\r",
  );
  await until(
    () => /YT_FLOOD_\d+/.test(rows()) && !rows().includes("YT_FLOOD_DONE"),
    "Real server output flood is in progress",
  );
  const menu = document.querySelector<HTMLElement>(".view-menu summary")!;
  menu.click();
  await until(
    () => !!document.querySelector<HTMLDetailsElement>(".view-menu")?.open,
    "Native View menu responds during the real server flood",
  );
  button("Single pane").click();
  await until(
    () => !document.querySelector<HTMLDetailsElement>(".view-menu")?.open,
    "Native layout action completes during the real server flood",
  );
  await until(
    () => rows().includes("YT_FLOOD_20000") && rows().includes("YT_FLOOD_DONE"),
    "Native renderer drains the complete 20,000-line real server burst",
  );
  input("printf 'YT_%s\\n' AFTER_FLOOD\r");
  await until(
    () => rows().includes("YT_AFTER_FLOOD"),
    "Native input remains functional after the flood (no performance claim)",
  );

  // Change the opt-in through the normal session editor, not fixture metadata.
  if (line() || profile.telnet.local_line_editing)
    throw Error("Line mode must default off");
  document.querySelector<HTMLElement>(".tab")!.dispatchEvent(
    new MouseEvent("contextmenu", {
      bubbles: true,
      clientX: 320,
      clientY: 65,
    }),
  );
  await until(
    () => !!button("Edit session"),
    "Normal session context menu exposes Edit session",
  );
  button("Edit session").click();
  await until(
    () => !!document.getElementById("session-tab-terminal"),
    "Normal session editor opens",
  );
  document.getElementById("session-tab-terminal")!.click();
  const toggle = () =>
    document.querySelector<HTMLInputElement>(
      '[aria-label="TELNET local line editing"]',
    );
  await until(
    () => !!toggle(),
    "TELNET local line editing is an explicit option",
  );
  toggle()!.click();
  await sleep(50);
  button("Save session").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Explicit Save session opts in to local editing",
  );
  if (line()) throw Error("Saved line mode changed an existing incarnation");

  const reconnect = async (unsent: boolean) => {
    const old = id();
    await invoke("session_disconnect", { id: old });
    await until(
      () => !!pane().querySelector<HTMLButtonElement>("button.connect"),
      "Disconnect offers explicit Reconnect",
    );
    pane().querySelector<HTMLButtonElement>("button.connect")!.click();
    await until(
      () => id() !== old && !!line() && !line()!.disabled,
      "Reconnect creates a new session ID with its own line editor",
    );
    await until(
      () => /login:\s*$/i.test(rows()),
      "Reconnected server waits at login without automatic commands",
    );
    await sleep(600);
    if (
      (await trace()).length !== 0 ||
      line()!.value !== "" ||
      !/login:\s*$/i.test(rows())
    )
      throw Error("Reconnect retained a draft or sent automatic input");
    checks.push(
      unsent
        ? "Reconnect discards the unsent draft and sends zero automatic input events"
        : "New line-mode incarnation sends zero automatic input events before explicit login",
    );
    await login();
  };
  await reconnect(false);
  for (const method of ["Enter", "Send line"] as const) {
    const marker = method === "Enter" ? "READ_ENTER" : "READ_SEND";
    await setLine(
      `printf 'YT_%s\\n' ${marker}; read -r yt_line; printf 'YT_LINE:%s\\n' "$yt_line"`,
    );
    enterLine();
    await until(
      () => rows().includes("YT_" + marker) && !line()!.disabled,
      "Real shell read waits for a line: " + method,
    );
    const count = (await trace()).length;
    await setLine("discarded draft");
    await setLine(method + " Ω 日");
    line()!.blur();
    await sleep(600);
    if (
      (await trace()).length !== count ||
      rows().includes("YT_LINE:" + method)
    )
      throw Error("Draft edit or blur sent input before explicit submission");
    checks.push(
      "Typing, editing and blur send no bytes while the real shell waits: " +
        method,
    );
    if (method === "Enter") enterLine();
    else button("Send line").click();
    await until(
      () =>
        rows().includes("YT_LINE:" + method + " Ω 日") && line()!.value === "",
      method + " submits only the final UTF-8 line to the real shell",
    );
  }
  await setLine("UNSENT_RECONNECT");
  await reconnect(true);

  await send(
    "PS1=''; printf '\\033[3J\\033[2J\\033[H'; printf 'TELNET external qualification\\nUTF-8: Ω 日\\nReal telnetd: login, resize, interrupt, line editor, reconnect, flood\\n'\r",
  );
  await until(
    () =>
      rows().includes("TELNET external qualification") &&
      !rows().includes(profile.username) &&
      !rows().includes(profile.host),
    "Real terminal is cleared of account and endpoint before retaining evidence",
  );
  // The clear-screen/scrollback sequences and visible output above come from
  // the real server. Redact only endpoint/account labels afterwards.
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const node = walker.currentNode;
    // Replace endpoint first; both fields are validated nonempty by the backend.
    node.textContent = node
      .textContent!.replaceAll(profile.host, "<test-host>")
      .replaceAll(profile.username, "<test-account>");
  }
  for (const element of document.querySelectorAll<HTMLElement>("[title]"))
    element.title = element.title
      .replaceAll(profile.host, "<test-host>")
      .replaceAll(profile.username, "<test-account>");
  checks.push(
    "Retained native screenshot redacts endpoint/account after all real protocol checks",
  );
  checks.push(
    "Synthetic native input only: physical keyboard, IME, DPI and performance remain unqualified",
  );
}
