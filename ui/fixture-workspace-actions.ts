// Synthetic DOM actions in the actual native SSH fixture; no new connection.
export async function checkWorkspaceActions(
  until: (
    test: () => boolean | Promise<boolean>,
    label: string,
  ) => Promise<void>,
  checks: string[],
) {
  const close = () =>
    document
      .querySelector<HTMLButtonElement>(
        'dialog[open] [aria-label="Close dialog"]',
      )!
      .click();
  close();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Workspace is visible after closing the SSH editor",
  );
  if (
    document.querySelector('.workspace-actions [aria-label="New session"]') ||
    document.querySelector(".workspace-actions .lucide-plus")
  )
    throw Error("Top-right New session action remains");
  checks.push(
    "Top-right workspace actions contain no New session button or plus icon",
  );
  const identities = () =>
    Array.from(document.querySelectorAll<HTMLElement>("section[data-live-id]"))
      .map((p) => p.dataset.liveId)
      .join(",");
  const before = identities();
  document.querySelector<HTMLButtonElement>(".sidebar .new-session")!.click();
  await until(
    () => !!document.querySelector('dialog[open][aria-label="New session"]'),
    "Sidebar New session still opens the connection editor",
  );
  close();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Sidebar creation can be cancelled without creating a connection",
  );
  const context = () =>
    document.querySelector(".brand")!.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        clientX: 1100,
        clientY: 100,
      }),
    );
  context();
  await until(
    () => !!document.querySelector(".context-menu"),
    "Workspace context menu remains available",
  );
  const entry = Array.from(
    document.querySelectorAll<HTMLButtonElement>(".context-menu button"),
  ).find((b) => b.textContent?.trim() === "New session");
  if (!entry) throw Error("Context-menu New session was removed");
  entry.click();
  await until(
    () => !!document.querySelector('dialog[open][aria-label="New session"]'),
    "Context-menu New session still opens the connection editor",
  );
  close();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Context-menu creation can be cancelled",
  );
  if (identities() !== before)
    throw Error("Creation controls replaced a live session");
  checks.push(
    "Both retained creation paths preserve all four original SSH session identities",
  );
  context();
  await until(
    () => !!document.querySelector(".context-menu"),
    "Native capture retains the context-menu New session entry alongside the sidebar button",
  );
}
