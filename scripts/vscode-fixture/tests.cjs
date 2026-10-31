// Runs in the installed VS Code extension host against generated test files only.
const vscode = require("vscode");
const fs = require("node:fs/promises");
const path = require("node:path");
const assert = require("node:assert/strict");
exports.run = async () => {
  const root = process.env.YUUTERM_EDITOR_FIXTURE;
  if (!root || !path.isAbsolute(root))
    throw new Error("Isolated fixture directory required");
  const checks = [];
  const json = async (name) => {
    try {
      return JSON.parse(await fs.readFile(path.join(root, name), "utf8"));
    } catch {
      return {};
    }
  };
  const write = async (phase) =>
    fs.writeFile(
      path.join(root, "editor-state.json"),
      JSON.stringify({ phase, version: vscode.version, checks }),
    );
  const wait = async (test, label) => {
    const end = Date.now() + 60000;
    while (Date.now() < end) {
      if (await test()) {
        checks.push(label);
        return;
      }
      await new Promise((r) => setTimeout(r, 100));
    }
    throw new Error(label);
  };
  try {
    await wait(
      async () => !!(await json("editor-request.json")).local,
      "YuuTerm supplied an isolated staged file",
    );
    const { local } = await json("editor-request.json");
    const relative = path.relative(path.join(root, "staging"), local);
    assert(
      relative && !relative.startsWith("..") && !path.isAbsolute(relative),
    );
    const doc = await vscode.workspace.openTextDocument(vscode.Uri.file(local));
    await vscode.window.showTextDocument(doc);
    const replace = async (text) => {
      const edit = new vscode.WorkspaceEdit();
      edit.replace(
        doc.uri,
        new vscode.Range(
          doc.positionAt(0),
          doc.positionAt(doc.getText().length),
        ),
        text,
      );
      assert(await vscode.workspace.applyEdit(edit));
    };
    await replace("VS Code saved revision one");
    assert(await doc.save());
    assert.equal(
      await fs.readFile(local, "utf8"),
      "VS Code saved revision one",
    );
    checks.push(
      "Installed VS Code saved the staged document through TextDocument.save",
    );
    await write("saved");
    await wait(
      async () => (await json("editor-control.json")).command === "uploaded",
      "YuuTerm confirmed upload of the VS Code save",
    );
    await wait(
      () =>
        doc.getText() === "server reload into the real VS Code editor" &&
        !doc.isDirty,
      "VS Code automatically refreshed its clean open document after YuuTerm reload",
    );
    await replace("VS Code unsaved local buffer");
    assert(doc.isDirty);
    await write("dirty");
    await wait(
      async () =>
        (await fs.readFile(local, "utf8")) ===
        "server second reload while editor dirty",
      "YuuTerm replaced disk content after explicit second reload",
    );
    assert(doc.isDirty);
    assert.equal(doc.getText(), "VS Code unsaved local buffer");
    checks.push(
      "VS Code retained unsaved buffer contents when the disk version changed",
    );
    await write("dirty-preserved");
    await wait(
      async () => (await json("editor-control.json")).command === "finish",
      "Native checks completed without requesting unsaved-buffer overwrite",
    );
    await write("passed");
  } catch (e) {
    checks.push("FAILED: " + String(e));
    await write("failed");
    throw e;
  }
};
