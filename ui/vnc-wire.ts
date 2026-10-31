export type VncInput =
  | { kind: "key"; down: boolean; keysym: number }
  | { kind: "pointer"; buttons: number; x: number; y: number }
  | { kind: "releaseAll" }
  | { kind: "clipboard"; text: string }
  | { kind: "resize"; width: number; height: number };
const named: Record<string, number> = {
  Backspace: 0xff08,
  Tab: 0xff09,
  Enter: 0xff0d,
  Escape: 0xff1b,
  Insert: 0xff63,
  Delete: 0xffff,
  Home: 0xff50,
  End: 0xff57,
  PageUp: 0xff55,
  PageDown: 0xff56,
  ArrowLeft: 0xff51,
  ArrowUp: 0xff52,
  ArrowRight: 0xff53,
  ArrowDown: 0xff54,
  Shift: 0xffe1,
  Control: 0xffe3,
  Alt: 0xffe9,
  Meta: 0xffeb,
  CapsLock: 0xffe5,
  NumLock: 0xff7f,
};
export function clipboardError(text: string): string | null {
  if (text.length > 16384) return "Clipboard text exceeds 16 KiB";
  if ([...text].some((c) => c.codePointAt(0)! > 255))
    return "Classic VNC clipboard supports Latin-1 text only";
  return null;
}
export function keysym(key: string, location = 0): number | null {
  if (named[key])
    return (
      named[key] +
      (location === 2 && ["Shift", "Control", "Alt", "Meta"].includes(key)
        ? 1
        : 0)
    );
  if (/^F([1-9]|1[0-2])$/.test(key)) return 0xffbd + Number(key.slice(1));
  if ([...key].length !== 1) return null;
  const cp = key.codePointAt(0)!;
  return cp <= 255 ? cp : 0x01000000 + cp;
}
// Control_L/R and Alt_L/R. Windows reports AltGr as a synthetic left Control
// plus right Alt; servers must not see those around the produced character.
const altGrModifiers = new Set([0xffe3, 0xffe4, 0xffe9, 0xffea]);
export interface KeyLike {
  key: string;
  code: string;
  location?: number;
  ctrlKey?: boolean;
  altKey?: boolean;
  metaKey?: boolean;
  getModifierState?: (key: string) => boolean;
}
/**
 * Maps DOM keyboard events to RFB key events. Characters come from keypress,
 * which carries the layout's result (AltGr, dead-key composition); keydown
 * only covers named keys and Ctrl/Alt shortcuts, where no keypress follows.
 * A character keydown without keypress is sent on the next key event.
 */
export class VncKeyboard {
  private held = new Map<string, number>();
  private pending: { code: string; keysym: number; altGr: boolean } | null =
    null;
  /** `prevent` false lets the WebView produce keypress/paste for this key. */
  down(e: KeyLike): { inputs: VncInput[]; prevent: boolean } {
    const inputs = this.flush();
    if (["Dead", "AltGraph", "Process", "Unidentified"].includes(e.key))
      return { inputs, prevent: e.key === "AltGraph" };
    const symbol = keysym(e.key, e.location);
    if (symbol === null) return { inputs, prevent: true };
    const altGr = !!(
      e.getModifierState?.("AltGraph") ||
      (e.ctrlKey && e.altKey)
    );
    if (
      [...e.key].length === 1 &&
      (altGr || !(e.ctrlKey || e.altKey || e.metaKey))
    ) {
      this.pending = { code: e.code, keysym: symbol, altGr };
      return { inputs, prevent: false };
    }
    this.held.set(e.code, symbol);
    inputs.push({ kind: "key", down: true, keysym: symbol });
    return { inputs, prevent: true };
  }
  press(e: KeyLike): VncInput[] {
    const symbol = [...e.key].length === 1 ? keysym(e.key) : null;
    if (symbol === null) return [];
    const altGr = this.pending?.altGr ?? false;
    this.pending = null;
    return this.type(symbol, altGr);
  }
  up(e: KeyLike): VncInput[] {
    const inputs = this.flush();
    const symbol = this.held.get(e.code);
    if (symbol !== undefined) {
      this.held.delete(e.code);
      inputs.push({ kind: "key", down: false, keysym: symbol });
    }
    return inputs;
  }
  /** Forget everything; the backend releases what the server holds. */
  reset() {
    this.held.clear();
    this.pending = null;
  }
  /** Sends a deferred character before the caller handles a key itself. */
  flush(): VncInput[] {
    const pending = this.pending;
    this.pending = null;
    return pending ? this.type(pending.keysym, pending.altGr) : [];
  }
  private type(symbol: number, altGr: boolean): VncInput[] {
    const inputs: VncInput[] = [];
    if (altGr)
      for (const [code, held] of this.held)
        if (altGrModifiers.has(held)) {
          this.held.delete(code);
          inputs.push({ kind: "key", down: false, keysym: held });
        }
    inputs.push(
      { kind: "key", down: true, keysym: symbol },
      { kind: "key", down: false, keysym: symbol },
    );
    return inputs;
  }
}
/** Largest size that fits the box without changing the aspect ratio. */
export function fit(
  width: number,
  height: number,
  box: number,
  boxHeight: number,
) {
  if (width <= 0 || height <= 0 || box <= 0 || boxHeight <= 0) return null;
  const scale = Math.min(box / width, boxHeight / height);
  return {
    width: Math.max(1, Math.floor(width * scale)),
    height: Math.max(1, Math.floor(height * scale)),
  };
}
export function pointer(
  clientX: number,
  clientY: number,
  r: { left: number; top: number; width: number; height: number },
  width: number,
  height: number,
) {
  if (r.width <= 0 || r.height <= 0 || width <= 0 || height <= 0) return null;
  return {
    x: Math.max(
      0,
      Math.min(width - 1, Math.floor(((clientX - r.left) * width) / r.width)),
    ),
    y: Math.max(
      0,
      Math.min(height - 1, Math.floor(((clientY - r.top) * height) / r.height)),
    ),
  };
}
export interface VncStatus {
  id: number;
  endpoint: string;
  status: string;
  connected: boolean;
  finished: boolean;
  width: number;
  height: number;
  challenge: number;
  clipboard_enabled: boolean;
  resize_enabled: boolean;
  resize_supported: boolean;
  resize_status: number | null;
  cursor: null | {
    serial: number;
    hotspot_x: number;
    hotspot_y: number;
    width: number;
    height: number;
  };
  prompt:
    | null
    | {
        kind: "certificate";
        subject: string;
        issuer: string;
        valid_from: string;
        valid_until: string;
        sha256: string;
      }
    | { kind: "credentials"; username: string; auth: "Plain" | "Vnc" };
}
export function packet(buffer: ArrayBuffer) {
  if (buffer.byteLength < 7 || buffer.byteLength > 17500000)
    throw Error("Invalid VNC packet size");
  const data = new DataView(buffer),
    length = data.getUint32(0);
  if (length > 20000 || length + 6 > buffer.byteLength)
    throw Error("Invalid VNC status length");
  const status = JSON.parse(
    new TextDecoder().decode(new Uint8Array(buffer, 4, length)),
  ) as VncStatus;
  if (
    !Number.isInteger(status.width) ||
    !Number.isInteger(status.height) ||
    status.width < 0 ||
    status.height < 0 ||
    status.width * status.height > 4194304
  )
    throw Error("Invalid VNC dimensions");
  let offset = length + 4;
  const count = data.getUint16(offset);
  offset += 2;
  if (count > 1024) throw Error("VNC tile count exceeds limit");
  const tiles: {
    x: number;
    y: number;
    width: number;
    height: number;
    rgba: Uint8ClampedArray<ArrayBuffer>;
  }[] = [];
  for (let i = 0; i < count; i++) {
    if (offset + 8 > buffer.byteLength) throw Error("Truncated VNC tile");
    const x = data.getUint16(offset),
      y = data.getUint16(offset + 2),
      width = data.getUint16(offset + 4),
      height = data.getUint16(offset + 6);
    offset += 8;
    const bytes = width * height * 4;
    if (
      !width ||
      !height ||
      width > 64 ||
      height > 64 ||
      x + width > status.width ||
      y + height > status.height ||
      offset + bytes > buffer.byteLength
    )
      throw Error("Invalid VNC tile geometry");
    tiles.push({
      x,
      y,
      width,
      height,
      rgba: new Uint8ClampedArray(buffer, offset, bytes),
    });
    offset += bytes;
  }
  if (offset >= buffer.byteLength) throw Error("Truncated VNC cursor flag");
  const flag = data.getUint8(offset++);
  let cursor: Uint8ClampedArray<ArrayBuffer> | null = null;
  if (flag > 1) throw Error("Invalid VNC cursor flag");
  if (flag) {
    const shape = status.cursor;
    if (!shape || shape.width > 128 || shape.height > 128)
      throw Error("Invalid VNC cursor shape");
    const bytes = shape.width * shape.height * 4;
    if (offset + bytes > buffer.byteLength) throw Error("Truncated VNC cursor");
    cursor = new Uint8ClampedArray(buffer, offset, bytes);
    offset += bytes;
  }
  if (offset !== buffer.byteLength) throw Error("Unexpected VNC packet tail");
  return { status, tiles, cursor };
}
