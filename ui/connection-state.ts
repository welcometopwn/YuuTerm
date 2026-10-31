import type { Live } from "./types";

/** A live incarnation without terminal metadata is still attempting to connect. */
export function connectionState(live?: Pick<Live, "meta">) {
  if (!live || live.meta?.finished) return "Disconnected";
  return live.meta?.connected ? "Connected" : "Connecting…";
}
