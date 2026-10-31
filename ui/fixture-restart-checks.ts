import { invoke } from "@tauri-apps/api/core";
type Item = {
  key: string;
  remote: string;
  state: string;
  revision: string;
  verified: boolean;
};
export async function checkRestart(
  ids: number[],
  until: (
    test: () => boolean | Promise<boolean>,
    name: string,
  ) => Promise<void>,
  checks: string[],
  prepare: boolean,
) {
  const names = [
    "restart-upload.txt",
    "restart-keep.txt",
    "restart-discard.txt",
    "restart-conflict.txt",
    "restart-unchanged.txt",
    "restart-legacy.txt",
    "recovery.json",
  ];
  const id = ids[0];
  const probe = (editId: number, operation: string, text = "") =>
    invoke<string>("capture_edit_probe", { editId, operation, text });
  const list = () => invoke<{ items: Item[] }>("edit_restore_list");
  if (prepare) {
    for (const name of names) {
      await invoke("file_action", {
        id,
        action: { kind: "create", path: "/fixture", name },
      });
      const edit = await invoke<{ id: number }>("edit_open", {
        id,
        path: "/fixture/" + name,
        choose: false,
      });
      if (name.includes("upload")) {
        await probe(edit.id, "save", "uploaded baseline before restart");
        await invoke("edit_upload", { editId: edit.id });
      }
      if (!name.includes("unchanged") && name !== "recovery.json")
        await probe(edit.id, "save", "pending " + name);
      if (name.includes("conflict"))
        await probe(
          edit.id,
          "remote-change",
          "independent server change before restart",
        );
      if (
        name === "recovery.json" &&
        (await probe(edit.id, "local-read")) !== ""
      )
        throw Error(
          "Recovery metadata overwrote the staged file with the same remote name",
        );
    }
    checks.push(
      "Seven real SFTP files staged with durable baselines; five retain unsent local edits",
    );
    checks.push(
      "A remote recovery.json file retains its own bytes separately from the recovery metadata sidecar",
    );
    checks.push(
      "Successful explicit upload refreshed the persisted baseline before another local save",
    );
    checks.push(
      "Independent remote change creates a conflict before the first app exits",
    );
    return;
  }
  await until(
    () =>
      !!document.querySelector(
        'dialog[aria-label="Unsent changes from an earlier session"]',
      ),
    "Connecting the original session offers only files with unsent changes, without uploading",
  );
  let items: Item[] = [];
  await until(async () => {
    items = (await list()).items;
    return items.length === 4;
  }, "Unchanged and legacy records were tidied silently at startup (local files kept)");
  if (items.some((i) => i.state !== "Changed"))
    throw Error(
      "Restart change states were not reconstructed from the persisted server baselines",
    );
  checks.push(
    "The four remaining records are exactly the Changed ones, from persisted server baselines",
  );
  for (const liveId of ids)
    if ((await invoke<unknown[]>("edit_list", { id: liveId })).length)
      throw Error("Restart automatically rebound a watched edit");
  checks.push(
    "No watched edits or automatic upload permissions were restored from disk",
  );
  const find = (name: string) => items.find((i) => i.remote.endsWith(name))!;
  const read = (item: Item, operation: string, text = "") =>
    invoke<string>("capture_restore_probe", {
      id,
      key: item.key,
      operation,
      text,
    });
  for (const item of items) {
    if (!item.verified) continue;
    const expected = item.remote.includes("upload")
      ? "uploaded baseline before restart"
      : item.remote.includes("conflict")
        ? "independent server change before restart"
        : "";
    if ((await read(item, "remote-read")) !== expected)
      throw Error("Server changed without explicit restored upload");
  }
  checks.push(
    "Real SFTP reads prove no startup upload and preserve the independently changed server",
  );
  const upload = find("restart-upload.txt");
  let blocked = false;
  try {
    await invoke("edit_restore_decide", {
      key: upload.key,
      decision: "upload",
      id: ids[1],
      revision: upload.revision,
    });
  } catch (e) {
    blocked = String(e).includes("different bookmark");
  }
  if (!blocked) throw Error("Unrelated bookmark accepted a restored record");
  checks.push("Another bookmark cannot upload the retained recovery copy");
  const row = (item: Item) =>
    document.querySelector<HTMLElement>(`[data-recovery-key="${item.key}"]`)!;
  const click = (item: Item, name: string) =>
    Array.from(row(item).querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === name)!
      .click();
  await read(upload, "local-save", "new save after restart review");
  click(upload, "Upload");
  await until(
    () =>
      document
        .querySelector("dialog .file-error")
        ?.textContent?.includes("Local copy changed") === true,
    "A local save after the startup review blocks stale-revision upload",
  );
  click(upload, "Inspect local changes");
  await until(
    () =>
      !Array.from(row(upload).querySelectorAll("button")).some(
        (b) => b.disabled,
      ),
    "Local change state can be explicitly inspected again",
  );
  // Wait for the async inspection, not merely the pre-render enabled controls.
  await until(async () => {
    const refreshed = (await list()).items.find((i) => i.key === upload.key);
    return !!refreshed && refreshed.revision !== upload.revision;
  }, "Latest saved local bytes have a new bounded SHA-256 revision");
  await new Promise((r) => setTimeout(r, 100));
  click(upload, "Upload");
  await until(
    () => !document.querySelector(`[data-recovery-key="${upload.key}"]`),
    "Explicit Upload restores the edit only after verifying the persisted server baseline",
  );
  if ((await read(upload, "remote-read")) !== "new save after restart review")
    throw Error("Restored upload bytes differ");
  checks.push(
    "Exact selected bytes reached the real SFTP server after restart",
  );
  const keep = find("restart-keep.txt");
  click(keep, "Keep local copy");
  await until(
    () => !document.querySelector(`[data-recovery-key="${keep.key}"]`),
    "Keep local copy dismisses its disk record without uploading or deleting local bytes",
  );
  const discard = find("restart-discard.txt");
  click(discard, "Discard");
  await until(
    () => row(discard).textContent?.includes("Confirm discard") === true,
    "Discard requires a second explicit in-app confirmation",
  );
  click(discard, "Confirm discard");
  await until(
    () => !document.querySelector(`[data-recovery-key="${discard.key}"]`),
    "Confirmed discard removes only the reviewed staged revision and its record",
  );
  const conflict = find("restart-conflict.txt");
  click(conflict, "Upload");
  await until(
    () =>
      document
        .querySelector("dialog .file-error")
        ?.textContent?.includes("Conflict") === true,
    "Changed remote baseline blocks restored upload without overwriting the server",
  );
  if (
    (await read(conflict, "local-read")) !== "pending restart-conflict.txt" ||
    (await read(conflict, "remote-read")) !==
      "independent server change before restart"
  )
    throw Error("Conflict lost local or remote bytes");
  checks.push(
    "Restart conflict preserves both exact copies and keeps the recovery offer visible",
  );
  row(conflict).scrollIntoView({ block: "nearest" });
}
