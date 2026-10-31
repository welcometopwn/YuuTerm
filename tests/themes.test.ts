import { test } from "node:test";
import assert from "node:assert/strict";
import { themeFor, themes } from "../ui/themes.ts";

const hex = /^#[0-9a-f]{6}$/;

test("themes have unique ids and complete #rrggbb palettes", () => {
  assert.equal(new Set(themes.map((t) => t.id)).size, themes.length);
  for (const t of themes) {
    assert.match(t.id, /^[a-z0-9-]{1,32}$/);
    for (const color of Object.values(t.ui)) assert.match(color, hex, t.id);
    const colors = Object.values(t.terminal) as string[];
    for (const color of colors) assert.match(color, hex, t.id);
    // Neutral keeps xterm's default ANSI colors; every other theme sets all 16.
    if (t.id !== "neutral") assert.equal(colors.length, 4 + 16, t.id);
  }
});

test("older settings map the mocha flag; unknown ids fall back", () => {
  assert.equal(themeFor("", true).id, "catppuccin-mocha");
  assert.equal(themeFor("", false).id, "neutral");
  assert.equal(themeFor("nord", true).id, "nord");
  assert.equal(themeFor("removed-theme", false).id, "neutral");
});
