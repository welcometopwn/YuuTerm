import { version } from "../package.json";
export async function checkAbout(
  until: (f: () => boolean | Promise<boolean>, name: string) => Promise<void>,
) {
  Array.from(document.querySelectorAll<HTMLButtonElement>(".side-tabs button"))
    .find((b) => b.textContent?.trim() === "Sessions")!
    .click();
  await until(
    () =>
      document.querySelector(".sessions-footer .app-version")?.textContent ===
      `v${version}`,
    "Sidebar footer shows the package version",
  );
  const fonts = [
    "Fira Code",
    "Source Code Pro",
    "IBM Plex Mono",
    "Roboto Mono",
    "Iosevka",
  ];
  const loaded = await Promise.all(
    fonts.map((name) =>
      document.fonts
        .load(`700 14px ${JSON.stringify(name)}`, "Aa")
        .then((faces) => faces.length > 0),
    ),
  );
  await until(
    () => loaded.every(Boolean),
    "All five bundled programming fonts load in the native app",
  );
  const dev = document.querySelector(".brand .dev-build")?.textContent ?? "";
  await until(
    () => /^Dev( [0-9a-f]{7,})?$/.test(dev),
    "Local capture build carries the Dev title-bar badge",
  );
  document
    .querySelector<HTMLButtonElement>('button[aria-label="Settings"]')!
    .click();
  await until(
    () => !!document.getElementById("settings-tab-about"),
    "Settings dialog offers an About section",
  );
  const heights: number[] = [];
  for (const name of ["appearance", "highlighting", "statistics", "about"]) {
    document.getElementById(`settings-tab-${name}`)!.click();
    await new Promise((r) => setTimeout(r, 80));
    heights.push(document.querySelector("dialog")!.offsetHeight);
  }
  await until(
    () => heights.every((h) => h === heights[0]),
    "Settings dialog keeps one height across all sections",
  );
  await until(
    () =>
      document.querySelector("#settings-panel-about .link-button")
        ?.textContent === "github.com/welcometopwn/yuuterm",
    "About links the public GitHub project",
  );
  await until(() => {
    const panel = document.getElementById("settings-panel-about");
    const values = Array.from(panel?.querySelectorAll("dd") ?? []).map(
      (d) => d.textContent,
    );
    return (
      !!panel &&
      !panel.hidden &&
      values[0] === version &&
      values[1] === dev &&
      values[2] === "MIT"
    );
  }, "About shows version, Dev build and license");
}
