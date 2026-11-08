// Sidebar session groups: in the dragged order, else ungrouped first, then by name.
import type { Profile } from "./types.ts";

export interface SessionGroup {
  /** Empty for ungrouped sessions. */
  name: string;
  profiles: Profile[];
}

const byName = (a: string, b: string) =>
  a.localeCompare(b, undefined, { sensitivity: "base", numeric: true });

/**
 * Groups in display order; sessions keep their saved order inside a group.
 * `order` is the user's dragged group order (empty string = ungrouped); groups
 * missing from it follow alphabetically, with ungrouped sessions first.
 */
export function groupSessions(
  profiles: Profile[],
  order: string[] = [],
): SessionGroup[] {
  const rank = (name: string) => {
    const index = order.indexOf(name);
    return index >= 0 ? index : name ? Infinity : -1;
  };
  const groups = new Map<string, Profile[]>();
  for (const p of profiles) {
    const name = p.group?.trim() ?? "";
    const list = groups.get(name);
    if (list) list.push(p);
    else groups.set(name, [p]);
  }
  return [...groups]
    .map(([name, list]) => ({ name, profiles: list }))
    .sort((a, b) => {
      const ra = rank(a.name);
      const rb = rank(b.name);
      if (ra !== rb) return ra < rb ? -1 : 1;
      return byName(a.name, b.name);
    });
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

const orderKey = "yuuterm.groupOrder";

/** The dragged group order is a per-machine view preference, like collapsing. */
export function loadGroupOrder(): string[] {
  try {
    const raw = JSON.parse(localStorage.getItem(orderKey) ?? "[]");
    return Array.isArray(raw)
      ? raw.filter((g) => typeof g === "string").slice(0, 500)
      : [];
  } catch {
    return [];
  }
}

export function saveGroupOrder(order: string[]) {
  try {
    localStorage.setItem(orderKey, JSON.stringify(order));
  } catch {
    // The view still works without remembering.
  }
}
