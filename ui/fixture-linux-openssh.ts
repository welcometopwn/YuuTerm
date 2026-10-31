import { invoke } from "@tauri-apps/api/core";
// Synthetic native DOM and disk saves against the explicitly selected Linux server.
export async function checkLinuxOpenSsh(ids: number[], checks: string[]) {
  const id = ids[0];
  const path = await invoke<string>("capture_external_path");
  const wait = async (f: () => boolean | Promise<boolean>, label: string) => {
    const end = Date.now() + 45000;
    while (Date.now() < end) {
      if (await f()) {
        checks.push(label);
        return;
      }
      await new Promise((r) => setTimeout(r, 100));
    }
    throw Error(label);
  };
  const button = (label: string) =>
    Array.from(document.querySelectorAll<HTMLButtonElement>("button")).find(
      (b) => b.textContent?.trim() === label,
    )!;
  const row = (name: string) =>
    document.querySelector<HTMLButtonElement>(
      `[aria-label="Actions for ${name}"]`,
    );
  const rows = () => document.querySelector(".xterm-rows")?.textContent ?? "";
  const send = (command: string) =>
    invoke("session_input", {
      id,
      bytes: Array.from(new TextEncoder().encode(command + "\r")),
    });
  const setValue = (input: HTMLInputElement, value: string) => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  };
  // Markers are assembled by the shell so the echoed command cannot match.
  await send(
    "unset HISTFILE; PS1='YuuTerm test$ '; clear; printf 'native-%s\\n' bash; printf '\\316\\251\\n'",
  );
  await wait(
    () => rows().includes("native-bash") && rows().includes("Ω"),
    "Actual native terminal renders bash and Unicode output from Linux OpenSSH",
  );
  const input = document.querySelector<HTMLInputElement>(
    '[aria-label="Remote path"]',
  )!;
  setValue(input, path + "/tree/../");
  await new Promise((r) => setTimeout(r, 0));
  input.form!.requestSubmit();
  await wait(
    () => input.value === path && !!row("editor.txt"),
    "File browser canonicalizes a dot-dot path into the real SFTP directory",
  );
  for (const [label, name] of [
    ["New directory", "native-dir"],
    ["New file", "native-empty.txt"],
  ]) {
    document
      .querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!
      .click();
    await wait(
      () => !!document.querySelector('[aria-label="Filename"]'),
      `${label} opens inline naming`,
    );
    const nameInput = document.querySelector<HTMLInputElement>(
      '[aria-label="Filename"]',
    )!;
    setValue(nameInput, name);
    await new Promise((r) => setTimeout(r, 0));
    nameInput.form!.requestSubmit();
    await wait(() => !!row(name), `${label} creates a real Linux SFTP entry`);
  }
  const rect = document.querySelector(".files-panel")!.getBoundingClientRect();
  const dropped = await invoke<string>("capture_simulate_drop", {
    x: (rect.left + rect.width / 2) * devicePixelRatio,
    y: (rect.top + rect.height / 2) * devicePixelRatio,
  });
  await wait(
    () => !!row(dropped),
    "Native drop token/event path uploads into the actual Linux SFTP directory",
  );
  const download = await invoke<{ completed: number; problem: string | null }>(
    "files_download",
    { id, paths: [path + "/tree"] },
  );
  if (download.completed !== 1 || download.problem)
    throw Error("Linux directory download failed");
  checks.push(
    "Normal directory-download backend completes against Linux OpenSSH; runner independently checks exact bytes",
  );
  await invoke("file_action", {
    id,
    action: {
      kind: "rename",
      path: path + "/native-empty.txt",
      name: "native renamed ' Ω.txt",
    },
  });
  await wait(
    () => !!row("native renamed ' Ω.txt"),
    "Rename preserves spaces, apostrophe and Unicode on Linux",
  );
  await send(
    `test -e "${path}/native renamed ' Ω.txt" && printf 'YT_%s\\n' PATH_OK`,
  );
  await wait(
    () => rows().includes("YT_PATH_OK"),
    "bash resolves the renamed filename with spaces, apostrophe and Unicode",
  );
  row("native renamed ' Ω.txt")!.click();
  await wait(() => !!button("Delete"), "File context menu exposes delete");
  button("Delete").click();
  await wait(
    () => !!button("Delete permanently"),
    "Deletion requires explicit confirmation",
  );
  button("Delete permanently").click();
  await wait(
    () =>
      !row("native renamed ' Ω.txt") && !document.querySelector("dialog[open]"),
    "Confirmed Linux file deletion refreshes the browser",
  );
  // Double-clicking a symlink opens its resolved directory.
  row("tree-link")!
    .parentElement!.querySelector("button")!
    .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  await wait(
    () => input.value === path + "/tree" && !!row("space Ω.txt"),
    "Opening a directory symlink browses its resolved target",
  );
  setValue(input, path);
  await new Promise((r) => setTimeout(r, 0));
  input.form!.requestSubmit();
  await wait(
    () => input.value === path && !!row("editor.txt"),
    "Explicit path entry returns to the test run",
  );
  row("editor.txt")!
    .parentElement!.querySelector("button")!
    .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  await wait(
    () =>
      document
        .querySelector(".files-panel")
        ?.getAttribute("data-watched-edits") === "1",
    "Real Linux remote file stages for external editing without launching an installed editor",
  );
  const [edit] = await invoke<{ id: number }[]>("edit_list", { id });
  const probe = (operation: string, text = "") =>
    invoke<string>("capture_edit_probe", { editId: edit.id, operation, text });
  await probe("save", "native explicit save\nΩ\n");
  await wait(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "Local disk save prompts before uploading to Linux",
  );
  button("Yes, upload").click();
  await wait(
    () => !document.querySelector("dialog[open]"),
    "Explicitly approved edit upload completes",
  );
  if ((await probe("read")) !== "native explicit save\nΩ\n")
    throw Error("Linux edit upload changed Unicode/LF bytes");
  checks.push(
    "Approved editor upload preserves Unicode and LF on the real server",
  );
  await probe("save", "native automatic save\n");
  await wait(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "Automatic upload remains explicit opt-in",
  );
  button("Auto-upload all").click();
  await wait(
    async () =>
      !document.querySelector("dialog[open]") &&
      (await probe("read")) === "native automatic save\n",
    "Opted-in automatic editor upload reaches Linux OpenSSH",
  );
  await probe("remote-change", "independent Linux server content\n");
  await probe("save", "conflicting local Linux edit\n");
  await wait(
    () =>
      document
        .querySelector('dialog[aria-label="Upload saved changes?"] .file-error')
        ?.textContent?.includes("Conflict") === true,
    "A real server-side change blocks conflicting automatic upload",
  );
  if (
    (await probe("read")) !== "independent Linux server content\n" ||
    (await probe("local-read")) !== "conflicting local Linux edit\n"
  )
    throw Error("Conflict lost one of the copies");
  checks.push(
    "Conflict preserves both the server version and local saved copy",
  );
  button("No, keep local").click();
  await wait(
    () => !!document.querySelector('dialog[aria-label="Server file changed"]'),
    "Conflict offers explicit remote-change review",
  );
  button("Keep local").click();
  await wait(
    () => !document.querySelector("dialog[open]"),
    "Keeping local leaves the server conflict intact",
  );
  row("editor.txt")!.click();
  await wait(() => !!button("Permissions"), "Linux file exposes permissions");
  button("Permissions").click();
  await wait(
    () => !!document.querySelector('[aria-label="Octal mode"]'),
    "Linux permissions dialog shows POSIX controls",
  );
  if (document.querySelector('[aria-label="Windows read-only"]'))
    throw Error("POSIX file shown with the Windows read-only mapping");
  setValue(
    document.querySelector<HTMLInputElement>('[aria-label="Octal mode"]')!,
    "640",
  );
  await new Promise((r) => setTimeout(r, 0));
  button("Apply permissions").click();
  await wait(
    () => !document.querySelector("dialog[open]"),
    "Octal mode 640 applies to the test file",
  );
  await send(`printf 'YT_MODE_%s\\n' $(stat -c %a "${path}/editor.txt")`);
  await wait(
    () => rows().includes("YT_MODE_640"),
    "Server stat reports the applied POSIX mode 640",
  );
  row("editor.txt")!.click();
  await wait(
    () => !!button("Permissions"),
    "Permissions can be reviewed again",
  );
  button("Permissions").click();
  await wait(
    () =>
      document.querySelector<HTMLInputElement>('[aria-label="Octal mode"]')
        ?.value === "640",
    "Native screenshot retains the applied POSIX mode",
  );
  document
    .querySelector<HTMLButtonElement>('[aria-label="Close dialog"]')!
    .click();
  // chmod -R: two directories and a file change, the symlink is skipped.
  await send(
    `mkdir "${path}/native-dir/sub" && : > "${path}/native-dir/sub/f.txt" && ln -s sub "${path}/native-dir/link" && printf 'YT_%s\\n' TREE_OK`,
  );
  await wait(
    () => rows().includes("YT_TREE_OK"),
    "Recursive test tree created",
  );
  const openPermissions = async (name: string) => {
    row(name)!.click();
    await wait(() => !!button("Permissions"), `${name} exposes permissions`);
    button("Permissions").click();
    await wait(
      () => !!document.querySelector('[aria-label="Apply recursively"]'),
      "Directory permissions offer a recursive option",
    );
  };
  await openPermissions("native-dir");
  document
    .querySelector<HTMLInputElement>('[aria-label="Apply recursively"]')!
    .click();
  setValue(
    document.querySelector<HTMLInputElement>('[aria-label="Octal mode"]')!,
    "750",
  );
  await wait(
    () =>
      document.querySelector('[aria-label="Symbolic mode"]')?.textContent ===
      "rwxr-x---",
    "Symbolic preview follows the octal mode",
  );
  button("Apply permissions").click();
  await wait(
    () =>
      document.body.textContent!.includes(
        "Permissions applied to 3 items · 1 symlinks/special files skipped",
      ),
    "Recursive permissions report 3 changed entries and the skipped symlink",
  );
  await send(
    `printf 'YT_R_%s\\n' "$(stat -c %a "${path}/native-dir" "${path}/native-dir/sub" "${path}/native-dir/sub/f.txt" | tr '\\n' _)"`,
  );
  await wait(
    () => rows().includes("YT_R_750_750_750_"),
    "Server stat reports mode 750 on the directory, subdirectory and file",
  );
  await openPermissions("native-dir");
  document
    .querySelector<HTMLInputElement>('[aria-label="Apply recursively"]')!
    .click();
  await wait(
    () =>
      document.querySelector<HTMLInputElement>('[aria-label="Octal mode"]')
        ?.value === "750",
    "Native screenshot shows the directory dialog with the recursive option",
  );
  await send(
    "clear; printf 'Linux OpenSSH %s\\n' qualification; printf '\\316\\251\\n'",
  );
  await wait(
    () =>
      rows().includes("Linux OpenSSH qualification") &&
      !rows().includes("yuuterm-test"),
    "Native capture terminal is clear of test account paths",
  );
  // Endpoint/account labels only are redacted for retained evidence, after all
  // protocol assertions. There are no fabricated connection or permission states.
  const profile = (
    await invoke<{ prefs: { profiles: { host: string; username: string }[] } }>(
      "bootstrap",
    )
  ).prefs.profiles[0];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const node = walker.currentNode;
    node.textContent = node
      .textContent!.replaceAll(profile.host, "<test-host>")
      .replaceAll(profile.username, "<test-account>");
  }
  input.value = "/home/<test-account>/yuuterm-test/<isolated-run>";
}
