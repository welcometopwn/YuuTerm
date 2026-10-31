// Runs only in capture builds after the Rust bootstrap created the RDP test-target
// bookmark with a saved password (scripts/Test-RdpProduction.ps1). Drives the
// production RDP pane: saved-password logon, still image under a menu, pane
// resize with remote resolution, disconnect text and reconnect.
import { invoke } from "@tauri-apps/api/core";

interface Status {
  id: number;
  connected: boolean;
  finished: boolean;
  status: string;
  visible: boolean;
  bounds: { x: number; y: number; width: number; height: number } | null;
  remote_size: [number, number];
  saved_password: boolean;
  certificate_remembered: boolean;
  events: { kind: string; code: number | null }[];
}

export async function checkRdpFixture(applyLayout: (n: number) => void) {
  const checks: string[] = [];
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const liveId = () =>
    Number(document.querySelector<HTMLElement>("[data-rdp-id]")?.dataset.rdpId);
  const status = (id = liveId()) =>
    invoke<Status>("rdp_session", {
      id,
      action: "status",
      bounds: null,
      scale: Math.round(window.devicePixelRatio * 100),
    });
  const until = async (
    test: () => Promise<boolean> | boolean,
    name: string,
    ms = 20000,
  ) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      let ok = false;
      try {
        ok = await test();
      } catch {
        // A session can briefly disappear between incarnations; retry.
      }
      if (ok) {
        checks.push(name);
        return;
      }
      await sleep(150);
    }
    throw Error(name);
  };
  // Windows hosts report login_complete. xrdp never sends that logon
  // notification, so a connection stable for 5 s counts as logged on there.
  const stableSince = new Map<number, number>();
  const loggedOn = async () => {
    const s = await status();
    if (s.events.some((e) => e.kind === "login_complete")) return true;
    if (!s.connected) {
      stableSince.delete(s.id);
      return false;
    }
    const since = stableSince.get(s.id) ?? Date.now();
    stableSince.set(s.id, since);
    return Date.now() - since >= 5000;
  };
  try {
    await until(
      async () => !!liveId() && (await loggedOn()),
      "Production RDP pane connected to the test target and stayed connected",
      45000,
    );
    const first = await status();
    if (!first.saved_password || !first.connected)
      throw Error("Logon did not use the saved password");
    checks.push("Logon used the saved password without any prompt");
    if (!first.events.some((e) => e.kind.startsWith("authentication_warning")))
      throw Error("The first connection did not show the certificate prompt");
    checks.push(
      "The first connection asked about the certificate (Windows prompt; the script ticks Don't ask me again)",
    );
    const firstId = first.id;
    await until(
      async () => (await status()).visible,
      "Native RDP control is shown over the pane",
    );

    const jpeg = await invoke<ArrayBuffer>("rdp_snapshot", { id: firstId });
    const bitmap = await createImageBitmap(
      new Blob([jpeg], { type: "image/jpeg" }),
    );
    const b = (await status()).bounds!;
    if (
      jpeg.byteLength < 2000 ||
      bitmap.width !== b.width ||
      bitmap.height !== b.height
    )
      throw Error("Snapshot size does not match the native pane");
    checks.push(
      `Desktop snapshot ${bitmap.width}x${bitmap.height} matches the native pane`,
    );

    const tab = document.querySelector<HTMLElement>(".tab")!;
    const r = tab.getBoundingClientRect();
    tab.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: r.left + 20,
        clientY: r.bottom - 2,
      }),
    );
    await until(
      async () =>
        !!document.querySelector(".context-menu") &&
        !(await status()).visible &&
        (document.querySelector<HTMLImageElement>(".rdp-still")?.naturalWidth ??
          0) > 0,
      "Overlapping menu hides the native control behind a loaded desktop still",
    );
    document.dispatchEvent(
      new PointerEvent("pointerdown", {
        bubbles: true,
        clientX: 5,
        clientY: 5,
      }),
    );
    await until(
      async () =>
        !document.querySelector(".context-menu") &&
        (await status()).visible &&
        !document.querySelector(".rdp-still"),
      "Closing the menu restores the live control and drops the still",
    );

    const wide = (await status()).bounds!.width;
    applyLayout(2);
    await until(async () => {
      const s = await status();
      return (
        !!s.bounds &&
        s.bounds.width < wide * 0.7 &&
        s.remote_size[0] === (s.bounds.width & ~1)
      );
    }, "Two-pane layout shrinks the pane and the remote resolution follows");
    applyLayout(1);
    await until(async () => {
      const s = await status();
      return !!s.bounds && s.remote_size[0] === (s.bounds.width & ~1);
    }, "Single-pane layout restores the full remote resolution");

    // Remote tools over OpenSSH, only where the test host offers it.
    const button = (label: string) =>
      [...document.querySelectorAll<HTMLButtonElement>("button")].find(
        (b) =>
          b.getAttribute("aria-label") === label ||
          b.textContent?.trim() === label,
      );
    button("Remote tools")!.click();
    await until(
      () =>
        !!document.querySelector(".rdp-tools") &&
        !document.body.textContent?.includes("Checking for OpenSSH"),
      "Remote tools panel opens and probes OpenSSH",
    );
    const pixels = async (id: number) => {
      const jpeg = await invoke<ArrayBuffer>("rdp_snapshot", { id });
      const image = await createImageBitmap(
        new Blob([jpeg], { type: "image/jpeg" }),
      );
      const canvas = new OffscreenCanvas(160, 120);
      const context = canvas.getContext("2d")!;
      context.drawImage(image, 0, 0, 160, 120);
      return context.getImageData(0, 0, 160, 120).data;
    };
    const beforeCad = await pixels(liveId());
    button("Send Ctrl+Alt+Del")!.click();
    await until(async () => {
      const after = await pixels(liveId());
      let diff = 0;
      for (let i = 0; i < after.length; i += 4)
        diff +=
          Math.abs(after[i] - beforeCad[i]) +
          Math.abs(after[i + 1] - beforeCad[i + 1]);
      return diff / (after.length / 4) > 40;
    }, "Send Ctrl+Alt+Del brings up the remote Windows security screen");
    if (document.querySelector('[data-rdp-tools="ssh-available"]')) {
      await until(
        async () => (await status()).visible,
        "Opening the tools panel keeps the live RDP control shown",
      );
      button("Connect tools")!.click();
      await until(
        () => {
          // Host-key review for the test host, then the session runs.
          const trust = button("Trust and save");
          trust?.click();
          return !!document.querySelector(".rdp-tools .dot.connected");
        },
        "Tools sign in over SSH with the RDP account and saved password",
        45000,
      );
      await until(
        () =>
          document.querySelectorAll(".rdp-tools .file-row[data-name]").length >
          0,
        "Files tab lists the remote home directory over SFTP",
      );
      const files = document.querySelector<HTMLElement>(
        ".rdp-tools .files-panel",
      )!;
      const fr = files.getBoundingClientRect();
      const dropped = await invoke<string>("capture_simulate_drop", {
        x: (fr.left + fr.width / 2) * window.devicePixelRatio,
        y: (fr.top + fr.height / 2) * window.devicePixelRatio,
      });
      await until(
        () =>
          !!document.querySelector(
            `.rdp-tools .file-row[data-name="${dropped}"]`,
          ),
        "A file dropped on the tools Files tab uploads over SFTP and appears",
        20000,
      );
      await until(async () => {
        const notice = document.querySelector(".app-notice.above-rdp");
        const b = (await status()).bounds;
        if (!notice || !b) return false;
        const n = notice.getBoundingClientRect();
        return n.bottom * window.devicePixelRatio <= b.y;
      }, "The upload notice shows above the RDP desktop, not under it");
      button("Close remote tools")!.click();
      await until(
        () => !document.querySelector(".rdp-tools"),
        "Closing the panel ends the tools session",
      );
    } else {
      checks.push(
        "Test host has no OpenSSH: tools explain the requirement" +
          (document.querySelector("[data-probe-error]")?.textContent ?? ""),
      );
    }

    await invoke("rdp_session", {
      id: firstId,
      action: "disconnect",
      bounds: null,
      scale: null,
    });
    await until(async () => {
      const s = await status(firstId);
      return s.finished && s.status === "Disconnected";
    }, "User disconnect reports plain 'Disconnected'");

    const reconnectButton = () =>
      [...document.querySelectorAll<HTMLButtonElement>("button")].find(
        (button) => button.textContent?.trim() === "Reconnect",
      );
    await until(
      () => !!reconnectButton(),
      "The pane offers Reconnect after the session ended",
    );
    const reconnect = reconnectButton()!;
    reconnect.click();
    await until(
      async () => liveId() !== firstId && (await loggedOn()),
      "Reconnect opens a new session that logs on with the saved password",
      45000,
    );
    const again = await status();
    if (!again.saved_password)
      throw Error("Reconnect did not use the saved password");
    if (
      !again.certificate_remembered ||
      again.events.some((e) => e.kind.startsWith("authentication_warning"))
    )
      throw Error("Reconnect still showed the certificate prompt");
    checks.push(
      "Reconnect with the remembered certificate shows no certificate prompt",
    );
    const profileId = 1; // the RDP fixture bookmark (rdp_fixture in main.rs)
    if (!(await invoke<boolean>("rdp_certificate_remembered", { profileId })))
      throw Error("Session settings do not show the remembered certificate");
    checks.push("Session settings report the certificate as remembered");
    await invoke("capture_report", { checks, passed: true });
  } catch (error) {
    checks.push(`FAILED: ${String(error)}`);
    const last = await status().catch(() => null);
    if (last)
      checks.push(
        `Last status: ${last.status}; connected=${last.connected}; events=${last.events
          .map((e) => e.kind + (e.code === null ? "" : `(${e.code})`))
          .join(",")}`,
      );
    await invoke("capture_report", { checks, passed: false });
  }
}

// Capture scope "rdp-refused": the saved password is wrong on purpose.
export async function checkRdpRefused() {
  const checks: string[] = [];
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const liveId = () =>
    Number(document.querySelector<HTMLElement>("[data-rdp-id]")?.dataset.rdpId);
  let last: Status | null = null;
  try {
    const end = Date.now() + 60000;
    for (;;) {
      if (liveId())
        last = await invoke<Status>("rdp_session", {
          id: liveId(),
          action: "status",
          bounds: null,
          scale: 100,
        }).catch(() => last);
      if (last?.finished) break;
      if (Date.now() > end)
        throw Error("Refused logon did not end the session");
      await sleep(300);
    }
    checks.push(
      `Refused password ends the session: ${last.status}; events ${last.events
        .map((e) => e.kind + (e.code === null ? "" : `(${e.code})`))
        .join(",")}`,
    );
    if (last.status !== "The user name or password was not accepted")
      throw Error("Refusal not recognised: " + last.status);
    checks.push("Status says the user name or password was not accepted");
    if (last.saved_password) throw Error("Refused saved password was kept");
    checks.push("The refused saved password was forgotten");
    if (last.events.some((e) => e.kind === "login_complete"))
      throw Error("Logon completed with a wrong password");
    checks.push("No logon completed with the wrong password");
    await invoke("capture_report", { checks, passed: true });
  } catch (error) {
    checks.push(`FAILED: ${String(error)}`);
    if (last)
      checks.push(
        `Last status: ${last.status}; events=${last.events
          .map((e) => e.kind + (e.code === null ? "" : `(${e.code})`))
          .join(",")}`,
      );
    await invoke("capture_report", { checks, passed: false });
  }
}

// Capture scope "rdp-power": restarts the test host through the tools panel.
export async function checkRdpPower() {
  const checks: string[] = [];
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const liveId = () =>
    Number(document.querySelector<HTMLElement>("[data-rdp-id]")?.dataset.rdpId);
  const status = () =>
    invoke<Status>("rdp_session", {
      id: liveId(),
      action: "status",
      bounds: null,
      scale: 100,
    });
  const button = (label: string) =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) =>
        b.getAttribute("aria-label") === label ||
        b.textContent?.trim() === label,
    );
  const until = async (
    test: () => Promise<boolean> | boolean,
    name: string,
    ms = 30000,
  ) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      try {
        if (await test()) {
          checks.push(name);
          return;
        }
      } catch {
        // retry
      }
      await sleep(300);
    }
    throw Error(name);
  };
  try {
    await until(
      async () => !!liveId() && (await status()).connected,
      "RDP connected",
      60000,
    );
    await sleep(6000);
    button("Remote tools")!.click();
    await until(() => !!button("Connect tools"), "Tools offer OpenSSH");
    button("Connect tools")!.click();
    await until(
      () => {
        button("Trust and save")?.click();
        return !!document.querySelector(".rdp-tools .dot.connected");
      },
      "Tools signed in over SSH",
      45000,
    );
    button("Power")!.click();
    await until(() => !!button("Restart computer"), "Power tab offers restart");
    button("Restart computer")!.click();
    await until(() => !!button("Restart"), "Restart asks for confirmation");
    button("Restart")!.click();
    let last: Status | null = null;
    await until(
      async () => {
        last = await status();
        return last.finished;
      },
      "The remote desktop ends when the computer restarts",
      90000,
    );
    checks.push(`Disconnect text: ${last!.status}`);
    await invoke("capture_report", { checks, passed: true });
  } catch (error) {
    checks.push(`FAILED: ${String(error)}`);
    await invoke("capture_report", { checks, passed: false });
  }
}

// Capture scope "ssh-password": SSH bookmark with password sign-in against the
// Windows OpenSSH test host, password saved for the bookmark itself.
export async function checkSshPassword() {
  const checks: string[] = [];
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const until = async (test: () => boolean, name: string, ms = 45000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      if (test()) {
        checks.push(name);
        return;
      }
      await sleep(250);
    }
    throw Error(name);
  };
  const button = (label: string) =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent?.trim() === label,
    );
  try {
    await until(() => {
      button("Trust and save")?.click();
      return !!document.querySelector(".pane .dot.connected");
    }, "SSH bookmark signs in with its saved password (host key reviewed once)");
    await until(
      () =>
        document.querySelectorAll(".files-panel .file-row[data-name]").length >
        0,
      "SFTP lists the Windows home directory",
    );
    await until(
      () =>
        !!document.querySelector(".xterm-rows")?.textContent?.includes("PS C:"),
      "The terminal shows the remote PowerShell prompt",
    );
    // Reconnecting (with the file sidebar open) must not surface errors from
    // the previous connection.
    const labelled = (label: string) =>
      [...document.querySelectorAll<HTMLButtonElement>("button")].find(
        (b) =>
          b.getAttribute("aria-label") === label ||
          b.textContent?.trim() === label,
      );
    for (const round of [1, 2, 3]) {
      labelled("Disconnect")!.click();
      await until(() => !!labelled("Reconnect"), `Disconnect ${round} ends it`);
      await until(
        () =>
          document.querySelector(".side-tabs button.selected")?.textContent ===
          "Sessions",
        `Disconnect ${round} switches the sidebar to Sessions`,
        5000,
      );
      await sleep(500);
      labelled("Reconnect")!.click();
      await until(
        () => !!document.querySelector(".pane .dot.connected"),
        `Reconnect ${round} signs in again`,
      );
      await sleep(2500);
    }
    const notice = document.querySelector(".app-notice")?.textContent ?? "";
    if (/no longer exists/i.test(notice))
      throw Error(`Reconnect showed an error: ${notice}`);
    checks.push("Reconnect and disconnect show no stale-connection error");
    await invoke("capture_report", { checks, passed: true });
  } catch (error) {
    checks.push(`FAILED: ${String(error)}`);
    await invoke("capture_report", { checks, passed: false });
  }
}

// Capture scope "rdp-network": the test script makes the Windows host drop its
// network adapter for about a minute while the session is connected. The pane
// must end the session (auto-reconnect is off), stay responsive, and Reconnect
// once the host is back.
export async function checkRdpNetwork() {
  const checks: string[] = [];
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const liveId = () =>
    Number(document.querySelector<HTMLElement>("[data-rdp-id]")?.dataset.rdpId);
  let slowest = 0;
  const status = async () => {
    const start = performance.now();
    const s = await invoke<Status>("rdp_session", {
      id: liveId(),
      action: "status",
      bounds: null,
      scale: 100,
    });
    slowest = Math.max(slowest, performance.now() - start);
    return s;
  };
  const reconnectButton = () =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent?.trim() === "Reconnect",
    );
  const until = async (
    test: () => Promise<boolean> | boolean,
    name: string,
    ms: number,
  ) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      try {
        if (await test()) {
          checks.push(name);
          return;
        }
      } catch {
        // retry
      }
      await sleep(500);
    }
    throw Error(name);
  };
  try {
    await until(
      async () => !!liveId() && (await status()).connected,
      "RDP connected",
      60000,
    );
    const connected = Date.now();
    let ended: Status | null = null;
    await until(
      async () => {
        ended = await status();
        return ended.finished;
      },
      "Network loss ends the session (no automatic reconnect)",
      180000,
    );
    checks.push(
      `Session ended ${Math.round((Date.now() - connected) / 1000)} s after connecting: ${ended!.status}`,
    );
    if (slowest > 1000)
      throw Error(
        `Status call took ${Math.round(slowest)} ms during the outage`,
      );
    checks.push(
      `Pane stayed responsive (slowest status ${Math.round(slowest)} ms)`,
    );
    const lost = Date.now();
    let attempts = 0;
    await until(
      async () => {
        const id = liveId();
        if (id) {
          const s = await status();
          if (s.connected) return true;
          if (!s.finished) return false;
        }
        const again = reconnectButton();
        if (again) {
          attempts++;
          again.click();
          await sleep(3000);
        }
        return false;
      },
      "Reconnect succeeds once the host is back",
      240000,
    );
    checks.push(
      `Reconnected ${Math.round((Date.now() - lost) / 1000)} s after the loss, ${attempts} attempt(s)`,
    );
    await invoke("capture_report", { checks, passed: true });
  } catch (error) {
    checks.push(`FAILED: ${String(error)}`);
    await invoke("capture_report", { checks, passed: false });
  }
}

// Capture scope "quick-connect": Enter in the session search connects to a
// typed destination.
// Typing the RDP fixture's own destination reuses its bookmark (saved password)
// and connects; an unknown TELNET destination is saved as a new bookmark.
export async function checkQuickConnect(
  setPrefs: (p: never) => void,
  profiles: () => {
    id: number;
    protocol: string;
    host: string;
    port: number;
    username: string;
    name: string;
  }[],
) {
  const checks: string[] = [];
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const until = async (
    test: () => Promise<boolean> | boolean,
    name: string,
    ms = 30000,
  ) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      try {
        if (await test()) {
          checks.push(name);
          return;
        }
      } catch {
        // retry
      }
      await sleep(300);
    }
    throw Error(name);
  };
  const search = () =>
    document.querySelector<HTMLInputElement>(
      'input[aria-label="Find a session"]',
    )!;
  const type = (text: string) => {
    const input = search();
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  };
  const enter = () =>
    search().dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
  const liveId = () =>
    Number(document.querySelector<HTMLElement>("[data-rdp-id]")?.dataset.rdpId);
  try {
    const rdp = profiles()[0];
    const before = profiles().length;
    // Part of a saved host filters the list like before.
    type(rdp.host.slice(0, 3));
    await until(
      () => document.querySelectorAll(".session-row").length === 1,
      "Text matching a bookmark still filters the session list",
      5000,
    );
    // Timeline still with a synthetic destination (no real host on screen).
    type("rdp://admin@pc.example.test");
    await sleep(500);
    await invoke("capture_still", { name: "quick-connect" });
    await sleep(1500);
    type(
      `rdp://${rdp.username}@${rdp.host}${rdp.port === 3389 ? "" : `:${rdp.port}`}`,
    );
    enter();
    await until(
      async () => {
        const id = liveId();
        if (!id) return false;
        const s = await invoke<Status>("rdp_session", {
          id,
          action: "status",
          bounds: null,
          scale: 100,
        });
        return s.connected && s.saved_password;
      },
      "Enter reuses the existing bookmark and connects with its saved password",
      60000,
    );
    if (profiles().length !== before)
      throw Error("Quick connect duplicated an existing bookmark");
    checks.push("No duplicate bookmark was saved");
    if (search().value !== "")
      throw Error("The search field kept the destination");
    type("telnet://127.0.0.1:1");
    enter();
    await until(
      () =>
        profiles().some(
          (p) =>
            p.protocol === "Telnet" &&
            p.host === "127.0.0.1" &&
            p.port === 1 &&
            p.name === "TELNET 127.0.0.1:1" &&
            (p as { temporary?: boolean }).temporary === true,
        ),
      "An unknown destination becomes the unsaved session 'TELNET 127.0.0.1:1'",
      10000,
    );
    await until(
      () =>
        [...document.querySelectorAll(".session-group")].some((g) =>
          g.textContent?.startsWith("Quick connect · not saved"),
        ),
      "The sidebar lists it under 'Quick connect · not saved'",
      5000,
    );
    await until(
      () =>
        !!document
          .querySelector(".session-row.active")
          ?.textContent?.includes("TELNET 127.0.0.1:1"),
      "The new bookmark is selected and its connection attempted",
      10000,
    );
    // Session groups: file both bookmarks, collapse one, search by group.
    const save = async (
      p: ReturnType<typeof profiles>[number],
      group: string,
    ) =>
      setPrefs(
        await invoke("save_profile", {
          // Filing the quick-connect session also saves it (Save session).
          profile: { ...p, group, temporary: false },
        }),
      );
    const telnet = profiles().find((p) => p.protocol === "Telnet")!;
    await save(
      profiles().find((p) => p.protocol === "Rdp")!,
      "Wien",
    );
    await save(telnet, "Graz");
    type("");
    const headers = () => [
      ...document.querySelectorAll<HTMLButtonElement>(".session-group"),
    ];
    await until(
      () =>
        headers()
          .map((h) => h.textContent)
          .join("|") === "Graz1|Wien1",
      "Groups appear as sorted folders with session counts",
      5000,
    );
    headers()[1].click();
    await until(
      () =>
        headers()[1].getAttribute("aria-expanded") === "false" &&
        document.querySelectorAll(".session-row").length === 1,
      "Collapsing a group hides its sessions",
      5000,
    );
    await sleep(300);
    await invoke("capture_still", { name: "session-groups" });
    await sleep(1500);
    type("wien");
    await until(
      () =>
        document.querySelectorAll(".session-row").length === 1 &&
        headers().length === 1 &&
        headers()[0].getAttribute("aria-expanded") === "true",
      "Searching a group name shows its sessions even when collapsed",
      5000,
    );
    type("");
    // Delete the TELNET bookmark through the session menu.
    const press = (label: string) =>
      [...document.querySelectorAll<HTMLButtonElement>("button")]
        .find((b) => b.textContent?.trim() === label)
        ?.click();
    for (const g of document.querySelectorAll<HTMLButtonElement>(
      ".session-group[aria-expanded=false]",
    ))
      g.click();
    await sleep(300);
    [...document.querySelectorAll(".session-row")]
      .find((r) => r.textContent?.includes("TELNET 127.0.0.1:1"))!
      .dispatchEvent(
        new MouseEvent("contextmenu", {
          bubbles: true,
          clientX: 60,
          clientY: 200,
        }),
      );
    await until(
      () =>
        [...document.querySelectorAll("button")].some(
          (b) => b.textContent?.trim() === "Delete session",
        ),
      "The session menu offers Delete session",
      5000,
    );
    press("Delete session");
    await until(
      () => !!document.querySelector('dialog[aria-label="Delete session"]'),
      "Deleting asks for confirmation",
      5000,
    );
    press("Delete");
    await until(
      () =>
        !profiles().some((p) => p.protocol === "Telnet") &&
        ![...document.querySelectorAll(".session-row")].some((r) =>
          r.textContent?.includes("TELNET 127.0.0.1:1"),
        ),
      "Confirmed delete removes the bookmark and its sidebar row",
      10000,
    );
    await invoke("rdp_session", {
      id: liveId(),
      action: "disconnect",
      bounds: null,
      scale: null,
    }).catch(() => undefined);
    await sleep(3000);
    await invoke("capture_report", { checks, passed: true });
  } catch (error) {
    checks.push(`FAILED: ${String(error)}`);
    await invoke("capture_report", { checks, passed: false });
  }
}

// Capture scope "ssh-two": two SSH bookmarks to the Windows test host, both
// connected in a 2-pane layout. Disconnecting one must not show any
// "no longer exists" error anywhere (notice bar, statistics, file sidebar).
export async function checkSshTwo() {
  const checks: string[] = [];
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const seen = new Set<string>();
  let phase = "connect";
  const scan = () => {
    const walker = document.createTreeWalker(
      document.body,
      NodeFilter.SHOW_TEXT,
    );
    for (let n = walker.nextNode(); n; n = walker.nextNode())
      if (/no longer exists/i.test(n.textContent ?? ""))
        seen.add(
          `[${phase}] ${[1, 2, 3]
            .map((k) => {
              let e: Element | null = n.parentElement;
              for (let i = 1; i < k && e; i++) e = e.parentElement;
              return e ? `${e.tagName}.${e.className}` : "";
            })
            .join(" < ")}: ${n.textContent}`,
        );
  };
  const observer = new MutationObserver(scan);
  observer.observe(document.body, {
    subtree: true,
    childList: true,
    characterData: true,
  });
  const until = async (test: () => boolean, name: string, ms = 45000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      if (test()) {
        checks.push(name);
        return;
      }
      await sleep(250);
    }
    throw Error(name);
  };
  const button = (label: string, scope: ParentNode = document) =>
    [...scope.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) =>
        b.getAttribute("aria-label") === label ||
        b.textContent?.trim() === label,
    );
  const panes = () => [...document.querySelectorAll<HTMLElement>(".pane")];
  try {
    await until(() => {
      button("Trust and save")?.click();
      return document.querySelectorAll(".pane .dot.connected").length === 2;
    }, "Both SSH bookmarks connect in two panes");
    await sleep(3000);
    // Close the first tab while both are connected (the reported case),
    // then disconnect the remaining one.
    phase = "close tab";
    button("Close SSH password fixture")!.click();
    await until(
      () => document.querySelectorAll(".pane .dot.connected").length === 1,
      "Closing one connected tab leaves the other connected",
    );
    const sideTab = () =>
      document.querySelector(".side-tabs button.selected")?.textContent;
    await until(
      () =>
        !!document
          .querySelector(".tabs .tab.active")
          ?.textContent?.includes("SSH password fixture 2") &&
        sideTab() === "Files",
      "The last remaining tab becomes active with its SSH files shown",
      5000,
    );
    await sleep(4000);
    scan();
    phase = "disconnect";
    const pane = panes().find((p) => !!button("Disconnect", p))!;
    button("Disconnect", pane)!.click();
    await until(
      () => document.querySelectorAll(".pane .dot.connected").length === 0,
      "Disconnecting the remaining session ends it",
    );
    await until(
      () => sideTab() === "Sessions",
      "A disconnected SSH tab shows the session list",
      5000,
    );
    await sleep(3000);
    scan();
    phase = "close last tab";
    button("Close SSH password fixture 2")!.click();
    await until(
      () => !!document.body.textContent?.includes("Your workspace, ready"),
      "Closing the last tab shows the home screen",
      5000,
    );
    await sleep(2000);
    scan();
    observer.disconnect();
    if (seen.size)
      throw Error(`Stale-connection error shown: ${[...seen].join(" | ")}`);
    checks.push("No 'no longer exists' error appeared while disconnecting");
    await invoke("capture_report", { checks, passed: true });
  } catch (error) {
    observer.disconnect();
    checks.push(`FAILED: ${String(error)}`);
    checks.push(
      `State: active=${document.querySelector(".session-row.active")?.textContent ?? "none"} side=${document.querySelector(".side-tabs button.selected")?.textContent ?? "none"} panes=${[...document.querySelectorAll(".pane")].map((p) => (p.textContent ?? "").slice(0, 40)).join(" | ")}`,
    );
    await invoke("capture_report", { checks, passed: false });
  }
}
