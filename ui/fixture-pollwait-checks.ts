import { invoke } from "@tauri-apps/api/core";
import type { Poll } from "./types";
export async function checkPollWait(ids: number[], checks: string[]) {
  await new Promise((r) => setTimeout(r, 500));
  const elapsed: number[] = [];
  for (let n = 0; n < 5; n++) {
    const at = performance.now();
    const p = await invoke<Poll>("session_poll", { id: ids[0], waitMs: 200 });
    if (p.more || p.chunks.length)
      throw new Error("Idle poll unexpectedly drained output");
    elapsed.push(performance.now() - at);
  }
  elapsed.sort((a, b) => a - b);
  if (elapsed[2] < 150 || elapsed[2] > 1000)
    throw new Error(
      `Idle poll did not wait without blocking UI: median ${Math.round(elapsed[2])} ms`,
    );
  checks.push(
    "Empty backend polls wait for notification or a bounded 200 ms deadline",
  );
  let rejected = false;
  try {
    await invoke("session_poll", { id: ids[0], waitMs: 251 });
  } catch {
    rejected = true;
  }
  if (!rejected) throw new Error("Unbounded poll deadline was accepted");
  checks.push("Backend rejects waits above 250 ms");
  for (const id of ids)
    if (!document.querySelector(`[data-live-id="${id}"]`))
      throw new Error("Poll wait changed a live session");
  checks.push("All four original live SSH identities remain present");
}
