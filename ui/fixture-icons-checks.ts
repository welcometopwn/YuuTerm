import { invoke } from "@tauri-apps/api/core";
import type { Bootstrap } from "./types";
export async function checkIcons(
  ids: number[],
  until: (f: () => boolean | Promise<boolean>, name: string) => Promise<void>,
  checks: string[],
) {
  const button = (text: string) =>
    Array.from(
      document.querySelectorAll<HTMLButtonElement>("dialog button"),
    ).find((b) => b.textContent?.trim() === text)!;
  const open = async () => {
    Array.from(
      document.querySelectorAll<HTMLButtonElement>(".side-tabs button"),
    )
      .find((b) => b.textContent?.trim() === "Sessions")!
      .click();
    await until(
      () => !!document.querySelector('[aria-label="Edit SSH fixture 1"]'),
      "Session editor action is available",
    );
    document
      .querySelector<HTMLButtonElement>('[aria-label="Edit SSH fixture 1"]')!
      .click();
    await until(
      () => !!document.querySelector('[aria-label="Session PNG icon"]'),
      "PNG icon picker opens in Connection settings",
    );
  };
  const canvas = document.createElement("canvas");
  canvas.width = 64;
  canvas.height = 64;
  const ctx = canvas.getContext("2d")!;
  ctx.fillStyle = "#202125";
  ctx.fillRect(0, 0, 64, 64);
  ctx.strokeStyle = "#89b4fa";
  ctx.lineWidth = 5;
  ctx.beginPath();
  ctx.moveTo(16, 18);
  ctx.lineTo(30, 32);
  ctx.lineTo(16, 46);
  ctx.moveTo(35, 46);
  ctx.lineTo(50, 46);
  ctx.stroke();
  const blob = await new Promise<Blob>((r) =>
    canvas.toBlob((b) => r(b!), "image/png"),
  );
  const choose = (file: File) => {
    const transfer = new DataTransfer();
    transfer.items.add(file);
    const input = document.querySelector<HTMLInputElement>(
      '[aria-label="Session PNG icon"]',
    )!;
    input.files = transfer.files;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  };
  const preview = () =>
    document.querySelector<HTMLImageElement>(".icon-preview img");
  await open();
  choose(new File([blob], "fixture.png", { type: "image/png" }));
  await until(
    () => !!preview() && !button("Save session").disabled,
    "PNG is decoded and resized into a draft preview",
  );
  if ((await invoke<Bootstrap>("bootstrap")).prefs.profiles[0].icon)
    throw new Error("Icon draft was persisted before Save");
  checks.push("Icon selection is draft-only until Save");
  button("Cancel").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Cancel discards the icon draft",
  );
  await open();
  const slow = new File([blob], "slow-fixture.png", { type: "image/png" });
  Object.defineProperty(slow, "arrayBuffer", {
    value: async () => {
      await new Promise((r) => setTimeout(r, 180));
      return blob.arrayBuffer();
    },
  });
  choose(slow);
  await until(
    () => button("Save session").disabled,
    "Save waits while an icon is being read",
  );
  document
    .querySelector<HTMLButtonElement>('dialog [aria-label="Close dialog"]')!
    .click();
  await new Promise((r) => setTimeout(r, 350));
  if (
    document.querySelector("dialog[open]") ||
    (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0].icon
  )
    throw new Error("Late icon completion reopened or saved a closed draft");
  checks.push(
    "Closing during a delayed read cannot reopen the editor or save an icon",
  );
  await open();
  choose(new File(["<svg/>"], "renamed.png", { type: "image/png" }));
  await until(
    () => !!document.querySelector(".icon-problem"),
    "SVG disguised as PNG is refused before decoding",
  );
  choose(new File([blob], "fixture.png", { type: "image/png" }));
  await until(
    () => !!preview() && !button("Save session").disabled,
    "Valid PNG clears the rejected-image error",
  );
  button("Save session").click();
  await until(
    async () =>
      !document.querySelector("dialog[open]") &&
      !!(await invoke<Bootstrap>("bootstrap")).prefs.profiles[0].icon,
    "Save persists the normalized PNG",
  );
  const saved = (await invoke<Bootstrap>("bootstrap")).prefs;
  const icon = saved.profiles[0].icon!;
  const bytes = Uint8Array.from(atob(icon.split(",")[1]), (c) =>
    c.charCodeAt(0),
  );
  const data = new DataView(bytes.buffer);
  if (data.getUint32(16) !== 32 || data.getUint32(20) !== 32)
    throw new Error("Icon was not normalized to 32 pixels");
  checks.push("Native canvas normalization produces a portable 32×32 PNG");
  await until(
    () =>
      document.querySelectorAll(
        ".session-list img.session-icon, .tabs img.session-icon",
      ).length === 2,
    "Saved icon appears in the sidebar and open tab",
  );
  const invalid = structuredClone(saved.profiles[0]);
  invalid.icon = "data:image/svg+xml,<svg/>";
  let rejected = false;
  try {
    await invoke("save_profile", { profile: invalid });
  } catch {
    rejected = true;
  }
  if (
    !rejected ||
    (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0].icon !== icon
  )
    throw new Error(
      "Backend accepted unsafe icon metadata or lost the valid icon",
    );
  checks.push(
    "Backend refuses unsafe icon IPC and preserves the previously saved image",
  );
  await open();
  button("Reset icon").click();
  await until(() => !preview(), "Reset changes only the icon draft");
  button("Cancel").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Cancelling Reset leaves the saved icon unchanged",
  );
  if ((await invoke<Bootstrap>("bootstrap")).prefs.profiles[0].icon !== icon)
    throw new Error("Cancelled Reset removed the icon");
  await open();
  button("Reset icon").click();
  await until(() => !preview(), "Reset draft is applied before Save");
  button("Save session").click();
  await until(
    async () =>
      !document.querySelector("dialog[open]") &&
      !(await invoke<Bootstrap>("bootstrap")).prefs.profiles[0].icon,
    "Saved Reset restores the default icon",
  );
  if (ids.some((id) => !document.querySelector(`[data-live-id="${id}"]`)))
    throw new Error("Icon edits replaced a live SSH session");
  checks.push("Icon edits preserve all four live SSH incarnations");
  await open();
  choose(new File([blob], "fixture.png", { type: "image/png" }));
  await until(
    () => !!preview() && !button("Save session").disabled,
    "Final icon preview is ready",
  );
  button("Save session").click();
  await until(
    () => !document.querySelector("dialog[open]"),
    "Final icon is saved for the retained native capture",
  );
  await open();
  // Keep the real agent's public identifier out of retained screenshots. This
  // clears only the final unsaved draft; all live sessions retain their pin.
  const fingerprint = document.querySelector<HTMLInputElement>(
    'input[placeholder="SHA256:…"]',
  )!;
  Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )!.set!.call(fingerprint, "");
  fingerprint.dispatchEvent(new Event("input", { bubbles: true }));
  await until(
    () => fingerprint.value === "",
    "Final capture omits the public agent fingerprint in an unsaved draft",
  );
  if (
    (await invoke<Bootstrap>("bootstrap")).prefs.profiles[0].ssh
      .identity_fingerprint !== saved.profiles[0].ssh.identity_fingerprint
  )
    throw new Error("Capture redaction changed the saved identity");
  checks.push(
    "Capture-only redaction preserves the real fixture's stored identity and live connections",
  );
}
