import { invoke } from "@tauri-apps/api/core";
export async function checkFileNavigation(
  ids: number[],
  until: (f: () => boolean | Promise<boolean>, label: string) => Promise<void>,
  checks: string[],
) {
  const path = () =>
    document.querySelector<HTMLInputElement>('[aria-label="Remote path"]')
      ?.value;
  await until(
    () => path() === "/fixture",
    "Real loopback SFTP initial directory is visible",
  );
  await invoke("session_browse", { id: ids[0], path: "/fixture/logs/one" });
  await until(
    () => path() === "/fixture/logs/one",
    "Nested real SFTP directory is visible before the replay",
  );
  const parent = document.querySelector<HTMLButtonElement>(
    '[aria-label="Go to parent directory"]',
  )!;
  parent.dispatchEvent(
    new MouseEvent("dblclick", { bubbles: true, detail: 2 }),
  );
  parent.dispatchEvent(
    new MouseEvent("dblclick", { bubbles: true, detail: 2 }),
  );
  await until(
    () => path() === "/fixture",
    "Two rapid parent-directory double-clicks preserve both navigation steps",
  );
  checks.push(
    "Rapid folder replay finishes at the requested parent without retry clicks",
  );
  const folder = (name: string) =>
    document.querySelector<HTMLButtonElement>(
      `.file-row[data-name="${name}"] > button`,
    )!;
  for (let round = 0; round < 6; round++) {
    const logs = folder("logs");
    for (let click = 0; click < 8; click++)
      logs.dispatchEvent(
        new MouseEvent("dblclick", { bubbles: true, detail: 2 }),
      );
    await until(
      () => path() === "/fixture/logs",
      `Rapid folder-open burst ${round + 1} reaches its destination`,
    );
    folder("one").dispatchEvent(
      new MouseEvent("dblclick", { bubbles: true, detail: 2 }),
    );
    await Promise.resolve();
    parent.dispatchEvent(
      new MouseEvent("dblclick", { bubbles: true, detail: 2 }),
    );
    parent.dispatchEvent(
      new MouseEvent("dblclick", { bubbles: true, detail: 2 }),
    );
    await until(
      () => path() === "/fixture",
      `Rapid enter/parent/parent cycle ${round + 1} preserves its final destination`,
    );
  }
  for (const id of ids) await invoke("session_input", { id, bytes: [] });
  // Chromium only generates dblclick for click count 2. A continuing native
  // multi-click sequence delivers click counts capped at 3 without more dblclicks.
  // Replay those clicks instead of manufacturing another dblclick event.
  await invoke("session_browse", { id: ids[0], path: "/fixture/logs/one" });
  await until(
    () => path() === "/fixture/logs/one",
    "Continuous click replay starts in nested directory",
  );
  for (const detail of [1, 2])
    parent.dispatchEvent(new MouseEvent("click", { bubbles: true, detail }));
  parent.dispatchEvent(
    new MouseEvent("dblclick", { bubbles: true, detail: 2 }),
  );
  await until(
    () => path() === "/fixture/logs",
    "First click pair opens the parent exactly once",
  );
  for (const detail of [3, 3])
    parent.dispatchEvent(new MouseEvent("click", { bubbles: true, detail }));
  await until(
    () => path() === "/fixture",
    "Continuing clicks capped at count 3 open the next parent without a pause",
  );
  for (const detail of [3, 3])
    folder("logs").dispatchEvent(
      new MouseEvent("click", { bubbles: true, detail }),
    );
  await until(
    () => path() === "/fixture/logs",
    "Continuing clicks capped at count 3 enter a folder without a pause",
  );
  for (const detail of [3, 3])
    folder("one").dispatchEvent(
      new MouseEvent("click", { bubbles: true, detail }),
    );
  await until(
    () => path() === "/fixture/logs/one",
    "Continuing clicks capped at count 3 enter the next folder without a pause",
  );
  for (const detail of [3, 3, 3, 3])
    parent.dispatchEvent(new MouseEvent("click", { bubbles: true, detail }));
  await until(
    () => path() === "/fixture",
    "Two continuing parent click pairs preserve both steps",
  );
  await new Promise((resolve) => setTimeout(resolve, 150));
  if (path() !== "/fixture")
    throw new Error("Continuous click pairs opened a directory more than once");
  checks.push(
    "Continuous click pairs finish at the exact requested directory with no extra opens",
  );
  checks.push(
    "All four original SSH connections remain active after rapid SFTP navigation",
  );
}
