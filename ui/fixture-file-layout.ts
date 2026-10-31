import { invoke } from "@tauri-apps/api/core";
import { layoutStorageKey, readWorkspaceLayout } from "./workspace-layout";

/** Real two-process WebView storage qualification over loopback SSH/SFTP. */
export async function checkFileLayout(prepare: boolean) {
  const checks: string[] = [];
  const pause = () => new Promise((resolve) => setTimeout(resolve, 40));
  const until = async (predicate: () => boolean, label: string) => {
    const deadline = Date.now() + 8000;
    while (!predicate()) {
      if (Date.now() > deadline) throw Error(label);
      await pause();
    }
    checks.push(label);
  };
  const click = (label: string) =>
    document
      .querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!
      .click();
  const headerOrder = () =>
    Array.from(document.querySelectorAll<HTMLElement>(".file-column"))
      .map((column) => column.dataset.column)
      .join(",");
  const rowOrder = () =>
    Array.from(
      document.querySelectorAll(
        '.file-row[data-name="read me.txt"] > button:first-child > span',
      ),
    )
      .map((cell) => cell.className.replace("file-", ""))
      .join(",");
  const key = (element: Element, key: string, altKey = false) =>
    element.dispatchEvent(
      new KeyboardEvent("keydown", {
        key,
        altKey,
        bubbles: true,
        cancelable: true,
      }),
    );
  try {
    await until(
      () => !!document.querySelector('.file-row[data-name="read me.txt"]'),
      "Real SFTP listing is ready in this app process",
    );
    if (prepare) {
      click("Widen file browser");
      await until(
        () =>
          document.querySelector(".sidebar")!.getBoundingClientRect().width ===
          740,
        "User widen action updates the sidebar",
      );
      key(
        document.querySelector('[aria-label="Resize sidebar"]')!,
        "ArrowRight",
      );
      await until(
        () =>
          document.querySelector(".sidebar")!.getBoundingClientRect().width ===
          760,
        "User resize action updates the sidebar to 760 pixels",
      );
      for (const column of ["name", "size", "modified", "owner"]) {
        const handle = document.querySelector<HTMLElement>(
          `.file-column[data-column="${column}"] [role="separator"]`,
        )!;
        const width = Number(handle.getAttribute("aria-valuenow"));
        key(handle, "ArrowRight");
        await until(
          () => Number(handle.getAttribute("aria-valuenow")) === width + 16,
          `${column} width saved after a user resize`,
        );
      }
      const source = document.querySelector<HTMLElement>(
        '[data-sort-column="owner"]',
      )!;
      const target = document.querySelector<HTMLElement>(
        '[data-sort-column="name"]',
      )!;
      const dataTransfer = new DataTransfer();
      source.dispatchEvent(
        new DragEvent("dragstart", {
          dataTransfer,
          bubbles: true,
          cancelable: true,
        }),
      );
      target.dispatchEvent(
        new DragEvent("drop", {
          dataTransfer,
          bubbles: true,
          cancelable: true,
        }),
      );
      await until(
        () =>
          headerOrder() === "owner,name,size,modified" &&
          rowOrder() === "owner,name,size,modified",
        "Header drag reorders both headers and actual row cells",
      );
      const size = () =>
        document.querySelector<HTMLButtonElement>('[data-sort-column="size"]')!;
      size().click();
      await until(
        () => size().dataset.direction === "asc",
        "Size sort selected",
      );
      size().click();
      await until(
        () => size().dataset.direction === "desc",
        "Descending size sort saved",
      );
      // Store the collapsed state as the final action before this process exits.
      click("Toggle sidebar");
      await until(
        () =>
          document.querySelector(".app")!.classList.contains("sidebar-hidden"),
        "Collapsed sidebar saved before app shutdown",
      );
      const saved = readWorkspaceLayout(localStorage);
      if (
        saved.sidebarWidth !== 760 ||
        saved.sidebarOpen ||
        saved.columns.join(",") !== "owner,name,size,modified"
      )
        throw Error("Stored layout differs from the UI");
      checks.push(
        "Final layout is present in versioned persistent storage before exit",
      );
    } else {
      await until(
        () =>
          document.querySelector(".app")!.classList.contains("sidebar-hidden"),
        "Collapsed sidebar restored after a full app restart",
      );
      click("Toggle sidebar");
      await until(
        () =>
          document.querySelector(".sidebar")!.getBoundingClientRect().width ===
          760,
        "Sidebar width restored after a full app restart",
      );
      await until(
        () =>
          headerOrder() === "owner,name,size,modified" &&
          rowOrder() === "owner,name,size,modified",
        "Custom column order restored for both headers and row cells",
      );
      for (const [column, width] of Object.entries({
        name: 196,
        size: 106,
        modified: 186,
        owner: 216,
      })) {
        const header = document.querySelector<HTMLElement>(
          `.file-column[data-column="${column}"]`,
        )!;
        const cell = document.querySelector<HTMLElement>(
          `.file-row[data-name="read me.txt"] .file-${column}`,
        )!;
        await until(
          () =>
            header.getBoundingClientRect().width === width &&
            cell.getBoundingClientRect().width === width,
          `${column} width restored and aligned after restart`,
        );
      }
      await until(
        () =>
          document.querySelector<HTMLElement>('[data-sort-column="size"]')!
            .dataset.direction === "desc",
        "Descending size sort restored after restart",
      );
      const owner = document.querySelector('[data-sort-column="owner"]')!;
      key(owner, "ArrowRight", true);
      await until(
        () =>
          headerOrder() === "name,owner,size,modified" &&
          rowOrder() === "name,owner,size,modified",
        "Alt+Arrow also reorders matching headers and row cells",
      );
    }
    if ((localStorage.getItem(layoutStorageKey)?.length ?? 0) > 4096)
      throw Error("Unexpected layout storage size");
    await invoke("capture_report", { checks, passed: true });
    document.documentElement.dataset.fixtureChecks = "passed";
  } catch (error) {
    checks.push("FAILED: " + String(error));
    await invoke("capture_report", { checks, passed: false });
  }
}
