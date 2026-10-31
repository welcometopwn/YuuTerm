import type { Highlighting } from "./types";

export const defaultHighlighting = (): Highlighting => ({
  enabled: false,
  ipv4: true,
  ipv6: true,
  ipv4_color: "#89b4fa",
  ipv6_color: "#89b4fa",
  rules: [
    {
      keywords: "error, failed, failure, fatal, denied",
      color: "#f38ba8",
      regex: false,
    },
    { keywords: "warning, warn, deprecated", color: "#f9e2af", regex: false },
    {
      keywords: "success, successful, passed, ready",
      color: "#a6e3a1",
      regex: false,
    },
  ],
});
