import { quotedPath } from "./input-policy";
import { invoke } from "@tauri-apps/api/core";
// Synthetic native DOM and disk saves against the explicitly selected real VM.
export async function checkWindowsOpenSsh(ids: number[], checks: string[]) {
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
  const send = (command: string) =>
    invoke("session_input", {
      id,
      bytes: Array.from(new TextEncoder().encode(command + "\r")),
    });
  await send(
    "if(Get-Module PSReadLine){Set-PSReadLineOption -HistorySaveStyle SaveNothing}; function prompt { 'YuuTerm test> ' }; Clear-Host; Write-Output ('native-'+'PowerShell'); Write-Output ([char]0x03a9)",
  );
  await wait(
    () =>
      document
        .querySelector(".xterm-rows")
        ?.textContent?.includes("native-PowerShell") === true &&
      document.querySelector(".xterm-rows")?.textContent?.includes("Ω") ===
        true,
    "Actual native terminal renders PowerShell and Unicode output from Windows OpenSSH",
  );
  const input = document.querySelector<HTMLInputElement>(
    '[aria-label="Remote path"]',
  )!;
  Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )!.set!.call(input, path.slice(1).replaceAll("/", "\\"));
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await new Promise((r) => setTimeout(r, 0));
  input.form!.requestSubmit();
  await wait(
    () => input.value === path && !!row("editor.txt"),
    "File browser canonicalizes a Windows drive/backslash path into the real SFTP directory",
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
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(nameInput, name);
    nameInput.dispatchEvent(new Event("input", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 0));
    nameInput.form!.requestSubmit();
    await wait(() => !!row(name), `${label} creates a real Windows SFTP entry`);
  }
  const rect = document.querySelector(".files-panel")!.getBoundingClientRect();
  const dropped = await invoke<string>("capture_simulate_drop", {
    x: (rect.left + rect.width / 2) * devicePixelRatio,
    y: (rect.top + rect.height / 2) * devicePixelRatio,
  });
  await wait(
    () => !!row(dropped),
    "Native drop token/event path uploads into the actual Windows SFTP directory",
  );
  const download = await invoke<{ completed: number; problem: string | null }>(
    "files_download",
    { id, paths: [path + "/tree"] },
  );
  if (download.completed !== 1 || download.problem)
    throw Error("Windows directory download failed");
  checks.push(
    "Normal directory-download backend completes against Windows OpenSSH; runner independently checks exact bytes",
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
    "Rename preserves spaces and Unicode on Windows",
  );
  await send(
    "if(Test-Path -LiteralPath " +
      quotedPath(path + "/native renamed ' Ω.txt") +
      "){Write-Output ('YT_'+'PATH_OK')}",
  );
  await wait(
    () =>
      document
        .querySelector(".xterm-rows")
        ?.textContent?.includes("YT_PATH_OK") === true,
    "PowerShell literal path quoting resolves a real Windows filename with spaces apostrophe and Unicode",
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
    "Confirmed Windows file deletion refreshes the browser",
  );
  row("editor.txt")!
    .parentElement!.querySelector("button")!
    .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  await wait(
    () =>
      document
        .querySelector(".files-panel")
        ?.getAttribute("data-watched-edits") === "1",
    "Real Windows remote file stages for external editing without launching an installed editor",
  );
  const [edit] = await invoke<{ id: number }[]>("edit_list", { id });
  const probe = (operation: string, text = "") =>
    invoke<string>("capture_edit_probe", { editId: edit.id, operation, text });
  await probe("save", "native explicit save\r\nΩ\r\n");
  await wait(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "Local disk save prompts before uploading to Windows",
  );
  button("Yes, upload").click();
  await wait(
    () => !document.querySelector("dialog[open]"),
    "Explicitly approved edit upload completes",
  );
  if ((await probe("read")) !== "native explicit save\r\nΩ\r\n")
    throw Error("Windows edit upload changed Unicode/CRLF bytes");
  checks.push(
    "Approved editor upload preserves Unicode and CRLF on the real server",
  );
  await probe("save", "native automatic save\r\n");
  await wait(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "Automatic upload remains explicit opt-in",
  );
  button("Auto-upload all").click();
  await wait(
    async () =>
      !document.querySelector("dialog[open]") &&
      (await probe("read")) === "native automatic save\r\n",
    "Opted-in automatic editor upload reaches Windows OpenSSH",
  );
  await probe("remote-change", "independent Windows server content\r\n");
  await probe("save", "conflicting local Windows edit\r\n");
  await wait(
    () =>
      document
        .querySelector('dialog[aria-label="Upload saved changes?"] .file-error')
        ?.textContent?.includes("Conflict") === true,
    "A real server-side change blocks conflicting automatic upload",
  );
  if (
    (await probe("read")) !== "independent Windows server content\r\n" ||
    (await probe("local-read")) !== "conflicting local Windows edit\r\n"
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
  await wait(() => !!button("Permissions"), "Windows file exposes permissions");
  button("Permissions").click();
  await wait(
    () => !!document.querySelector('[aria-label="Windows read-only"]'),
    "Windows permissions dialog shows the read-only mapping instead of POSIX ACL controls",
  );
  if (
    document.querySelector('[aria-label="Octal mode"]') ||
    !document.querySelector("dialog")?.textContent?.includes("not NTFS ACLs")
  )
    throw Error("Windows mode misrepresents ACL semantics");
  document
    .querySelector<HTMLInputElement>('[aria-label="Windows read-only"]')!
    .click();
  button("Apply permissions").click();
  await wait(
    () => !document.querySelector("dialog[open]"),
    "Read-only mapping applies to the test file",
  );
  const props = await invoke<{
    properties: { windows_permissions: boolean; permissions: number };
  }>("file_action", {
    id,
    action: { kind: "properties", path: path + "/editor.txt" },
  });
  if (
    !props.properties.windows_permissions ||
    (props.properties.permissions & 0o200) !== 0
  )
    throw Error("Server read-only mapping did not change");
  await invoke("file_action", {
    id,
    action: { kind: "permissions", path: path + "/editor.txt", mode: 0o600 },
  });
  checks.push(
    "Mapped read-only property is observed on the server and restored to writable",
  );
  row("editor.txt")!.click();
  await wait(
    () => !!button("Permissions"),
    "Permissions can be reviewed again",
  );
  button("Permissions").click();
  await wait(
    () => !!document.querySelector('[aria-label="Windows read-only"]'),
    "Native screenshot retains the Windows mapping explanation",
  );
  await send(
    "Clear-Host; Write-Output ('Windows OpenSSH '+'qualification'); Write-Output ([char]0x03a9)",
  );
  await wait(
    () =>
      document
        .querySelector(".xterm-rows")
        ?.textContent?.includes("Windows OpenSSH qualification") === true &&
      !document
        .querySelector(".xterm-rows")
        ?.textContent?.includes("yuuterm-test"),
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
  input.value = "/C:/Users/<test-account>/yuuterm-test/<isolated-run>";
}
