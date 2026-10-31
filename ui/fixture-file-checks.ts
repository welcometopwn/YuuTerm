import { invoke } from "@tauri-apps/api/core";
export async function checkFileWorkflow(
  ids: number[],
  until: (p: () => boolean | Promise<boolean>, label: string) => Promise<void>,
  checks: string[],
) {
  const pause = (ms = 80) => new Promise((r) => setTimeout(r, ms));
  const button = (label: string) =>
    Array.from(
      document.querySelectorAll<HTMLButtonElement>(
        "dialog button, .context-menu button",
      ),
    ).find((b) => b.textContent?.trim() === label)!;
  const row = (name: string) =>
    document.querySelector<HTMLButtonElement>(
      `button[aria-label="Actions for ${name}"]`,
    )!.parentElement!;
  const double = (name: string) =>
    row(name)
      .querySelector("button")!
      .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  const menu = async (name: string) => {
    document
      .querySelector<HTMLButtonElement>(
        `button[aria-label="Actions for ${name}"]`,
      )!
      .click();
    await pause(150);
  };
  const probe = (id: number, operation: string, text = "") =>
    invoke<string>("capture_edit_probe", { editId: id, operation, text });
  await until(
    () =>
      document.querySelectorAll(".file-row:not(.parent-directory)").length ===
        6 && document.querySelectorAll(".session-statistics").length === 4,
    "SSH connection automatically shows its file browser and six isolated fixture entries",
  );
  await until(() => {
    const file = row("read me.txt");
    return (
      file
        .querySelector(".file-size")
        ?.getAttribute("title")
        ?.startsWith("42 bytes") === true &&
      file.querySelector(".file-owner")?.textContent?.trim() ===
        "fixture-user / fixture-group" &&
      file.querySelector(".file-modified")?.getAttribute("title") ===
        "2023-11-14T22:13:20.000Z"
    );
  }, "File columns display real SFTP size, last-modified time and resolved owner/group names");
  document
    .querySelector<HTMLButtonElement>('[aria-label="Widen file browser"]')!
    .click();
  await until(
    () =>
      document.querySelector(".sidebar")!.getBoundingClientRect().width >= 576,
    "File browser can widen to show all detail columns together",
  );
  const separator = document.querySelector<HTMLElement>(
    '[aria-label="Resize sidebar"]',
  )!;
  const widthBefore = document
    .querySelector(".sidebar")!
    .getBoundingClientRect().width;
  separator.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "ArrowRight",
      bubbles: true,
      cancelable: true,
    }),
  );
  await until(
    () =>
      document.querySelector(".sidebar")!.getBoundingClientRect().width ===
      widthBefore + 20,
    "Sidebar separator resizes the actual grid without reconnecting",
  );
  if (
    row("read me.txt").querySelector(".file-owner")?.getAttribute("title") !==
    "fixture-user / fixture-group\nUID: 1000 / GID: 1000"
  )
    throw Error("Named ownership lost its numeric tooltip");
  checks.push("Named owners retain exact UID/GID in the tooltip");
  const sortBy = (column: string) =>
    document.querySelector<HTMLButtonElement>(
      `[data-sort-column="${column}"]`,
    )!;
  const orderedNames = () =>
    Array.from(document.querySelectorAll<HTMLElement>(".file-row[data-name]"))
      .map((item) => item.dataset.name)
      .join(",");
  sortBy("name").click();
  await until(
    () =>
      orderedNames() ===
      "logs,read me.txt,file-link,dir-link,cycle-link,broken-link",
    "Name header reverses the displayed rows while directories stay first",
  );
  row("read me.txt").querySelector("button")!.click();
  row("cycle-link")
    .querySelector("button")!
    .dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
  await until(
    () => document.querySelectorAll(".file-row.file-selected").length === 4,
    "Shift selection uses the visible sorted row order",
  );
  row("cycle-link")
    .querySelector("button")!
    .dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
  for (const column of ["size", "modified", "owner"]) {
    sortBy(column).click();
    await until(
      () => sortBy(column).dataset.direction === "asc",
      `${column} header selects ascending sort`,
    );
    sortBy(column).click();
    await until(
      () => sortBy(column).dataset.direction === "desc",
      `${column} header toggles descending sort`,
    );
  }
  for (const column of ["name", "size", "modified", "owner"]) {
    const handle = document.querySelector<HTMLElement>(
      `.file-column[data-column="${column}"] [role="separator"]`,
    )!;
    const before = row("read me.txt")
      .querySelector<HTMLElement>(`.file-${column}`)!
      .getBoundingClientRect().width;
    handle.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "ArrowRight",
        bubbles: true,
        cancelable: true,
      }),
    );
    await until(
      () =>
        row("read me.txt")
          .querySelector<HTMLElement>(`.file-${column}`)!
          .getBoundingClientRect().width ===
        before + 16,
      `${column} column resize updates actual row cells`,
    );
    const headerBox = handle.parentElement!.getBoundingClientRect();
    const cellBox = row("read me.txt")
      .querySelector<HTMLElement>(`.file-${column}`)!
      .getBoundingClientRect();
    const iconWidth = 0;
    if (
      Math.abs(headerBox.left + iconWidth - cellBox.left) > 1 ||
      Math.abs(headerBox.width - iconWidth - cellBox.width) > 1
    )
      throw Error(
        `${column} header and row cells became misaligned after resizing`,
      );
    checks.push(`${column} header and row cells remain aligned after resizing`);
    handle.dispatchEvent(
      new MouseEvent("dblclick", { bubbles: true, cancelable: true }),
    );
    await until(
      () =>
        row("read me.txt")
          .querySelector<HTMLElement>(`.file-${column}`)!
          .getBoundingClientRect().width === before,
      `${column} column double-click resets its width`,
    );
  }
  sortBy("name").click();
  await until(
    () => sortBy("name").dataset.direction === "asc",
    "Name order can be restored without reconnecting",
  );
  await pause(200);
  await invoke("capture_still", { name: "file-metadata" });
  await pause(1500);
  if (row("logs").querySelector(".file-modified")?.textContent?.trim() !== "—")
    throw Error("Missing remote metadata was invented");
  checks.push("Missing listing metadata remains explicitly unknown");
  await until(
    () =>
      document.querySelectorAll('[aria-label="Symbolic link"]').length === 4,
    "File browser visibly identifies file, folder, dangling and cyclic symlinks",
  );
  double("dir-link");
  await until(
    () =>
      document.querySelector<HTMLInputElement>('[aria-label="Remote path"]')
        ?.value === "/fixture/logs" &&
      !!document.querySelector('[aria-label="Actions for inner.txt"]'),
    "Opening a directory symlink navigates to its canonical target",
  );
  document
    .querySelector<HTMLButtonElement>('[aria-label="Parent directory"]')!
    .click();
  await until(
    () =>
      document.querySelector<HTMLInputElement>('[aria-label="Remote path"]')
        ?.value === "/fixture",
    "Parent navigation returns from the resolved link target",
  );
  row("file-link")
    .querySelector("button")!
    .dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        clientX: 80,
        clientY: 160,
      }),
    );
  await until(() => {
    const menu = document.querySelector<HTMLElement>(".context-menu");
    if (!menu) return false;
    const box = menu.getBoundingClientRect();
    return (
      !document.querySelector("dialog[open]") &&
      box.width <= 225 &&
      box.height <= 325 &&
      Math.abs(box.left - 80) < 2 &&
      Math.abs(box.top - 160) < 2
    );
  }, "File right click opens a compact menu at the pointer without a modal");
  button("Download").focus();
  await pause(1200);
  if (document.activeElement !== button("Download"))
    throw new Error("Live updates reset context-menu keyboard focus");
  checks.push("Context-menu keyboard focus survives live statistics updates");
  button("Open containing directory").click();
  await until(
    () =>
      !document.querySelector("dialog[open]") &&
      !document.querySelector(".context-menu") &&
      !document.querySelector<HTMLButtonElement>(
        '[aria-label="Actions for file-link"]',
      )?.disabled &&
      document.querySelector<HTMLInputElement>('[aria-label="Remote path"]')
        ?.value === "/fixture",
    "File-link context menu opens the target containing directory",
  );
  double("file-link");
  await until(
    () =>
      document
        .querySelector(".files-panel")
        ?.getAttribute("data-watched-edits") === "1",
    "File symlink opens a staged edit without launching an external editor in capture mode",
  );
  const edits = await invoke<{ id: number; remote: string }[]>("edit_list", {
    id: ids[0],
  });
  const edit = edits[0];
  if (edit.remote !== "/fixture/read me.txt")
    throw new Error("Local edit was not bound to canonical target");
  checks.push(
    "Staged edits bind to the canonical file target, preserving the symlink itself",
  );
  const original = await probe(edit.id, "read");
  await menu("file-link");
  button("Open in nano").click();
  await until(async () => {
    const values = await invoke<number[][]>("capture_trace", { id: ids[0] });
    return new TextDecoder()
      .decode(new Uint8Array(values.flat()))
      .includes("nano -- '/fixture/read me.txt'\r");
  }, "Nano action sends a quoted command with Enter only to the original SSH terminal");
  for (const id of ids.slice(1)) {
    const values = await invoke<number[][]>("capture_trace", { id });
    if (
      new TextDecoder()
        .decode(new Uint8Array(values.flat()))
        .includes("nano --")
    )
      throw new Error("Nano leaked into another terminal");
  }
  checks.push("Nano is not broadcast to other live terminals");
  double("broken-link");
  await until(
    () => !!document.querySelector(".app-notice"),
    "Dangling-link failure is shown without opening a local editor",
  );
  await until(() => {
    const notice = document.querySelector<HTMLElement>(".app-notice")!;
    const bars = Array.from(
      document.querySelectorAll<HTMLElement>(".session-statistics"),
    );
    return (
      notice.getBoundingClientRect().bottom <=
      window.innerHeight -
        Math.max(...bars.map((b) => b.getBoundingClientRect().height))
    );
  }, "Error notice stays above the live statistics bars");
  document
    .querySelector<HTMLButtonElement>('[aria-label="Dismiss notification"]')!
    .click();
  await probe(edit.id, "save", "declined local save");
  await until(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "Saving a local staged file opens Yes, No and auto-upload confirmation",
  );
  const order = Array.from(
    document.querySelectorAll(
      'dialog[aria-label="Upload saved changes?"] .dialog-actions button',
    ),
  )
    .map((b) => b.textContent?.trim())
    .join("|");
  if (order !== "Yes, upload|Auto-upload all|No, keep local")
    throw new Error("Incorrect upload button order");
  checks.push("Upload actions are ordered Yes, Auto-upload, No");
  button("No, keep local").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "No dismisses this saved revision while retaining the local copy",
  );
  if ((await probe(edit.id, "read")) !== original)
    throw new Error("Declined save changed remote file");
  checks.push("Declining upload leaves the remote file unchanged");
  await probe(edit.id, "save", "temporary edit before reverting");
  await until(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "A temporary saved edit opens a prompt",
  );
  await probe(edit.id, "atomic-save", "declined local save");
  await until(
    () => !document.querySelector("dialog[open]"),
    "Reverting to the acknowledged local contents dismisses the obsolete upload prompt",
  );
  await probe(edit.id, "save", "accepted local save");
  await until(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "A later save prompts again after declining the previous revision",
  );
  await probe(edit.id, "atomic-save", "latest save while prompt is open");
  await pause(2600);
  button("Yes, upload").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Yes uploads the selected saved revision after conflict checks",
  );
  if ((await probe(edit.id, "read")) !== "latest save while prompt is open")
    throw new Error("Accepted upload content mismatch");
  checks.push("Accepted save reaches the real SFTP fixture");
  await probe(edit.id, "save", "enable auto upload save");
  await until(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "Auto-upload is offered explicitly on a saved revision",
  );
  button("Auto-upload all").click();
  await until(
    () =>
      !document.querySelector("dialog[open]") &&
      !!document.querySelector('[aria-label="Ask before uploads"]'),
    "Explicit auto-upload is enabled only for this live connection",
  );
  await probe(edit.id, "save", "next automatic save");
  await pause(3200);
  if (
    (await probe(edit.id, "read")) !== "next automatic save" ||
    document.querySelector("dialog[open]")
  )
    throw new Error("Auto-upload did not publish next saved revision");
  checks.push("Subsequent saves auto-upload without another prompt");
  double("logs");
  await until(
    () => !!document.querySelector('[aria-label="Actions for inner.txt"]'),
    "Nested directory is available for another local edit",
  );
  double("inner.txt");
  await until(
    () =>
      document
        .querySelector(".files-panel")
        ?.getAttribute("data-watched-edits") === "2",
    "A second file can join the watched edits on the same connection",
  );
  const second = (
    await invoke<{ id: number; remote: string }[]>("edit_list", { id: ids[0] })
  ).find((e) => e.remote.endsWith("/inner.txt"))!;
  await probe(second.id, "save", "automatically saved second file");
  await pause(3200);
  if (
    (await probe(second.id, "read")) !== "automatically saved second file" ||
    document.querySelector("dialog[open]")
  )
    throw new Error("Auto-upload policy did not cover the second file");
  checks.push(
    "Auto-upload all also covers newly opened local edits on the same live connection",
  );
  // The watched-edits panel is gone (user decision); release via the backend.
  await invoke("edit_stop", { editId: second.id });
  await until(
    async () =>
      (await invoke<unknown[]>("edit_list", { id: ids[0] })).length === 1,
    "Stop watching releases the edit record without deleting its local recovery copy",
  );
  document
    .querySelector<HTMLButtonElement>('[aria-label="Parent directory"]')!
    .click();
  await until(
    () =>
      document.querySelectorAll(".file-row:not(.parent-directory)").length ===
      6,
    "File browser returns to the symlink listing after editing another file",
  );
  await probe(edit.id, "remote-change", "independent server change");
  await probe(edit.id, "save", "conflicting local save");
  await until(
    () =>
      document
        .querySelector('dialog[aria-label="Upload saved changes?"] .file-error')
        ?.textContent?.includes("Conflict") === true,
    "Remote conflict stops automatic upload and opens an actionable error prompt",
  );
  if ((await probe(edit.id, "read")) !== "independent server change")
    throw new Error("Auto-upload overwrote a remote conflict");
  if (!!document.querySelector('[aria-label="Ask before uploads"]'))
    throw new Error("Conflict left automatic uploads enabled");
  checks.push(
    "Conflicting server content is preserved and auto-upload is revoked",
  );
  button("No, keep local").click();
  await until(
    () => !!document.querySelector('dialog[aria-label="Server file changed"]'),
    "Remote metadata change offers an explicit reload after the local conflict is acknowledged",
  );
  button("Keep local").click();
  await pause(2500);
  if (
    document.querySelector("dialog[open]") ||
    (await probe(edit.id, "local-read")) !== "conflicting local save"
  )
    throw new Error("Keep local did not retain the local version");
  checks.push(
    "Keep local acknowledges only this server version without overwriting disk or enabling upload",
  );
  await probe(
    edit.id,
    "remote-change",
    "a new independent server version to reload",
  );
  await until(
    () => !!document.querySelector('dialog[aria-label="Server file changed"]'),
    "A subsequent server change prompts again",
  );
  await probe(edit.id, "save", "local save during remote prompt");
  button("Reload server version").click();
  await until(
    () =>
      document
        .querySelector("dialog .file-error")
        ?.textContent?.includes("Local file changed") === true,
    "Reload refuses to replace a local version saved after the prompt",
  );
  if (
    (await probe(edit.id, "local-read")) !== "local save during remote prompt"
  )
    throw new Error("Reload overwrote a concurrent save");
  button("Keep local").click();
  await until(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "Concurrent local save still offers its own upload decision",
  );
  button("No, keep local").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Declined concurrent local version stays on disk",
  );
  await probe(
    edit.id,
    "remote-change",
    "confirmed server reload with another length",
  );
  await until(
    () => !!document.querySelector('dialog[aria-label="Server file changed"]'),
    "Another remote change can be explicitly reloaded",
  );
  button("Reload server version").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Confirmed reload replaces the staged disk file",
  );
  if (
    (await probe(edit.id, "local-read")) !==
    "confirmed server reload with another length"
  )
    throw new Error("Server content was not loaded locally");
  const backups = JSON.parse(await probe(edit.id, "backups")) as string[];
  if (!backups.includes("local save during remote prompt"))
    throw new Error("Previous local contents were not backed up");
  checks.push("Reload keeps a recovery backup of the prior local disk version");
  await pause(2500);
  if (document.querySelector("dialog[open]"))
    throw new Error("Reload triggered an upload feedback loop");
  checks.push("Downloaded server version does not upload itself again");
  await probe(edit.id, "save", "save after successful remote reload");
  await until(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "Saving after reload uses the refreshed remote baseline",
  );
  button("Yes, upload").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Upload works after a confirmed remote reload",
  );
  if ((await probe(edit.id, "read")) !== "save after successful remote reload")
    throw new Error("Post-reload save failed");
  await menu("file-link");
}
