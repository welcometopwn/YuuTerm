// Wide Settings and session-editor layouts in the native window: cards side by
// side and no horizontal scrolling in any section. The run ends with one dialog open for the capture.
export async function checkDialogLayout(
  until: (f: () => boolean | Promise<boolean>, name: string) => Promise<void>,
  keepOpen: "settings" | "session",
) {
  const pause = () => new Promise((r) => setTimeout(r, 80));
  const dialog = () => document.querySelector<HTMLElement>("dialog[open]");
  const fits = () => {
    const d = dialog();
    return !!d && d.scrollWidth <= d.clientWidth + 1;
  };
  const box = (el: Element | null | undefined) => el!.getBoundingClientRect();

  document
    .querySelector<HTMLButtonElement>('button[aria-label="Settings"]')!
    .click();
  await until(
    () => !!document.getElementById("settings-tab-appearance"),
    "Settings open",
  );
  await until(() => {
    const cards = document.querySelectorAll(
      "#settings-panel-appearance > .settings-card",
    );
    return (
      cards.length === 2 &&
      Math.abs(box(cards[0]).top - box(cards[1]).top) < 2 &&
      box(cards[1]).left > box(cards[0]).right
    );
  }, "Settings show the appearance cards side by side");
  for (const name of [
    "general",
    "highlighting",
    "statistics",
    "commands",
    "about",
  ]) {
    document.getElementById(`settings-tab-${name}`)!.click();
    await pause();
    if (!fits()) throw Error(`Settings ${name} scrolls horizontally`);
  }
  await until(() => true, "No settings section scrolls horizontally");
  const before = box(dialog());
  document.getElementById("settings-tab-commands")!.click();
  await pause();
  Array.from(
    document.querySelectorAll<HTMLButtonElement>(".command-set button"),
  )
    .find((b) => b.textContent?.startsWith("RDP remote tools"))!
    .click();
  await until(() => {
    const names = Array.from(
      document.querySelectorAll(
        "#settings-panel-commands .command-category strong",
      ),
    ).map((i) => i.textContent ?? "");
    const after = box(dialog());
    return (
      names.includes("Services") &&
      names.includes("Event log") &&
      Math.abs(after.top - before.top) < 1 &&
      Math.abs(after.height - before.height) < 1 &&
      Math.abs(after.width - before.width) < 1
    );
  }, "Settings list the Windows remote-tools commands without the dialog moving");
  if (keepOpen === "settings") return;
  document.getElementById("settings-tab-appearance")!.click();
  await pause();
  document
    .querySelector<HTMLButtonElement>(
      'dialog[open] [aria-label="Close dialog"]',
    )!
    .click();
  await until(() => !dialog(), "Settings close");

  Array.from(document.querySelectorAll<HTMLButtonElement>(".side-tabs button"))
    .find((b) => b.textContent?.trim() === "Sessions")!
    .click();
  await pause();
  document.querySelector<HTMLElement>(".session-row")!.dispatchEvent(
    new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 100,
      clientY: 200,
    }),
  );
  await pause();
  Array.from(document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'))
    .find((b) => b.textContent?.trim() === "Edit session")!
    .click();
  await until(() => {
    const cards = document.querySelectorAll(
      "#session-panel-connection > .settings-card",
    );
    return (
      cards.length === 2 &&
      cards[0].querySelector("legend")?.textContent === "Session" &&
      cards[1].querySelector("legend")?.textContent === "Server" &&
      Array.from(cards[1].querySelectorAll("label")).some((l) =>
        l.textContent?.startsWith("Authentication"),
      ) &&
      Math.abs(box(cards[0]).top - box(cards[1]).top) < 2 &&
      box(cards[1]).left > box(cards[0]).right
    );
  }, "The SSH editor shows Session and Server (with sign-in) side by side");
  for (const name of [
    "gateway",
    "x11",
    "terminal",
    "statistics",
    "connection",
  ]) {
    document.getElementById(`session-tab-${name}`)!.click();
    await pause();
    if (!fits()) throw Error(`Session ${name} scrolls horizontally`);
  }
  await until(() => true, "No session section scrolls horizontally");
}
