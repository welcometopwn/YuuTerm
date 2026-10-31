import "./terminal-font.css";

export const terminalFontFamily =
  '"YuuTerm JetBrains Mono", "Cascadia Mono", Consolas, monospace';

export const terminalFontChoices = [
  "YuuTerm JetBrains Mono",
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
