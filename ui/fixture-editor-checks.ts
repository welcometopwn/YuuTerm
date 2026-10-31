import { invoke } from "@tauri-apps/api/core";
export async function checkEditor(ids: number[], checks: string[]) {
  const wait = async (
    predicate: () => boolean | Promise<boolean>,
    label: string,
  ) => {
    const end = Date.now() + 60000;
    while (Date.now() < end) {
      if (await predicate()) {
        checks.push(label);
        return;
      }
      await new Promise((r) => setTimeout(r, 100));
    }
    throw new Error(label);
  };
  const action = (label: string) =>
    Array.from(
      document.querySelectorAll<HTMLButtonElement>("dialog button"),
    ).find((b) => b.textContent?.trim() === label)!;
  const bridge = (operation: string, editId?: number) =>
    invoke<string>("capture_editor_bridge", { operation, editId });
  const state = async () => {
    try {
      return JSON.parse(await bridge("state")) as {
        phase?: string;
        checks?: string[];
        version?: string;
      };
    } catch {
      return {};
    }
  };
  const phase = async (expected: string) =>
    wait(async () => {
      const s = await state();
      if (s.phase === "failed") throw new Error(s.checks?.join("; "));
      return s.phase === expected;
    }, `Installed VS Code reached ${expected}`);
  await wait(
    () => document.querySelectorAll(".file-row").length >= 2,
    "SSH file browser is ready for installed-editor qualification",
  );
  document
    .querySelector<HTMLButtonElement>('[aria-label="Actions for read me.txt"]')!
    .parentElement!.querySelector("button")!
    .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  await wait(
    () =>
      document
        .querySelector(".files-panel")
        ?.getAttribute("data-watched-edits") === "1",
    "Generated remote file is staged by the normal file-open action",
  );
  const [edit] = await invoke<{ id: number }[]>("edit_list", { id: ids[0] });
  await bridge("request", edit.id);
  await phase("saved");
  await wait(
    () =>
      !!document.querySelector('dialog[aria-label="Upload saved changes?"]'),
    "An actual VS Code save triggers YuuTerm upload confirmation",
  );
  action("Yes, upload").click();
  await wait(
    () => !document.querySelector("dialog[open]"),
    "Approved VS Code save uploads successfully",
  );
  const probe = (operation: string, text = "") =>
    invoke<string>("capture_edit_probe", { editId: edit.id, operation, text });
  if ((await probe("read")) !== "VS Code saved revision one")
    throw new Error("Installed-editor save did not reach SFTP server");
  checks.push("Real SFTP content matches the installed VS Code saved document");
  await bridge("uploaded");
  await probe("remote-change", "server reload into the real VS Code editor");
  await wait(
    () => !!document.querySelector('dialog[aria-label="Server file changed"]'),
    "Independent remote change prompts while VS Code has the file open",
  );
  action("Reload server version").click();
  await phase("dirty");
  await probe("remote-change", "server second reload while editor dirty");
  await wait(
    () => !!document.querySelector('dialog[aria-label="Server file changed"]'),
    "Another remote version prompts without assuming the editor buffer is saved",
  );
  action("Reload server version").click();
  await phase("dirty-preserved");
  await bridge("finish");
  await phase("passed");
  const result = await state();
  checks.push(
    `Installed editor version: ${result.version}`,
    ...(result.checks ?? []),
  );
}
