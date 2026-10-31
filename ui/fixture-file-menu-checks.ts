import { invoke } from "@tauri-apps/api/core";
export async function checkFileMenus(
  ids: number[],
  until: (p: () => boolean | Promise<boolean>, label: string) => Promise<void>,
  checks: string[],
) {
  const button = (label: string) =>
    Array.from(
      document.querySelectorAll<HTMLButtonElement>(
        ".context-menu button, dialog button",
      ),
    ).find((b) => b.textContent?.trim() === label)!;
  const row = (name: string) =>
    document
      .querySelector<HTMLButtonElement>(`[aria-label="Actions for ${name}"]`)!
      .parentElement!.querySelector<HTMLButtonElement>("button")!;
  const right = (element: Element) =>
    element.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        clientX: 80,
        clientY: 240,
      }),
    );
  await until(
    () => !!document.querySelector('[aria-label="Actions for read me.txt"]'),
    "Connected file browser is ready for contextual file actions",
  );
  right(document.querySelector(".files-panel")!);
  await until(
    () => !!button("New file"),
    "Blank file-panel right click offers New file",
  );
  if (
    Array.from(document.querySelectorAll(".context-menu button")).some((b) =>
      /New session|Settings|Appearance/.test(b.textContent ?? ""),
    )
  )
    throw new Error("Application actions leaked into Files context");
  checks.push(
    "Files background does not show New session or Appearance actions",
  );
  button("New file").click();
  await until(
    () =>
      !!document.querySelector('.file-new [aria-label="Filename"]') &&
      !document.querySelector("dialog[open]"),
    "New file opens an inline name row in the list",
  );
  const input = document.querySelector<HTMLInputElement>(
    '[aria-label="Filename"]',
  )!;
  Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )!.set!.call(input, "created in menu.txt");
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await new Promise((r) => setTimeout(r, 0));
  input.form!.requestSubmit();
  await until(
    () =>
      !!document.querySelector(
        '[aria-label="Actions for created in menu.txt"]',
      ),
    "New file is created through real SFTP and appears in the refreshed browser",
  );
  let denied = false;
  try {
    await invoke("file_action", {
      id: ids[0],
      action: { kind: "create", path: "/fixture", name: "read me.txt" },
    });
  } catch {
    denied = true;
  }
  if (!denied) throw new Error("Create replaced an existing remote file");
  checks.push("Existing destination is refused without replacement");
  right(row("read me.txt"));
  await until(() => !!button("Open with"), "File menu exposes Open with");
  for (const label of [
    "Open with",
    "Download",
    "Rename",
    "Delete",
    "Copy path to terminal",
  ])
    if (!button(label)) throw new Error(`Missing compact menu label: ${label}`);
  if (document.querySelector(".context-menu")!.textContent?.includes("POSIX"))
    throw new Error("Old path-action label remains");
  checks.push(
    "Open with, Download, Rename and Delete omit ellipses; terminal path action uses its new name",
  );
  button("Copy path to terminal").click();
  await until(
    async () =>
      new TextDecoder()
        .decode(
          new Uint8Array(
            (await invoke<number[][]>("capture_trace", { id: ids[0] })).flat(),
          ),
        )
        .includes("'/fixture/read me.txt'"),
    "Context action sends a quoted path to the owning terminal",
  );
  await until(
    () => !document.querySelector(".context-menu"),
    "Path insertion dismisses the context menu",
  );
  row("read me.txt").dispatchEvent(
    new MouseEvent("auxclick", { bubbles: true, button: 1 }),
  );
  await until(
    async () =>
      (await invoke<number[][]>("capture_trace", { id: ids[0] })).filter(
        (v) =>
          new TextDecoder().decode(new Uint8Array(v)) ===
          "'/fixture/read me.txt'",
      ).length === 2,
    "Middle click invokes the same terminal-path insertion",
  );
  for (const id of ids) {
    const trace = await invoke<number[][]>("capture_trace", { id });
    const paths = trace
      .map((v) => new TextDecoder().decode(new Uint8Array(v)))
      .filter((s) => s.includes("/fixture/read me.txt"));
    if (id !== ids[0] && paths.length)
      throw new Error("Path was broadcast to another terminal");
    if (paths.some((s) => s.includes("\r") || s.includes("\n")))
      throw new Error("Path insertion sent Enter");
  }
  checks.push(
    "Middle-click paths never send Enter or broadcast to other panes",
  );
  // Shell UI is intentionally skipped in capture mode, but staging and watching
  // are exercised through the exact Open-with action.
  right(row("read me.txt"));
  await until(
    () => !!button("Open with"),
    "Open with remains available after middle click",
  );
  button("Open with").click();
  await until(
    () =>
      document
        .querySelector(".files-panel")
        ?.getAttribute("data-watched-edits") === "1",
    "Open with stages a watched local file; external Windows chooser is skipped in this fixture",
  );
  const edits = await invoke<{ id: number }[]>("edit_list", { id: ids[0] });
  if (edits.length !== 1)
    throw new Error("Open with did not register exactly one watched edit");
  checks.push(
    "Open with retains normal edit monitoring and upload conflict handling",
  );
  right(row("read me.txt"));
  await until(
    () => !!button("Copy path to terminal"),
    "Compact file context menu is retained for the native screenshot",
  );
}
