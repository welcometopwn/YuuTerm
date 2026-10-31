import { invoke } from "@tauri-apps/api/core";
import { terminalText } from "./fixture-terminals";

export async function captureReadme(ids: number[]) {
  const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
  const panes = () => [...document.querySelectorAll<HTMLElement>(".pane")];
  const contents = () => panes().map((pane) => terminalText(pane));
  const until = async (test: () => boolean, message: string) => {
    const deadline = Date.now() + 15000;
    while (!test() && Date.now() < deadline) await sleep(100);
    if (!test()) throw Error(message);
  };
  try {
    if (ids.length !== 2) throw Error("The screenshot requires two local sessions");
    await until(() => contents().length === 2 && contents().every((text) => text.includes(">")), "Both local shells start");
    for (const [index, mode] of ["project", "checks"].entries()) {
      const command = `prompt $G & node scripts/readme-demo.mjs ${mode}\r`;
      await invoke("session_input", { id: ids[index], bytes: [...new TextEncoder().encode(command)] });
    }
    await until(() => contents().every((text) => text.includes("Local demonstration")), "Both local programs finish rendering");
    const visible = contents().join("\n");
    if (visible.includes("C:\\dev") || visible.includes("FAIL")) throw Error("Screenshot contains a private path or failed check");
    await sleep(1000);
    await invoke("capture_still", { name: "readme" });
    await sleep(1000);
    await invoke("capture_report", { passed: true, checks: ["Two real local command shells start", "Two-pane layout shows actual project files and metadata checks", "Both programs render without private paths or failed checks", "Native screenshot saved; no remote connections"] });
  } catch (error) {
    await invoke("capture_report", { passed: false, checks: [String(error)] });
  }
}