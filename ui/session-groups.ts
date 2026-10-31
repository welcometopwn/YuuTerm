// Sidebar session groups: ungrouped sessions first, then groups by name.
import type { Profile } from "./types.ts";

export interface SessionGroup {
  /** Empty for ungrouped sessions. */
  name: string;
  profiles: Profile[];
}

const byName = (a: string, b: string) =>
  a.localeCompare(b, undefined, { sensitivity: "base", numeric: true });

/** Groups in display order; sessions keep their saved order inside a group. */
export function groupSessions(profiles: Profile[]): SessionGroup[] {
  const groups = new Map<string, Profile[]>();
  for (const p of profiles) {
    const name = p.group?.trim() ?? "";
    const list = groups.get(name);
    if (list) list.push(p);
    else groups.set(name, [p]);
  }
  return [...groups]
    .map(([name, list]) => ({ name, profiles: list }))
    .sort((a, b) => (!a.name ? -1 : !b.name ? 1 : byName(a.name, b.name)));
}

/** Existing group names, for suggestions in the session editor. */
export const groupNames = (profiles: Profile[]) =>
  [...new Set(profiles.map((p) => p.group?.trim() ?? "").filter(Boolean))].sort(
    byName,
  );

const key = "yuuterm.collapsedGroups";

/** Collapsed groups are a per-machine view preference; failures are ignored. */
export function loadCollapsed(): Set<string> {
  try {
    const raw = JSON.parse(localStorage.getItem(key) ?? "[]");
    return new Set(
      Array.isArray(raw)
        ? raw.filter((g) => typeof g === "string").slice(0, 500)
        : [],
    );
  } catch {
    return new Set();
  }
}

export function saveCollapsed(groups: Set<string>) {
  try {
    localStorage.setItem(key, JSON.stringify([...groups]));
  } catch {
    // The view still works without remembering.
  }
}
