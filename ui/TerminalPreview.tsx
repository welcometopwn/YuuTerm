import { useEffect, type CSSProperties, type ReactNode } from "react";
import type { ITheme } from "@xterm/xterm";
import { fontStack, loadTerminalFont } from "./terminal-font";
import type { CursorStyle } from "./types";

type Color = "green" | "blue" | "yellow" | "red" | "cyan" | "magenta";

// xterm.js defaults for themes that leave the ANSI colors unset.
const xtermDefaults: Record<Color, string> = {
  red: "#cd3131",
  green: "#0dbc79",
  yellow: "#e5e510",
  blue: "#2472c8",
  magenta: "#bc3fbc",
  cyan: "#11a8cd",
};

export function TerminalPreview({
  theme,
  fontFamily,
  fontSize,
  cursorStyle,
  blink,
}: {
  theme: ITheme;
  fontFamily: string;
  fontSize: number;
  cursorStyle: CursorStyle;
  blink: boolean;
}) {
  useEffect(() => {
    void loadTerminalFont(fontFamily);
  }, [fontFamily]);
  const c = (color: Color, text: ReactNode, bold = false) => (
    <span
      style={{
        color: theme[color] ?? xtermDefaults[color],
        fontWeight: bold ? 700 : undefined,
      }}
    >
      {text}
    </span>
  );
  const prompt = (
    <>
      {c("green", "user@server", true)}:{c("blue", "~", true)}${" "}
    </>
  );
  const style: CSSProperties = {
    background: theme.background,
    color: theme.foreground,
    fontFamily: fontStack(fontFamily),
    fontSize: Math.min(32, Math.max(8, fontSize || 14)),
  };
  return (
    <div
      className="terminal-preview"
      style={style}
      aria-label="Terminal preview"
    >
      <div>{prompt}ls -l</div>
      <div>
        {c("blue", "drwxr-xr-x")}
        {" 2 user user 4096 "}
        {c("blue", "projects", true)}
      </div>
      <div>
        {"-rwxr-xr-x 1 user user  812 "}
        {c("green", "deploy.sh", true)}
      </div>
      <div>{"-rw-r--r-- 1 user user  163 notes.txt"}</div>
      <div>{prompt}systemctl status nginx</div>
      <div>
        {c("green", "●")} nginx.service - {c("yellow", "warning:")} reload
        pending · {c("red", "error", true)} 0 · {c("cyan", "192.0.2.5")}{" "}
        {c("magenta", "0x1f")}
      </div>
      <div>
        {prompt}
        <span
          className={`terminal-preview-cursor ${cursorStyle.toLowerCase()} ${blink ? "blink" : ""}`}
          style={{ color: theme.cursor ?? theme.foreground }}
          aria-label={`${cursorStyle} cursor${blink ? ", blinking" : ""}`}
        >
          {" "}
        </span>
      </div>
    </div>
  );
}
