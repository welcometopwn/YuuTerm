import { invoke } from "@tauri-apps/api/core";
// Settings → Commands against the local SSH fixture: a custom category moved to
// the top, custom commands in every mode (also inside built-in Docker), a
// hidden built-in category, then the SSH tools panel types exactly the saved
// text into this session only. The fixture server refuses exec, so the probe
// fails and every command stays enabled.
export async function checkCommandSettings(
  ids: number[],
  until: (f: () => boolean | Promise<boolean>, name: string) => Promise<void>,
) {
  const id = ids[0];
  const button = (label: string, root: ParentNode = document) =>
    Array.from(root.querySelectorAll<HTMLButtonElement>("button")).find(
      (b) => b.textContent?.trim() === label,
    );
  const section = (name: string) =>
    document.querySelector<HTMLElement>(
      `.command-category[data-category="${name}"]`,
    );
  const type = (el: HTMLInputElement | HTMLSelectElement, value: string) => {
    const prototype =
      el instanceof HTMLSelectElement
        ? HTMLSelectElement.prototype
        : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(el, value);
    el.dispatchEvent(
      new Event(el instanceof HTMLSelectElement ? "change" : "input", {
        bubbles: true,
      }),
    );
  };
  const frame = () =>
    new Promise<void>((r) => requestAnimationFrame(() => r()));
  const addCommand = async (
    category: string,
    label: string,
    command: string,
    mode: string,
  ) => {
    button("Add command", section(category)!)!.click();
    await frame();
    const row = Array.from(
      section(category)!.querySelectorAll<HTMLElement>(".command-row.custom"),
    ).at(-1)!;
    type(row.querySelector('[aria-label="Command name"]')!, label);
    await frame();
    type(row.querySelector(".command-text")!, command);
    await frame();
    type(row.querySelector("select")!, mode);
    await frame();
  };
  const trace = async () =>
    (await invoke<number[][]>("capture_trace", { id })).map((b) =>
      new TextDecoder().decode(new Uint8Array(b)),
    );

  document
    .querySelector<HTMLButtonElement>('button[aria-label="Settings"]')!
    .click();
  await until(
    () => !!document.getElementById("settings-tab-commands"),
    "Settings dialog offers a Commands section",
  );
  document.getElementById("settings-tab-commands")!.click();
  await until(() => {
    const dialog = document.querySelector("dialog")!;
    const tabs = document.querySelector(".settings-tabs")!;
    return (
      dialog.scrollWidth <= dialog.clientWidth &&
      tabs.scrollWidth <= tabs.clientWidth
    );
  }, "All six settings sections fit without horizontal scrolling");
  await until(
    () =>
      !!section("System") &&
      !section("System")!.querySelector('[aria-label="Category name"]') &&
      !section("System")!.querySelector('[aria-label^="Delete"]'),
    "Built-in categories are listed and cannot be renamed or deleted",
  );
  button("Add category")!.click();
  await until(() => !!section("My commands"), "Add category creates one");
  const name = section("My commands")!.querySelector<HTMLInputElement>(
    '[aria-label="Category name"]',
  )!;
  type(name, "Deploy");
  await until(() => !!section("Deploy"), "Custom categories can be renamed");
  await addCommand("Deploy", "Uptime", "  uptime -p  ", "run");
  await addCommand("Deploy", "List files", "ls -la ", "insert");
  await addCommand("Docker", "Compose up", "docker compose up -d", "confirm");
  // Custom commands can be ordered above the built-in ones.
  const compose = () =>
    section("Docker")!.querySelectorAll<HTMLElement>(".command-row")[0];
  for (let n = 0; n < 20 && !compose().classList.contains("custom"); n++) {
    section("Docker")!
      .querySelector<HTMLButtonElement>('[aria-label="Move Compose up up"]')!
      .click();
    await frame();
  }
  await until(
    () => compose().classList.contains("custom"),
    "A custom command moves above the built-in Docker commands",
  );
  // Pointer drag as the grip receives it under pointer capture. Synthetic
  // pointers cannot be captured, so only the capture calls are substituted.
  const dragTo = async (handle: Element, target: Element, below = false) => {
    target.scrollIntoView({ block: "center" });
    await frame();
    const box = target.getBoundingClientRect();
    const clientX = box.left + Math.min(40, box.width / 2);
    const clientY = below ? box.bottom - 3 : box.top + 3;
    const start = handle.getBoundingClientRect();
    const proto = Element.prototype;
    const { setPointerCapture, hasPointerCapture } = proto;
    proto.setPointerCapture = () => {};
    proto.hasPointerCapture = () => true;
    const fire = (type: string, x: number, y: number) =>
      handle.dispatchEvent(
        new PointerEvent(type, {
          bubbles: true,
          cancelable: true,
          pointerId: 7,
          button: 0,
          clientX: x,
          clientY: y,
        }),
      );
    try {
      fire("pointerdown", start.left + 4, start.top + 4);
      await frame();
      fire("pointermove", clientX, clientY);
      await frame();
      fire("pointermove", clientX, clientY);
      await frame();
      fire("pointerup", clientX, clientY);
      await frame();
    } finally {
      proto.setPointerCapture = setPointerCapture;
      proto.hasPointerCapture = hasPointerCapture;
    }
  };
  const rowNames = (category: string) =>
    Array.from(
      section(category)!.querySelectorAll(".command-row .command-name"),
    ).map((n) => n.textContent);
  await dragTo(
    section("System")!.querySelectorAll(".command-grip")[2],
    section("System")!.querySelectorAll(".command-row")[0],
  );
  await until(
    () =>
      rowNames("System")[0] === "Memory" &&
      rowNames("System")[1] === "Processes (htop)",
    "Dragging a built-in command reorders it within its category",
  );
  const deployRows = section("Deploy")!.querySelectorAll(".command-row").length;
  await dragTo(
    section("Docker")!.querySelector('[aria-label="Drag Containers"]')!,
    section("Deploy")!.querySelector(".command-row")!,
  );
  await until(
    () =>
      section("Deploy")!.querySelectorAll(".command-row").length ===
        deployRows && rowNames("Docker").includes("Containers"),
    "A built-in command cannot be dragged into another category",
  );
  await dragTo(
    section("Deploy")!.querySelector('[aria-label="Drag Deploy"]')!,
    document.querySelector(".command-category")!,
  );
  await until(
    () =>
      document
        .querySelector(".command-category")
        ?.getAttribute("data-category") === "Deploy",
    "The custom category moves above the built-in ones",
  );
  section("Power")!
    .querySelector<HTMLInputElement>('input[type="checkbox"]')!
    .click();
  await until(
    () => section("Power")!.classList.contains("hidden-category"),
    "Built-in categories can be hidden",
  );
  // A second line in a command would run unconfirmed text.
  const uptime =
    section("Deploy")!.querySelector<HTMLInputElement>(".command-text")!;
  type(uptime, "uptime\u0007");
  await until(
    () => !!button("Save settings")?.disabled,
    "A command with a control character blocks saving",
  );
  type(uptime, "  uptime -p  ");
  await until(
    () => button("Save settings")?.disabled === false,
    "A valid layout can be saved",
  );
  button("Save settings")!.click();
  await until(
    () => !document.getElementById("settings-tab-commands"),
    "Settings save and close",
  );

  document
    .querySelector<HTMLButtonElement>('button[aria-label="Remote tools"]')!
    .click();
  const command = (key: string) =>
    document.querySelector<HTMLButtonElement>(`[data-command="${key}"]`);
  const categories = () =>
    Array.from(document.querySelectorAll(".ssh-category")).map((c) =>
      c.textContent?.trim(),
    );
  await until(
    () =>
      categories()[0] === "Deploy" &&
      !categories().includes("Power") &&
      !!command("custom-0") &&
      !command("custom-0")!.disabled,
    "The panel shows the custom category first and hides Power",
  );
  await until(() => {
    const docker = Array.from(
      document.querySelectorAll(".ssh-command-list section"),
    ).find(
      (s) => s.querySelector(".ssh-category")?.textContent?.trim() === "Docker",
    );
    return (
      docker?.querySelector(".ssh-command")?.getAttribute("data-command") ===
      "custom-2"
    );
  }, "The custom Docker command sits before the built-in ones");

  let before = (await trace()).length;
  command("custom-0")!.click();
  await until(
    async () => (await trace()).length > before,
    "Run types the command",
  );
  let typed = (await trace()).slice(before);
  if (typed.join("") !== "uptime -p\r")
    throw Error(`Unexpected run input: ${JSON.stringify(typed)}`);
  await until(() => true, "Run sends the trimmed text followed by Enter");

  before = (await trace()).length;
  command("custom-1")!.click();
  await until(
    async () => (await trace()).length > before,
    "Insert types the command",
  );
  typed = (await trace()).slice(before);
  if (typed.join("") !== "ls -la")
    throw Error(`Unexpected insert input: ${JSON.stringify(typed)}`);
  await until(() => true, "Insert types the text without Enter");
  // Clear the inserted text so later input starts on an empty line.
  await invoke("session_input", { id, bytes: [0x15] });

  before = (await trace()).length;
  command("custom-2")!.click();
  await until(
    () =>
      document.querySelector(".ssh-preview")?.textContent ===
      "docker compose up -d",
    "Confirm mode shows the exact command first",
  );
  button("Cancel", document.querySelector(".rdp-tools-confirm")!)!.click();
  await new Promise((r) => setTimeout(r, 300));
  if ((await trace()).length !== before)
    throw Error("Cancelled custom command typed input");
  await until(() => true, "Cancelling a confirmed command types nothing");

  document
    .querySelector<HTMLButtonElement>('button[aria-label="Settings"]')!
    .click();
  await until(
    () => !!document.getElementById("settings-tab-commands"),
    "Settings reopen",
  );
  document.getElementById("settings-tab-commands")!.click();
  await until(
    () =>
      document
        .querySelector(".command-category")
        ?.getAttribute("data-category") === "Deploy" &&
      section("Deploy")!.querySelectorAll(".command-row.custom").length === 2,
    "The saved layout is shown again in Settings",
  );
}
