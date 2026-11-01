import { execFileSync } from "node:child_process";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { version } from "./package.json";

function commit(): string {
  try {
    return execFileSync("git", ["rev-parse", "--short", "HEAD"], {
      encoding: "utf8",
    }).trim();
  } catch {
    return "";
  }
}

// Only GitHub Actions produces release builds; mark every other build so a
// local executable cannot be mistaken for a published one.
const release = process.env.GITHUB_ACTIONS === "true";
const build = `${release ? "Release" : "Dev"} ${commit()}`.trim();

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: "127.0.0.1" },
  build: { target: "es2022", sourcemap: false },
  define: {
    __YUUTERM_VERSION__: JSON.stringify(version),
    __YUUTERM_BUILD__: JSON.stringify(build),
    __YUUTERM_DEV_BUILD__: JSON.stringify(release ? "" : build),
  },
});
