import { invoke } from "@tauri-apps/api/core";
import { normalizeIcon } from "./icon-normalization";
import type { Bootstrap } from "./types";

export async function checkQol(
  _ids: number[],
  until: (p: () => boolean | Promise<boolean>, label: string) => Promise<void>,
  checks: string[],
) {
  document.querySelector<HTMLButtonElement>('[aria-label="Settings"]')!.click();
  await until(
    () => !!document.querySelector("#settings-panel-appearance select"),
    "Appearance opens",
  );
  const theme = document.querySelector<HTMLSelectElement>(
    "#settings-panel-appearance select",
  )!;
  theme.value = "catppuccin-mocha";
  theme.dispatchEvent(new Event("change", { bubbles: true }));
  Array.from(document.querySelectorAll<HTMLButtonElement>("dialog button"))
    .find((b) => b.textContent?.trim() === "Save settings")!
    .click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Mocha is saved before reconnect",
  );
  const pane = () =>
    document.querySelector<HTMLElement>('.pane[data-profile-id="1"]')!;
  const background = () => {
    const viewport =
      pane().querySelector<HTMLElement>(".xterm-screen")?.parentElement;
    return viewport && getComputedStyle(viewport).backgroundColor;
  };
  await until(() => !!background(), "Initial SSH terminal renderer is ready");
  await until(
    () => background() === "rgb(30, 30, 46)",
    "Initial renderer uses Mocha background",
  );
  const expected = background();
  const previous = Number(pane().dataset.liveId);
  await invoke("session_disconnect", { id: previous });
  await until(
    () => !!pane().querySelector("button.connect"),
    "Disconnect exposes reconnect",
  );
  pane().querySelector<HTMLButtonElement>("button.connect")!.click();
  await until(
    () => Number(pane().dataset.liveId) !== previous && !!background(),
    "Reconnect creates a new terminal renderer",
  );
  await new Promise((resolve) => setTimeout(resolve, 300));
  await until(
    () => background() === expected,
    "Reconnect preserves the exact theme background",
  );
  checks.push("Real loopback SSH reconnect retains terminal theme");
  const button = (text: string, scope = "dialog") =>
    Array.from(
      document.querySelectorAll<HTMLButtonElement>(`${scope} button`),
    ).find((b) => b.textContent?.trim() === text)!;
  const click = (label: string) =>
    document
      .querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!
      .click();
  const pick = (name: string, keys: MouseEventInit = {}) =>
    document
      .querySelector<HTMLButtonElement>(
        `.file-row[data-name="${CSS.escape(name)}"] > button`,
      )!
      .dispatchEvent(
        new MouseEvent("click", { bubbles: true, cancelable: true, ...keys }),
      );
  const selectedNames = () =>
    Array.from(
      document.querySelectorAll<HTMLElement>(".file-row.file-selected"),
    )
      .map((row) => row.dataset.name)
      .sort()
      .join(",");
  const inputValue = (el: HTMLInputElement, value: string) => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  };
  click("Settings");
  await until(
    () => !!document.getElementById("settings-tab-highlighting"),
    "Highlighting settings opens",
  );
  document.getElementById("settings-tab-highlighting")!.click();
  button("Add color rule").click();
  await until(
    () =>
      document.querySelectorAll(".highlight-rule:not(.ip-highlight-rule)")
        .length === 4,
    "Custom highlighting draft exists",
  );
  button("Reset highlighting to defaults").click();
  await until(
    () => !!button("Keep my rules"),
    "Reset asks for explicit confirmation",
  );
  button("Keep my rules").click();
  if (
    document.querySelectorAll(".highlight-rule:not(.ip-highlight-rule)")
      .length !== 4
  )
    throw new Error("Declining reset changed the draft");
  button("Reset highlighting to defaults").click();
  button("Reset highlighting").click();
  await until(
    () =>
      document.querySelectorAll(".highlight-rule:not(.ip-highlight-rule)")
        .length === 3,
    "Confirmed reset restores default rules in draft",
  );
  button("Cancel").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Cancel closes highlighting without saving",
  );
  checks.push(
    "Highlighting reset requires confirmation and respects outer Cancel",
  );

  document.querySelector<HTMLButtonElement>(".side-tabs button")!.click();
  await until(
    () => !!document.querySelector(".sessions-footer"),
    "Saved-session count appears in the sidebar footer",
  );
  if (document.querySelector(".section-label > span"))
    throw new Error("Old header counter remains");
  click("Edit SSH fixture 1");
  await until(
    () => !!document.getElementById("session-tab-terminal"),
    "Session editor opens",
  );
  document.getElementById("session-tab-terminal")!.click();
  const font = document.querySelector<HTMLInputElement>(
    '[aria-label="Session font size"]',
  )!;
  inputValue(font, "7");
  await until(
    () => button("Save session").disabled,
    "Font below 8 cannot be saved",
  );
  inputValue(font, "33");
  await until(
    () => button("Save session").disabled,
    "Font above 32 cannot be saved",
  );
  inputValue(font, "17.5");
  await until(
    () => !button("Save session").disabled,
    "Typed valid font size can be saved",
  );
  button("Save session").click();
  await until(
    async () =>
      !document.querySelector("dialog[open]") &&
      (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0].font_size ===
        17.5,
    "Typed font override persists",
  );

  const svg =
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><path fill="#89b4fa" d="M2 2H30V30H2Z"/></svg>';
  const normalized = await normalizeIcon(
    new File([svg], "fixture.svg", { type: "image/svg+xml" }),
  );
  if (!normalized.startsWith("data:image/png;base64,"))
    throw new Error("SVG was not rasterized");
  const png = Uint8Array.from(atob(normalized.split(",")[1]), (c) =>
    c.charCodeAt(0),
  );
  const ico = new Uint8Array(22 + png.length),
    header = new DataView(ico.buffer);
  header.setUint16(2, 1, true);
  header.setUint16(4, 1, true);
  ico[6] = 32;
  ico[7] = 32;
  header.setUint16(10, 1, true);
  header.setUint16(12, 32, true);
  header.setUint32(14, png.length, true);
  header.setUint32(18, 22, true);
  ico.set(png, 22);
  if (
    !(await normalizeIcon(new File([ico], "fixture.ico"))).startsWith(
      "data:image/png;base64,",
    )
  )
    throw new Error("ICO was not normalized");
  checks.push(
    "Actual WebView rasterizes basic SVG and PNG-framed ICO to portable PNG",
  );
  const dib = new Uint8Array(22 + 40 + 32 * 32 * 4 + 32 * 4),
    dv = new DataView(dib.buffer);
  dv.setUint16(2, 1, true);
  dv.setUint16(4, 1, true);
  dib[6] = 32;
  dib[7] = 32;
  dv.setUint16(10, 1, true);
  dv.setUint16(12, 32, true);
  dv.setUint32(14, dib.length - 22, true);
  dv.setUint32(18, 22, true);
  dv.setUint32(22, 40, true);
  dv.setInt32(26, 32, true);
  dv.setInt32(30, 64, true);
  dv.setUint16(34, 1, true);
  dv.setUint16(36, 32, true);
  for (let i = 62; i < 62 + 32 * 32 * 4; i += 4) {
    dib[i] = 250;
    dib[i + 1] = 180;
    dib[i + 2] = 137;
    dib[i + 3] = 255;
  }
  await normalizeIcon(new File([dib], "bitmap.ico"));
  checks.push(
    "Uncompressed bitmap-framed ICO is also decoded in the real WebView",
  );
  for (const content of [
    '<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>',
    '<svg xmlns="http://www.w3.org/2000/svg"><image href="https://invalid.example/icon.png"/></svg>',
    '<svg xmlns="http://www.w3.org/2000/svg"><path fill="url(https://invalid.example/x)"/></svg>',
    '<svg xmlns="http://www.w3.org/2000/svg" style="background:red"/>',
    '<svg xmlns="http://www.w3.org/2000/svg" width="100000"/>',
    '<!DOCTYPE svg><svg xmlns="http://www.w3.org/2000/svg"/>',
    '<svg xmlns="http://www.w3.org/2000/svg"><?xml-stylesheet href="https://invalid.example/style.css"?></svg>',
  ]) {
    let refused = false;
    try {
      await normalizeIcon(new File([content], "unsafe.svg"));
    } catch {
      refused = true;
    }
    if (!refused) throw new Error("Unsafe SVG accepted");
  }
  checks.push(
    "Scripts, external references, styles, declarations and oversized SVG are refused before rendering",
  );
  ico[6] = 64;
  let refused = false;
  try {
    await normalizeIcon(new File([ico], "bad.ico"));
  } catch {
    refused = true;
  }
  if (!refused) throw new Error("ICO frame dimension mismatch accepted");
  checks.push("ICO directory/frame mismatch is rejected before decoding");
  const pngOfSize = async (size: number) => {
    const c = document.createElement("canvas");
    c.width = size;
    c.height = size;
    const g = c.getContext("2d")!;
    for (let y = 0; y < size; y += 8)
      for (let x = 0; x < size; x += 8) {
        g.fillStyle = (x + y) % 16 ? "#f38ba8" : "#89b4fa";
        g.fillRect(x, y, 8, 8);
      }
    return new Promise<Blob>((r) => c.toBlob((b) => r(b!), "image/png"));
  };
  const large = await normalizeIcon(
    new File([await pngOfSize(512)], "large.png", { type: "image/png" }),
  );
  const largeImage = new Image();
  largeImage.src = large;
  await largeImage.decode();
  if (
    largeImage.naturalWidth !== 32 ||
    largeImage.naturalHeight !== 32 ||
    large.length > 11000
  )
    throw new Error("512×512 PNG was not compressed to a 32×32 icon");
  checks.push("512×512 PNG is accepted and compressed to a 32×32 icon");
  for (const file of [
    new File([await pngOfSize(513)], "too-wide.png", { type: "image/png" }),
    new File([new Uint8Array(4 * 1024 * 1024 + 1)], "too-big.png", {
      type: "image/png",
    }),
  ]) {
    let rejected = false;
    try {
      await normalizeIcon(file);
    } catch {
      rejected = true;
    }
    if (!rejected) throw new Error(`${file.name} was accepted`);
  }
  checks.push("Icons above 512×512 pixels or 4 MB are refused");
  const viewport = pane().querySelector<HTMLElement>(".xterm-viewport");
  if (
    !viewport ||
    getComputedStyle(viewport).backgroundColor !== "rgba(0, 0, 0, 0)" ||
    getComputedStyle(viewport.closest(".terminal-host")!).backgroundColor !==
      expected
  )
    throw new Error("Terminal viewport can still show a black strip");
  checks.push(
    "Terminal viewport is transparent over the theme-colored host (no black strip)",
  );

  click("Show remote files");
  const path = () =>
    document.querySelector<HTMLInputElement>('[aria-label="Remote path"]')!;
  const liveId = () => Number(pane().dataset.liveId);
  await until(
    () => path()?.value === "/fixture",
    "File browser points to the active reconnected SSH session",
  );
  const make = async (directory: boolean, name: string) => {
    click(directory ? "New directory" : "New file");
    await until(
      () =>
        !!document.querySelector('.file-new [aria-label="Filename"]') &&
        !document.querySelector("dialog[open]"),
      "Toolbar opens an inline name row",
    );
    const field = document.querySelector<HTMLInputElement>(
      '.file-new [aria-label="Filename"]',
    )!;
    inputValue(field, name);
    await new Promise((r) => setTimeout(r, 0));
    field.form!.requestSubmit();
    await until(
      () =>
        !document.querySelector("dialog[open]") &&
        !!document.querySelector(`[aria-label="Actions for ${name}"]`),
      `${name} is created through real SFTP`,
    );
  };
  click("New file");
  await until(
    () => !!document.querySelector(".file-new"),
    "Inline row opens for cancellation",
  );
  document
    .querySelector('.file-new [aria-label="Filename"]')!
    .dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
  await until(
    () => !document.querySelector(".file-new"),
    "Escape cancels the inline row without creating anything",
  );
  await make(true, "qol-folder");
  await make(false, "qol-a.txt");
  await make(false, "qol-b.txt");
  await invoke("session_browse", { id: liveId(), path: "/fixture/logs" });
  await until(
    () => path().value === "/fixture/logs",
    "Entered child directory",
  );
  document.querySelector(".files-panel")!.dispatchEvent(
    new MouseEvent("mousedown", {
      button: 3,
      bubbles: true,
      cancelable: true,
    }),
  );
  await until(
    () => path().value === "/fixture",
    "Mouse4 navigates to parent through real SFTP",
  );
  await invoke("session_browse", { id: liveId(), path: "/fixture/logs" });
  await until(
    () => path().value === "/fixture/logs",
    "Child directory entered again",
  );
  document
    .querySelector('[aria-label="Go to parent directory"]')!
    .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  await until(
    () => path().value === "/fixture",
    "Parent row navigates up through real SFTP",
  );
  pick("read me.txt");
  pick("qol-a.txt", { ctrlKey: true });
  await until(
    () =>
      selectedNames() === "qol-a.txt,read me.txt" &&
      !document.querySelector<HTMLButtonElement>(
        '[aria-label="Download selected files"]',
      )!.disabled,
    "Click plus Ctrl+click enables toolbar bulk actions",
  );
  click("Download selected files");
  await until(
    () =>
      document
        .querySelector(".app-notice")
        ?.textContent?.includes("Downloaded 2 items") ?? false,
    "Two files download in one batch",
  );
  pick("qol-a.txt");
  pick("qol-b.txt", { shiftKey: true });
  await until(
    () => selectedNames() === "qol-a.txt,qol-b.txt",
    "Shift+click selects the range from the anchor",
  );
  click("Delete selected items");
  await until(
    () => !!button("Delete selected permanently"),
    "Bulk delete lists items and requires confirmation",
  );
  button("Cancel").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Cancelling bulk deletion retains selected files",
  );
  if (!document.querySelector('[aria-label="Actions for qol-a.txt"]'))
    throw new Error("Cancel deleted a file");
  click("Delete selected items");
  await until(
    () => !!button("Delete selected permanently"),
    "Bulk delete confirmation can be reopened",
  );
  button("Delete selected permanently").click();
  await until(
    () =>
      !document.querySelector("dialog[open]") &&
      !document.querySelector('[aria-label="Actions for qol-a.txt"]') &&
      !document.querySelector('[aria-label="Actions for qol-b.txt"]'),
    "Confirmed bulk delete removes only the selected fixture files",
  );
  pick("logs");
  pick("qol-folder", { ctrlKey: true });
  await until(
    () => selectedNames() === "logs,qol-folder",
    "Ctrl+click selects two directories",
  );
  click("Delete selected items");
  await until(
    () => !!button("Delete selected permanently"),
    "Directory batch also requires confirmation",
  );
  button("Delete selected permanently").click();
  await until(
    () =>
      document
        .querySelector('dialog [role="alert"]')
        ?.textContent?.includes("stopped") ?? false,
    "Nonempty-directory failure stops the deletion batch",
  );
  if (!document.querySelector('[aria-label="Actions for qol-folder"]'))
    throw new Error("Deletion continued after a failed item");
  button("Cancel").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Failed batch can be dismissed with remaining files retained",
  );
  document
    .querySelector(".file-row button")!
    .dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
  await until(() => selectedNames() === "", "Escape clears the selection");
  for (const paths of [
    ["relative"],
    ["/fixture/A", "/fixture/a"],
    Array(101).fill("/fixture/read me.txt"),
  ]) {
    let refused = false;
    try {
      await invoke("files_download", { id: liveId(), paths });
    } catch {
      refused = true;
    }
    if (!refused) throw new Error("Invalid bulk download accepted");
  }
  checks.push(
    "Bulk download refuses relative paths, Windows name collisions and oversized batches",
  );
  const partial = await invoke<{ completed: number; problem: string | null }>(
    "files_download",
    { id: liveId(), paths: ["/fixture/read me.txt", "/fixture/missing"] },
  );
  if (partial.completed !== 1 || !partial.problem)
    throw new Error(
      "Partial-download report did not retain the completed file",
    );
  checks.push(
    "Batch download reports partial completion and stops on a missing file",
  );
  type Batch = { completed: number; problem: string | null; skipped: number };
  const subtree = await invoke<Batch>("files_download", {
    id: liveId(),
    paths: ["/fixture/logs"],
  });
  if (subtree.completed !== 1 || subtree.problem)
    throw new Error(`Directory download failed: ${subtree.problem}`);
  checks.push("A directory downloads recursively in one batch");
  const tree = await invoke<Batch>("files_download", {
    id: liveId(),
    paths: ["/fixture"],
  });
  if (tree.completed !== 1 || tree.problem || tree.skipped !== 4)
    throw new Error(
      `Tree download did not skip exactly the four links: ${JSON.stringify(tree)}`,
    );
  checks.push(
    "Whole-tree download skips symlinks, including a cycle, without following them",
  );

  button("Two panes", ".view-menu").click();
  click("Close SSH fixture 2");
  await until(
    () => !!document.querySelector('[aria-label="Connections for pane 2"]'),
    "Empty right pane offers saved connections directly",
  );
  const chooser = document.querySelector(
    '[aria-label="Connections for pane 2"]',
  )!;
  if (chooser.textContent?.includes("New session"))
    throw new Error("Empty split pane still requires creating a profile");
  Array.from(chooser.querySelectorAll<HTMLButtonElement>("button"))
    .find((b) => b.textContent?.includes("SSH fixture 2"))!
    .click();
  await until(
    () => !!document.querySelector('.pane[data-profile-id="2"] .xterm-screen'),
    "Choosing a host opens its connection in the requested right pane",
  );
  const right = document.querySelector<HTMLElement>(
    '.pane[data-profile-id="2"]',
  )!;
  if (right.style.gridColumn !== "2")
    throw new Error("Host opened in the wrong pane");
  checks.push(
    "Pane chooser assigns the selected host to the intended split without duplicates",
  );
  click("Close SSH fixture 2");
  const layoutKey = (key: string) =>
    document
      .querySelector(".pane.focused .xterm-helper-textarea, body")!
      .dispatchEvent(
        new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }),
      );
  const workspace = () => document.querySelector(".workspace")!.className;
  layoutKey("F3");
  await until(() => workspace().includes("layout-4"), "F3 selects four panes");
  layoutKey("F1");
  await until(() => workspace().includes("layout-1"), "F1 selects one pane");
  if (document.querySelector(".empty-heading")?.textContent === "WORKSPACE")
    throw new Error("Workspace label is still shown");
  layoutKey("F2");
  await until(() => workspace().includes("layout-2"), "F2 selects two panes");
  checks.push("F1/F2/F3 switch layouts, also from a focused terminal");
  // Four panes with three tabs: the tabs fill three, the fourth offers only
  // sessions without a tab. Clicking a visible tab only moves the focus.
  const visible = () =>
    Array.from(document.querySelectorAll<HTMLElement>(".workspace > .pane"))
      .filter((e) => e.offsetParent && e.dataset.profileId)
      .map((e) => e.dataset.profileId!)
      .sort()
      .join(",");
  const tabs = () => Array.from(document.querySelectorAll<HTMLElement>(".tab"));
  const openIds = tabs().length;
  layoutKey("F3");
  await until(
    () => visible().split(",").length === Math.min(openIds, 4),
    "F3 fills the four panes with the open tabs",
  );
  const before = visible();
  if (openIds < 4) {
    const offer = document.querySelector(
      '[aria-label^="Connections for pane"]',
    );
    const names = Array.from(offer?.querySelectorAll("button") ?? []).map(
      (b) => b.textContent ?? "",
    );
    if (
      !names.length ||
      names.some((n) => tabs().some((t) => n.startsWith(t.textContent!.trim())))
    )
      throw new Error(
        `Chooser offers open tabs: ${names.join(" | ")} tabs=${tabs()
          .map((t) => t.textContent)
          .join(
            ",",
          )} panes=${document.querySelector<HTMLElement>(".workspace")!.dataset.panes} focused=${document.querySelector<HTMLElement>(".pane.focused")?.dataset.profileId} sessions=${document.querySelectorAll(".session-row").length}`,
      );
    checks.push("The empty fourth pane offers only sessions without a tab");
  }
  const target = tabs().find((t) => !t.classList.contains("active"))!;
  const name = target.textContent!.trim();
  target.querySelector<HTMLButtonElement>("button")!.click();
  await until(
    () =>
      tabs()
        .find((t) => t.textContent?.trim() === name)
        ?.classList.contains("active") === true,
    "Clicking a visible tab focuses its pane",
  );
  if (visible() !== before)
    throw new Error("Clicking a visible tab rearranged the panes");
  checks.push("Visible tabs keep their panes when clicked");
  // The pane switcher swaps two visible sessions.
  const order = () =>
    document.querySelector<HTMLElement>(".workspace")!.dataset.panes!;
  const swapBefore = order().split(",");
  document
    .querySelector<HTMLButtonElement>(
      '.pane.focused [aria-label="Switch session in this pane"]',
    )!
    .click();
  await until(
    () => !!document.querySelector(".context-menu button"),
    "The pane switcher lists the other open tabs",
  );
  Array.from(
    document.querySelectorAll<HTMLButtonElement>(".context-menu button"),
  )
    .find((b) => b.textContent?.includes("(swap with pane"))!
    .click();
  await until(() => {
    const now = order().split(",");
    return (
      now.join(",") !== swapBefore.join(",") &&
      [...now].sort().join(",") === [...swapBefore].sort().join(",")
    );
  }, "Choosing a visible tab swaps the two panes");
  await new Promise((r) => setTimeout(r, 300));
  await invoke("capture_still", { name: "panes-four" });
  await new Promise((r) => setTimeout(r, 1500));
  layoutKey("F2");
  await until(() => workspace().includes("layout-2"), "Back to two panes");
  // Closing a split pane's tab shows the last open tab there (never fixture 1 moved).
  await until(() => {
    const right = Array.from(
      document.querySelectorAll<HTMLElement>(".workspace > .pane"),
    ).find((e) => e.style.gridColumn === "2" && e.offsetParent);
    const id = right?.dataset.profileId;
    return !!id && id !== "1" && id !== "2";
  }, "The freed right pane shows the last open tab");
  if (pane().style.gridColumn !== "1")
    throw new Error("Closing a split tab moved the other pane's session");
  pane().querySelector<HTMLButtonElement>(".pane-title")!.click();
  click("Show remote files");
  await until(
    () => path()?.value === "/fixture",
    "Final capture shows toolbar and active files beside the right pane",
  );
  await invoke("session_browse", { id: liveId(), path: "/fixture" });
  await until(
    () =>
      !document
        .querySelector(".file-status")
        ?.textContent?.includes("No such file"),
    "Final browser status is refreshed after the intentional negative tests",
  );
  document
    .querySelector<HTMLButtonElement>('[aria-label="Dismiss notification"]')
    ?.click();
  pick("logs");
  pick("read me.txt", { ctrlKey: true });
  await until(
    () =>
      selectedNames() === "logs,read me.txt" &&
      !!document
        .querySelector(".file-status")
        ?.textContent?.includes("2 selected"),
    "Final capture shows a two-item click selection",
  );

  // Each theme recolors app chrome and the live terminal; stills for the timeline.
  const chooseTheme = async (id: string) => {
    document
      .querySelector<HTMLButtonElement>('[aria-label="Settings"]')!
      .click();
    await until(
      () => !!document.querySelector("#settings-panel-appearance select"),
      `Settings open for ${id}`,
    );
    const select = document.querySelector<HTMLSelectElement>(
      "#settings-panel-appearance select",
    )!;
    select.value = id;
    select.dispatchEvent(new Event("change", { bubbles: true }));
    Array.from(document.querySelectorAll<HTMLButtonElement>("dialog button"))
      .find((b) => b.textContent?.trim() === "Save settings")!
      .click();
    await until(() => !document.querySelector("dialog[open]"), `${id} saved`);
  };
  const rgb = (hex: string) =>
    `rgb(${[1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(", ")})`;
  for (const [id, surface] of [
    ["tokyo-night", "#1a1b26"],
    ["dracula", "#282a36"],
    ["nord", "#2e3440"],
    ["gruvbox-dark", "#282828"],
  ]) {
    await chooseTheme(id);
    await until(
      () =>
        background() === rgb(surface) &&
        getComputedStyle(document.documentElement).backgroundColor ===
          rgb(surface),
      `${id} recolors the live terminal and the app`,
    );
    await new Promise((r) => setTimeout(r, 250));
    await invoke("capture_still", { name: "theme-" + id });
    // The WebView writes the still asynchronously; hold the theme until then.
    await new Promise((r) => setTimeout(r, 1500));
  }
  checks.push("Four more themes recolor app chrome and the live terminal");
  await chooseTheme("catppuccin-mocha");
}
