type Step = { kind: "path"; path: string } | { kind: "parent" };
/** Chromium emits dblclick only for count 2 and can cap later click counts at
 * 3. Count further clicks ourselves, retaining single-click selection. */
export class FileClickPairs {
  private pending: string | null = null;
  continued(target: string, detail: number, button: number): boolean {
    if (button !== 0 || detail < 3) {
      this.pending = null;
      return false;
    }
    if (this.pending === target) {
      this.pending = null;
      return true;
    }
    this.pending = target;
    return false;
  }
}
/** Serialize navigation until SFTP confirms the canonical directory, not merely
 * until IPC accepts the command. Queues belong to one connection incarnation. */
export class FileNavigation {
  private id: number | null = null;
  private path = "";
  private epoch = 0;
  private running = false;
  private activePath: string | null = null;
  private queue: Step[] = [];
  private observedPath = "";
  private waitingForPath: string | null = null;
  private send: (id: number, path: string) => Promise<string>;
  private error: (message: unknown) => void;
  constructor(
    send: (id: number, path: string) => Promise<string>,
    error: (message: unknown) => void,
  ) {
    this.send = send;
    this.error = error;
  }
  sync(id: number | null, path: string, connected: boolean) {
    if (!connected || id !== this.id) {
      this.epoch++;
      this.queue = [];
      this.running = false;
      this.activePath = null;
      this.id = connected ? id : null;
      this.path = path;
      this.waitingForPath = null;
    } else {
      if (path === this.waitingForPath) this.waitingForPath = null;
      if (!this.running && this.waitingForPath === null) this.path = path;
    }
    this.observedPath = path;
  }
  navigate(id: number, step: Step) {
    if (id !== this.id || !this.path) return;
    const last = this.queue.at(-1);
    if (
      step.kind === "path" &&
      ((this.running && this.activePath === step.path && !this.queue.length) ||
        (last?.kind === "path" && last.path === step.path))
    )
      return;
    if (this.queue.length >= 32) {
      this.error(
        "Too many pending directory changes; wait for the current folder",
      );
      return;
    }
    this.queue.push(step);
    void this.drain();
  }
  dispose() {
    this.epoch++;
    this.id = null;
    this.queue = [];
    this.running = false;
  }
  private async drain() {
    if (this.running || this.id === null) return;
    this.running = true;
    const epoch = this.epoch;
    const id = this.id;
    while (this.queue.length && epoch === this.epoch) {
      const step = this.queue.shift()!;
      const path =
        step.kind === "parent"
          ? `${this.path.replace(/\/$/, "")}/..`
          : step.path;
      this.activePath = path;
      try {
        const canonical = await this.send(id, path);
        if (epoch !== this.epoch) return;
        this.path = canonical;
        this.waitingForPath =
          this.observedPath === canonical ? null : canonical;
      } catch (error) {
        if (epoch !== this.epoch) return;
        this.queue = [];
        this.error(error);
        break;
      }
    }
    if (epoch === this.epoch) {
      this.running = false;
      this.activePath = null;
    }
  }
}
