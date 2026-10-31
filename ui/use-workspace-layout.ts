import { useSyncExternalStore } from "react";
import {
  layoutStorageKey,
  readWorkspaceLayout,
  normalizeWorkspaceLayout,
  defaultWorkspaceLayout,
  type WorkspaceLayout,
} from "./workspace-layout";

let current: WorkspaceLayout | undefined;
const listeners = new Set<() => void>();
function snapshot(): WorkspaceLayout {
  if (!current) {
    try {
      current = readWorkspaceLayout(localStorage);
    } catch {
      current = defaultWorkspaceLayout();
    }
  }
  return current;
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
/** Persist synchronously before publishing: closing immediately retains the last edit. */
export function updateWorkspaceLayout(
  patch: Partial<Omit<WorkspaceLayout, "version">>,
) {
  const next = normalizeWorkspaceLayout({ ...snapshot(), ...patch });
  localStorage.setItem(layoutStorageKey, JSON.stringify(next));
  current = next;
  listeners.forEach((listener) => listener());
}
export function useWorkspaceLayout(): WorkspaceLayout {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}
