import { readFileSync, existsSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = (name) => readFileSync(path.join(root, name), "utf8");
const pkg = JSON.parse(read("package.json"));
const ansi = (code, text) => `\x1b[${code}m${text}\x1b[0m`;
const blue = (text) => ansi("38;2;137;180;250", text);
const green = (text) => ansi("38;2;166;227;161", text);
const muted = (text) => ansi("38;2;147;153;178", text);
const lilac = (text) => ansi("38;2;203;166;247", text);
const output = [];
const line = (text = "") => output.push(`  ${text}`);
const section = (text) => { line(muted(text)); line(muted("─".repeat(43))); };
const mode = process.argv[2];

await new Promise((resolve) => setTimeout(resolve, 1000));

line(blue(`YuuTerm ${pkg.version}`) + muted(`  /  ${mode === "project" ? "workspace" : "release"}`));

line();
line(green(">") + ` node scripts/readme-demo.mjs ${mode}`);

if (mode === "project") {
  section("PROJECT");
  const files = ["crates/yuuterm-core", "crates/yuuterm-ssh", "crates/yuuterm-desktop", "ui", "scripts", "Cargo.toml", "package.json", "SECURITY.md", "LICENSE"];
  for (const [index, name] of files.entries()) {
    const directory = statSync(path.join(root, name)).isDirectory();
    line(muted(index === files.length - 1 ? "└─ " : "├─ ") + (directory ? blue(name + "/") : name));
  }
  section("Cargo.toml");
  const text = read("Cargo.toml");
  const lines = text.split("\n");
  const start = lines.findIndex((item) => item === "[workspace.package]");
  for (let index = start; index < start + 6; index++) {
    const value = lines[index];
    const highlighted = value.startsWith("[") ? lilac(value) : value.replace(/^(\w+)( = )(.+)$/, (_, key, equal, literal) => blue(key) + equal + green(literal));
    line(muted(String(index + 1).padStart(2) + "  ") + highlighted);
  }
  section("WORKSPACE");
  line("Rust transports   ·   Tauri + React");
  line("SSH / SFTP  RDP  VNC  TELNET  SERIAL");
} else if (mode === "checks") {
  section("LOCAL METADATA CHECKS");
  const installer = JSON.parse(read("crates/yuuterm-desktop/tauri.installer.conf.json"));
  const app = JSON.parse(read("crates/yuuterm-desktop/tauri.conf.json"));
  const checks = [
    ["Version metadata agrees", pkg.version === app.version],
    ["MIT license is present", read("LICENSE").startsWith("MIT License")],
    ["Security notes are present", existsSync(path.join(root, "SECURITY.md"))],
    ["Dependency notices are present", existsSync(path.join(root, "THIRD_PARTY_NOTICES.md"))],
    ["Installer uses per-user scope", installer.bundle.windows.nsis.installMode === "currentUser"],
    ["Public publisher is configured", installer.bundle.publisher === "welcometopwn"],
  ];
  for (const [label, passed] of checks) line((passed ? green("PASS") : ansi("31", "FAIL")) + "  " + label);
  if (checks.some(([, passed]) => !passed)) process.exitCode = 1;
  section("RELEASE FILES");
  for (const [name, label] of [[`YuuTerm-${pkg.version}-x64-setup.exe`, "Windows installer"], [`YuuTerm-${pkg.version}-portable.zip`, "Portable ZIP"], [`YuuTerm-${pkg.version}-x11-sources.zip`, "Bundled X11 sources"]]) {
    const file = path.join(root, "target/distribution", name);
    line(blue(label.padEnd(24)) + (existsSync(file) ? muted(`${(statSync(file).size / 1048576).toFixed(1)} MB`) : muted("not built")));
  }
  section("TERMINAL PALETTE");
  line(["41", "42", "43", "44", "45", "46", "47"].map((color) => ansi(color, "   ")).join(" "));
  section("PREVIEW");
  line(lilac("0.2.0") + "  ·  Windows x64  ·  unsigned");
}
line();
line(muted("Local demonstration · no remote connections"));

process.stdout.write("\x1b[2J\x1b[H" + output.join("\r\n") + "\r\n");
