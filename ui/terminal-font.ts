import "./terminal-font.css";
// Popular open-source programming fonts, bundled so they work without an OS
// installation. Fira Code has no italic face; the browser slants it.
import "@fontsource/fira-code/400.css";
import "@fontsource/fira-code/700.css";
import "@fontsource/source-code-pro/400.css";
import "@fontsource/source-code-pro/700.css";
import "@fontsource/source-code-pro/400-italic.css";
import "@fontsource/source-code-pro/700-italic.css";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/700.css";
import "@fontsource/ibm-plex-mono/400-italic.css";
import "@fontsource/ibm-plex-mono/700-italic.css";
import "@fontsource/roboto-mono/400.css";
import "@fontsource/roboto-mono/700.css";
import "@fontsource/roboto-mono/400-italic.css";
import "@fontsource/roboto-mono/700-italic.css";
import "@fontsource/iosevka/400.css";
import "@fontsource/iosevka/700.css";
import "@fontsource/iosevka/400-italic.css";
import "@fontsource/iosevka/700-italic.css";

export const terminalFontFamily =
  '"YuuTerm JetBrains Mono", "Cascadia Mono", Consolas, monospace';

const bundledFonts = [
  "Fira Code",
  "Source Code Pro",
  "IBM Plex Mono",
  "Roboto Mono",
  "Iosevka",
];

export const terminalFontChoices = [
  "YuuTerm JetBrains Mono",
  ...bundledFonts,
  "Cascadia Mono",
  "Cascadia Code",
  "Consolas",
  "Lucida Console",
  "Courier New",
];
export function fontStack(name: string) {
  return /^[\p{L}\p{N} ._-]{1,128}$/u.test(name) && name.trim()
    ? `${JSON.stringify(name)}, ${terminalFontFamily}`
    : terminalFontFamily;
}

// Measure xterm cells only after the bundled faces load. All panes share this
// promise; no OS font installation or external font request is needed.
export const terminalFontsReady = Promise.all(
  ["400", "700", "italic 400", "italic 700"].map((style) =>
    document.fonts.load(
      `${style} 14px "YuuTerm JetBrains Mono"`,
      "Aa\ue0b0\uf120",
    ),
  ),
).then(
  (faces) => faces.every((loaded) => loaded.length > 0),
  () => false,
);

/** Loads a bundled optional font before xterm measures its cells. */
export function loadTerminalFont(name: string): Promise<void> {
  if (!bundledFonts.includes(name)) return Promise.resolve();
  return Promise.all(
    ["400", "700"].map((weight) =>
      document.fonts.load(`${weight} 14px ${JSON.stringify(name)}`, "Aa"),
    ),
  ).then(
    () => undefined,
    () => undefined,
  );
}
