import { invoke } from "@tauri-apps/api/core";
import {
  checkGatewayPresentation,
  redactGatewayLabels,
  showGatewayEditor,
} from "./fixture-gateway-presentation";

export async function checkGatewayRdp() {
  const checks: string[] = [];
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const id = () =>
    Number(document.querySelector<HTMLElement>("[data-rdp-id]")?.dataset.rdpId);
  type Status = {
    id: number;
    connected: boolean;
    finished: boolean;
    certificate_remembered: boolean;
    events: { kind: string }[];
  };
  const status = () =>
    invoke<Status>("rdp_session", { id: id(), action: "status" });
  const until = async (
    test: () => boolean | Promise<boolean>,
    label: string,
  ) => {
    const end = Date.now() + 60000;
    while (Date.now() < end) {
      try {
        if (await test()) {
          checks.push(label);
          return;
        }
      } catch {
        /* Wait for incarnation creation. */
      }
      await sleep(100);
    }
    throw Error(label);
  };
  const button = (label: string) =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) =>
        b.textContent?.trim() === label ||
        b.getAttribute("aria-label") === label,
    )!;
  try {
    await until(
      async () =>
        !!id() &&
        (await status()).events.some((e) => e.kind === "login_complete"),
      "Real Windows logon completes through the SSH gateway",
    );
    await checkGatewayPresentation(checks);
    const first = id();
    const colorCount = async () => {
      const jpeg = await invoke<ArrayBuffer>("rdp_snapshot", { id: first });
      if (jpeg.byteLength < 2000) return 0;
      const bitmap = await createImageBitmap(
        new Blob([jpeg], { type: "image/jpeg" }),
      );
      const canvas = new OffscreenCanvas(32, 24),
        context = canvas.getContext("2d")!;
      context.drawImage(bitmap, 0, 0, 32, 24);
      const pixels = context.getImageData(0, 0, 32, 24).data;
      const colors = new Set<string>();
      for (let at = 0; at < pixels.length; at += 4)
        colors.add(Array.from(pixels.slice(at, at + 3)).join(","));
      return colors.size;
    };
    // The desktop is painted some time after login_complete.
    await until(
      async () => (await colorCount()) >= 4,
      "Actual native RDP framebuffer snapshot contains varied desktop pixels",
    );
    if (!(await status()).certificate_remembered)
      throw Error("Original target certificate was not remembered");
    button("Disconnect").click();
    await until(
      async () => (await status()).finished,
      "Disconnect ends the first gateway RDP incarnation",
    );
    await until(() => !!button("Reconnect"), "The pane offers Reconnect");
    button("Reconnect").click();
    await until(
      async () =>
        id() !== first &&
        (await status()).events.some((e) => e.kind === "login_complete"),
      "Reconnect uses a fresh gateway chain and reaches real Windows logon",
    );
    const second = await status();
    if (
      !second.certificate_remembered ||
      second.events.some((e) => e.kind === "authentication_warning")
    )
      throw Error(
        "Remembered target certificate did not compose with the relay",
      );
    checks.push(
      "Remembered original target certificate is enforced by the relay on reconnect",
    );
    button("Disconnect").click();
    await until(
      async () => (await status()).finished,
      "Second gateway RDP incarnation disconnects cleanly",
    );
    document
      .querySelector<HTMLButtonElement>(".session-row button.edit")!
      .click();
    await until(
      () => !!document.querySelector(".gateway-settings .gateway-path"),
      "RDP editor shows the gateway list and connection path",
    );
    document
      .querySelector(".gateway-settings")!
      .scrollIntoView({ block: "center" });
    await redactGatewayLabels();
    await invoke("capture_still", { name: "gateway-rdp-editor" });
    await sleep(300);
    await invoke("capture_report", { checks, passed: true });
  } catch (error) {
    // Keep a native editor capture even when the real transport qualification
    // fails. The image is a UI artifact only and does not imply RDP works.
    try {
      await checkGatewayPresentation(checks);
      await showGatewayEditor(checks);
      await redactGatewayLabels();
      await invoke("capture_still", { name: "gateway-rdp-editor" });
      await sleep(500);
    } catch {
      /* A failed protocol fixture must still report its original failure. */
    }
    await invoke("capture_report", {
      checks: [...checks, String(error)],
      passed: false,
    });
  }
}
