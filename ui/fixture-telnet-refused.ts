import { invoke } from "@tauri-apps/api/core";

export async function checkTelnetRefused() {
  const checks: string[] = [];
  try {
    const panes = [...document.querySelectorAll<HTMLElement>(".pane")];
    if (!panes.length) throw Error("No refused TELNET pane");
    for (const pane of panes) {
      if (
        pane.querySelector(".pane-title")?.textContent?.trim() !==
        "Disconnected"
      )
        throw Error("Finished attempt must say Disconnected");
      const banner =
        pane.querySelector(".connection-message.ended")?.textContent ?? "";
      if (!banner || /Connecting/.test(banner) || !/10061/.test(banner))
        throw Error("Refused TCP attempt must show the actual Windows refusal");
      if (!pane.querySelector(".plaintext-warning"))
        throw Error("Missing plaintext warning");
      if (
        ![...pane.querySelectorAll("button")].some(
          (b) => b.textContent?.trim() === "Reconnect",
        )
      )
        throw Error("Finished attempt must offer Reconnect");
    }
    checks.push(
      "Real refused loopback TCP attempts finish with Disconnected and Windows error 10061",
    );
    checks.push(
      "Every finished pane replaces Connecting with the final reason and offers Reconnect",
    );
    checks.push("TELNET plaintext warning remains visible");
    checks.push(
      "Locked desktop: native functional check only, no focus/performance conclusion",
    );
    await invoke("capture_report", { checks, passed: true });
    await invoke("capture_ready", { terminalReady: true });
  } catch (error) {
    await invoke("capture_report", {
      checks: [...checks, String(error)],
      passed: false,
    });
  }
}
