/** A connection owns one draft. No callbacks until explicit submission. */
export class TelnetLineDraft {
  #text = "";
  #busy = false;
  get text() {
    return this.#text;
  }
  get busy() {
    return this.#busy;
  }
  change(value: string): string | null {
    if (this.busy) return "A line is being sent";
    if (value.length > 4096 || new TextEncoder().encode(value).length > 4096)
      return "Local line is limited to 4096 UTF-8 bytes";
    if (/[\x00-\x1f\x7f-\x9f\u2028\u2029]/.test(value))
      return "Use one line without control characters";
    // Refuse unpaired UTF-16 surrogates instead of silently replacing them on the wire.
    if (
      Array.from(value).some(
        (c) =>
          c.length === 1 &&
          c.charCodeAt(0) >= 0xd800 &&
          c.charCodeAt(0) <= 0xdfff,
      )
    )
      return "Use valid Unicode text";
    this.#text = value;
    return null;
  }
  append(value: string): string | null {
    return this.change(this.text + value);
  }
  clear() {
    if (!this.busy) this.#text = "";
  }
  async submit(send: (text: string) => Promise<unknown>): Promise<boolean> {
    if (this.busy) return false;
    this.#busy = true;
    try {
      // Enter-key semantics: the TELNET codec sends CR LF in NVT mode and a
      // bare CR once BINARY is agreed, where a raw CR LF would be two lines.
      await send(this.text + "\r");
      this.#text = "";
      return true;
    } finally {
      this.#busy = false;
    }
  }
}
