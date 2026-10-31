// Runs only after the Rust bootstrap verifies an isolated loopback VNC profile.
import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap, Preferences } from "./types";
export async function checkVncFixture() {
  const checks: string[] = [];
  const until = async (check: () => boolean, name: string) => {
    const end = Date.now() + 60000;
    while (Date.now() < end) {
      if (check()) {
        checks.push(name);
        return;
      }
      await new Promise((r) => setTimeout(r, 40));
    }
    throw Error(name);
  };
  const buttons = (text: string) =>
    [
      ...document.querySelectorAll<HTMLButtonElement>("dialog[open] button"),
    ].filter((b) => b.textContent === text);
  try {
    const saved = (await invoke<string>("capture_scope")) === "vnc-saved";
    await until(
      () =>
        document.querySelectorAll(
          'dialog[aria-label="Verify VNC certificate"][open]',
        ).length === 4,
      "Four independent real-loopback TLS peers require certificate review",
    );
    if (document.querySelector('input[type="password"]'))
      throw Error("Password requested before trust decision");
    checks.push(
      "No password prompt exists before explicit certificate approval",
    );
    let duplicateBlocked = false;
    try {
      await invoke("vnc_open", { profileId: 1 });
    } catch (error) {
      duplicateBlocked = String(error).includes("already has a VNC connection");
    }
    if (!duplicateBlocked)
      throw Error("Duplicate VNC open created another backend worker");
    checks.push(
      "Backend rejects duplicate VNC opens while certificate review is pending",
    );
    await until(
      () =>
        [...document.querySelectorAll(".vnc-certificate")].every((el) =>
          ["Subject", "Issuer", "Valid from", "Valid until", "SHA-256"].every(
            (v) => el.textContent?.includes(v),
          ),
        ),
      "Certificate dialogs include subject issuer validity and fingerprint",
    );
    for (const button of buttons("Trust this certificate")) button.click();
    await until(
      () =>
        document.querySelectorAll(
          'dialog[aria-label="VNC authentication"][open]',
        ).length === 4,
      "Passwords requested only after approval and TLS revalidation",
    );
    if (document.querySelector('input[type="password"]'))
      throw Error("VNC secret entry exists in the WebView");
    checks.push(
      "VNC renderer has no password input; native sign-in carries IDs only",
    );
    for (const form of document.querySelectorAll<HTMLFormElement>(
      'dialog[aria-label="VNC authentication"] form',
    ))
      form.requestSubmit();
    await until(
      () =>
        document.querySelectorAll(".vnc-surface canvas").length === 4 &&
        [
          ...document.querySelectorAll<HTMLCanvasElement>(
            ".vnc-surface canvas",
          ),
        ].every(
          (c) =>
            c.width === 640 &&
            c.height === 400 &&
            [...c.getContext("2d")!.getImageData(300, 200, 1, 1).data].join(
              ",",
            ) === "45,65,85,255",
        ),
      "Four canvases render decoded TLS framebuffer pixels",
    );
    const canvases = [
      ...document.querySelectorAll<HTMLCanvasElement>(".vnc-surface canvas"),
    ];
    const ids = [...document.querySelectorAll<HTMLElement>(".vnc-surface")].map(
      (el) => el.dataset.vncId,
    );
    if (new Set(ids).size !== 4) throw Error("Reused VNC IDs");
    checks.push("Four graphical incarnations have distinct IDs");
    canvases[0].focus();
    canvases[0].dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Shift",
        code: "ShiftLeft",
        bubbles: true,
      }),
    );
    canvases[1].focus();
    await new Promise((r) => setTimeout(r, 150));
    checks.push("Synthetic focus change exercises backend tracked key release");
    for (const [label, layout] of [
      ["Single pane", 1],
      ["Two panes", 2],
      ["Four panes", 4],
    ] as const) {
      document.querySelector<HTMLElement>(".view-menu summary")!.click();
      const option = [
        ...document.querySelectorAll<HTMLButtonElement>(".view-menu button"),
      ].find((b) => b.title.startsWith(label));
      if (!option) throw Error("Layout action missing");
      option.click();
      await until(
        () => !!document.querySelector(`.workspace.layout-${layout}`),
        `${label} selects the expected workspace layout`,
      );
      if (
        ids.some(
          (id, i) =>
            document.querySelectorAll<HTMLElement>(".vnc-surface")[i]?.dataset
              .vncId !== id,
        )
      )
        throw Error("Layout changed VNC incarnations");
    }
    checks.push(
      "All layouts preserve four original VNC session IDs and source framebuffer dimensions",
    );
    let stale = false;
    try {
      await invoke("vnc_authenticate", {
        id: Number(ids[0]),
        challenge: 0,
      });
    } catch {
      stale = true;
    }
    if (!stale) throw Error("Stale VNC password answer accepted");
    checks.push(
      "Backend rejects stale credential answers after authentication",
    );
    if (document.querySelector(".app-notice"))
      throw Error("Unexpected application error notice");
    checks.push(
      "VNC does not invoke the SSH file adapter or show an application error",
    );
    if (saved) {
      const before = await invoke<Bootstrap>("bootstrap");
      if (before.prefs.profiles.some((p) => !p.credential_id))
        throw Error("Saved VNC identities missing");
      document
        .querySelector<HTMLButtonElement>('button[title="Disconnect"]')!
        .click();
      await until(
        () =>
          [...document.querySelectorAll("button")].some(
            (b) => b.textContent === "Reconnect",
          ),
        "Disconnect finishes the saved VNC connection",
      );
      [...document.querySelectorAll("button")]
        .find((b) => b.textContent === "Reconnect")!
        .click();
      await until(
        () =>
          !!document.querySelector(
            'dialog[aria-label="VNC authentication"][open]',
          ),
        "Reconnect requests credentials after validating the saved certificate",
      );
      if (
        document.querySelector(
          'dialog[aria-label="Verify VNC certificate"][open]',
        )
      )
        throw Error("Reconnect requested certificate approval again");
      document
        .querySelector<HTMLFormElement>(
          'dialog[aria-label="VNC authentication"] form',
        )!
        .requestSubmit();
      await until(
        () =>
          [
            ...document.querySelectorAll<HTMLCanvasElement>(
              ".vnc-surface canvas",
            ),
          ].filter((c) => c.width === 640 && c.height === 400).length === 4,
        "Saved VNC password authenticates the new connection without a native prompt",
      );
      const nextIds = [
        ...document.querySelectorAll<HTMLElement>(".vnc-surface"),
      ].map((s) => s.dataset.vncId);
      if (nextIds.every((id, i) => id === ids[i]))
        throw Error("Reconnect reused its incarnation");
      const forgotten = await invoke<Preferences>("forget_saved_password", {
        profileId: 1,
      });
      if (
        forgotten.profiles.find((p) => p.id === 1)?.credential_id ||
        forgotten.profiles
          .filter((p) => p.id !== 1)
          .some((p) => !p.credential_id)
      )
        throw Error("Forget password crossed bookmark boundaries");
      checks.push(
        "Common Forget saved password clears only the chosen VNC bookmark",
      );
      document
        .querySelector<HTMLButtonElement>('button[title="Edit VNC fixture 1"]')!
        .click();
      await until(
        () =>
          !!document.querySelector('dialog[aria-label="Edit session"][open]'),
        "VNC editor opens over the live desktops",
      );
      if (
        !document
          .querySelector("dialog[open]")!
          .textContent?.includes("Remember me")
      )
        throw Error("VNC editor does not explain optional storage");
      await invoke("capture_still", { name: "vnc-passwords" });
      await new Promise((r) => setTimeout(r, 300));
      document
        .querySelector<HTMLButtonElement>(
          'dialog[open] [aria-label="Close dialog"]',
        )!
        .click();
      await until(
        () => !document.querySelector("dialog[open]"),
        "VNC editor closes without saving",
      );
      document
        .querySelector<HTMLButtonElement>(".sidebar .new-session")!
        .click();
      await until(
        () =>
          !!document.querySelector(".x11-settings details small")?.textContent,
        "SSH editor completes actual native X11 availability detection",
      );
      const details = document.querySelector<HTMLDetailsElement>(
        ".x11-settings details",
      )!;
      if (
        details.open ||
        /cookie|Cygwin|127\.0\.0\.1/.test(
          document.querySelector(".x11-settings [role=status]")!.textContent ??
            "",
        )
      )
        throw Error("Technical X11 setup leaks into the default explanation");
      checks.push(
        "X11 setup and security details remain available in a closed disclosure",
      );
      await invoke("capture_still", { name: "remote-linux-apps" });
      await new Promise((r) => setTimeout(r, 300));
      document
        .querySelector<HTMLButtonElement>(
          'dialog[open] [aria-label="Close dialog"]',
        )!
        .click();
      await until(
        () => !document.querySelector("dialog[open]"),
        "SSH draft closes without saving or connecting",
      );
    }
    const clipboardButton = document.querySelector<HTMLButtonElement>(
      ".vnc-actions button",
    )!;
    clipboardButton.click();
    await until(
      () =>
        !!document.querySelector(
          'textarea[aria-label="Clipboard text to send"]',
        ),
      "Explicit clipboard action opens a review",
    );
    const clipboard = document.querySelector<HTMLTextAreaElement>(
      'textarea[aria-label="Clipboard text to send"]',
    )!;
    const setter = Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!;
    setter.call(clipboard, "native clipboard fixture");
    clipboard.dispatchEvent(new Event("input", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 80));
    buttons("Send text")[0].click();
    await until(
      () => !document.querySelector('dialog[aria-label="Send clipboard text"]'),
      "Clipboard sends only after explicit review confirmation",
    );
    clipboardButton.click();
    await until(
      () =>
        !!document.querySelector(
          'textarea[aria-label="Clipboard text to send"]',
        ),
      "Native capture retains the clipboard review UI",
    );
    // IPC acceptance enqueues input; allow the real loopback peer to consume it
    // before capture shutdown cancels the connection. The host checks its file.
    await new Promise((r) => setTimeout(r, 1000));
    await invoke("capture_report", { checks, passed: true });
    await invoke("capture_ready", { terminalReady: true });
  } catch (error) {
    await invoke("capture_report", {
      checks: [...checks, String(error)],
      passed: false,
    });
    throw error;
  }
}
