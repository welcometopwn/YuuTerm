import type { MutableRefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { HandOff } from "./TerminalPane";
import { fixtureTerminals } from "./fixture-terminals";

const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));
const screen = () => {
  const host = document.querySelector<HTMLElement>(".terminal-host");
  const term = host && fixtureTerminals.get(host);
  if (!term) return "";
  const b = term.buffer.active;
  return Array.from(
    { length: b.length },
    (_, i) => b.getLine(i)?.translateToString(true) ?? "",
  ).join("\n");
};
async function until(f: () => boolean | Promise<boolean>, label: string) {
  const end = Date.now() + 15000;
  while (Date.now() < end) {
    if (await f()) return label;
    await pause(100);
  }
  throw Error(label);
}

// Runs inside the pop-out window of a capture build: the content moved
// over, typing reaches the session, and every general command is refused.
export async function checkPopoutWindow(
  handOff: MutableRefObject<HandOff | null>,
  handBack: () => Promise<void>,
) {
  const checks: string[] = [];
  let passed = false;
  try {
    checks.push(
      await until(
        () => screen().includes("ui-stream-check"),
        "The pop-out shows the main window's terminal content",
      ),
    );
    for (const [name, args] of [
      ["session_input", { id: 1, bytes: [65] }],
      ["session_poll", { id: 1 }],
      ["bootstrap", {}],
      ["save_appearance", {}],
      ["popout_open", { id: 1, title: "x", fontSize: 14, snapshot: "" }],
      ["popout_take_returned", { id: 1 }],
      ["capture_report", { checks: [], passed: true }],
    ] as const) {
      const refused = await invoke(name, args).then(
        () => false,
        () => true,
      );
      if (!refused) throw Error(`${name} was allowed from the pop-out`);
    }
    checks.push("General and other-session commands are refused here");
    await invoke("popout_input", {
      bytes: Array.from(new TextEncoder().encode("popout-typed\r")),
    });
    checks.push(
      await until(
        () => screen().includes("popout-typed"),
        "Typing in the pop-out reaches the session and echoes",
      ),
    );
    if (!handOff.current) throw Error("Hand-off is not ready");
    passed = true;
  } catch (e) {
    checks.push(("FAILED: " + String(e)).slice(0, 390));
  }
  await invoke("capture_popout_report", { checks, passed });
  if (passed) await handBack();
}

// Main window side of the "popout" capture scope.
export async function checkPopoutMain(
  until: (f: () => boolean | Promise<boolean>, name: string) => Promise<void>,
) {
  const pane = () =>
    document.querySelector<HTMLElement>("section[data-profile-id]")!;
  const button = () =>
    pane().querySelector<HTMLButtonElement>(
      'button[aria-label="Open in own window"]',
    );
  await until(
    () => !!button() && !button()!.disabled,
    "Connected terminal panes offer Open in own window",
  );
  button()!.click();
  await until(
    () => !!pane().querySelector(".terminal-popped"),
    "The main pane shows that the terminal is in its own window",
  );
  let report: { checks: string[]; passed: boolean } | null = null;
  await until(async () => {
    report = await invoke<{ checks: string[]; passed: boolean } | null>(
      "capture_popout_result",
    );
    return !!report;
  }, "The pop-out window reports its own checks");
  const result = report as unknown as { checks: string[]; passed: boolean };
  for (const check of result.checks)
    await until(() => true, `Pop-out: ${check}`);
  if (!result.passed) throw Error("Pop-out window checks failed");
  await until(() => {
    const host = pane().querySelector<HTMLElement>(".terminal-host");
    const term = host && fixtureTerminals.get(host);
    if (!term || pane().querySelector(".terminal-popped")) return false;
    const b = term.buffer.active;
    return Array.from(
      { length: b.length },
      (_, i) => b.getLine(i)?.translateToString(true) ?? "",
    )
      .join("\n")
      .includes("popout-typed");
  }, "Closing the pop-out brings the session and its new output back");
  // Leave a second pop-out open for the capture of the main window.
  button()!.click();
  await until(
    () => !!pane().querySelector(".terminal-popped"),
    "The session can be popped out again",
  );
  await until(async () => {
    const ids = await invoke<string[]>("capture_popout_taskbar_ids");
    return ids.length === 1 && /\.yuuterm\.term-\d+$/.test(ids[0]);
  }, "The pop-out has its own taskbar ID instead of the YuuTerm group");
}
