import { invoke } from "@tauri-apps/api/core";
export async function checkRecovery(
  ids: number[],
  until: (
    test: () => boolean | Promise<boolean>,
    label: string,
  ) => Promise<void>,
  checks: string[],
) {
  const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const action = (label: string) =>
    Array.from(document.querySelectorAll<HTMLButtonElement>("button")).find(
      (b) => b.textContent?.trim() === label,
    )!;
  const probe = (editId: number, operation: string, text = "") =>
    invoke<string>("capture_edit_probe", { editId, operation, text });
  const pane = () =>
    document.querySelector<HTMLElement>('section[data-profile-id="1"]')!;
  const liveId = () => Number(pane().dataset.liveId);
  await until(
    () => document.querySelectorAll(".file-row").length >= 2,
    "Recovery fixture has an active SSH browser",
  );
  document
    .querySelector<HTMLButtonElement>('[aria-label="Actions for read me.txt"]')!
    .parentElement!.querySelector("button")!
    .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  await until(
    () =>
      document
        .querySelector(".files-panel")
        ?.getAttribute("data-watched-edits") === "1",
    "A local edit is staged before disconnection",
  );
  const [edit] = await invoke<{ id: number; local: string }[]>("edit_list", {
    id: ids[0],
  });
  const original = await probe(edit.id, "read");
  await probe(edit.id, "save", "saved before connection was lost");
  await until(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "Unsent local save prompts on its original connection",
  );
  action("Auto-upload all").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Original connection explicitly enables auto-upload",
  );
  // Disconnect before the next local save, retaining it for explicit recovery.
  await invoke("session_disconnect", { id: ids[0] });
  await until(
    () =>
      Array.from(pane().querySelectorAll("button")).some(
        (b) => b.textContent?.trim() === "Reconnect",
      ),
    "Disconnected terminal offers explicit reconnect",
  );
  await probe(edit.id, "atomic-save", "local work saved while disconnected");
  action("Reconnect").click();
  await until(
    () =>
      liveId() !== ids[0] &&
      !!document.querySelector('dialog[aria-label="Resume local edit"]'),
    "New connection asks about retained local edits without rebinding automatically",
  );
  const replacement = liveId();
  if ((await invoke<unknown[]>("edit_list", { id: replacement })).length)
    throw new Error("Edit silently rebound after reconnect");
  checks.push(
    "Reconnect assigns a fresh live ID and does not silently resume watched edits",
  );
  // Another bookmark targeting the same endpoint is not the owning bookmark.
  let denied = false;
  try {
    await invoke("edit_resume", { id: ids[1], editId: edit.id });
  } catch {
    denied = true;
  }
  if (!denied) throw new Error("Unrelated bookmark accepted recovery");
  checks.push(
    "Recovery refuses another bookmark even when its network endpoint matches",
  );
  await until(
    () => !!document.querySelector('dialog[aria-label="Resume local edit"]'),
    "Recovery is reviewed in a separate explicit dialog",
  );
  action("Verify and resume").click();
  await until(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "Verified recovery prompts for the retained local changes rather than uploading automatically",
  );
  if ((await probe(edit.id, "read")) !== "saved before connection was lost")
    throw new Error("Recovery uploaded without approval");
  if (
    (await probe(edit.id, "local-read")) !==
    "local work saved while disconnected"
  )
    throw new Error("Recovery replaced local work");
  checks.push(
    "Recovery preserves unsent local bytes and the server version until approval",
  );
  const resumed = (
    await invoke<{ id: number; local: string; live_id: number }[]>(
      "edit_list",
      { id: replacement },
    )
  )[0];
  if (
    resumed.id !== edit.id ||
    resumed.local !== edit.local ||
    resumed.live_id !== replacement
  )
    throw new Error("Recovery changed the staged path or edit identity");
  checks.push(
    "Resuming keeps the same staged path so an already open editor can continue saving",
  );
  action("Yes, upload").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Approved recovered version uploads on the new connection",
  );
  if ((await probe(edit.id, "read")) !== "local work saved while disconnected")
    throw new Error("Recovered upload failed");
  await probe(edit.id, "save", "another save after recovery");
  await until(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "Automatic permission from the original connection is not inherited",
  );
  action("No, keep local").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Declined save after recovery is retained locally",
  );
  await probe(
    edit.id,
    "remote-change",
    "independent server modification before another reconnect",
  );
  await until(
    () => !!document.querySelector('dialog[aria-label="Server file changed"]'),
    "Remote change is visible before disconnecting a second time",
  );
  await invoke("session_disconnect", { id: replacement });
  await until(
    () => !document.querySelector("dialog[open]"),
    "Disconnect dismisses a remote-reload prompt bound to the old connection",
  );
  await until(
    () =>
      Array.from(pane().querySelectorAll("button")).some(
        (b) => b.textContent?.trim() === "Reconnect",
      ),
    "Second disconnection can be explicitly reconnected",
  );
  action("Reconnect").click();
  await until(
    () =>
      liveId() !== replacement &&
      !!document.querySelector('dialog[aria-label="Resume local edit"]'),
    "Changed server still retains a visible local recovery candidate",
  );
  await until(
    () => !!document.querySelector('dialog[aria-label="Resume local edit"]'),
    "Second recovery requires another review",
  );
  action("Verify and resume").click();
  await until(
    () =>
      document
        .querySelector("dialog .file-error")
        ?.textContent?.includes("Conflict") === true,
    "Full remote content verification blocks recovery after an independent server modification",
  );
  if ((await probe(edit.id, "local-read")) !== "another save after recovery")
    throw new Error("Conflict destroyed local recovery bytes");
  if ((await invoke<unknown[]>("edit_list", { id: liveId() })).length)
    throw new Error("Conflict rebound the edit");
  checks.push(
    "Conflict retains local work and leaves the new connection unbound",
  );
  if (!original) throw new Error("Fixture original unexpectedly empty");
  await pause(100);
}
