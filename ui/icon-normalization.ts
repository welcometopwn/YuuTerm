// Input limits; accepted images are downscaled to a compact 32×32 PNG.
export const MAX_ICON_BYTES = 4 * 1024 * 1024;
export const MAX_ICON_PIXELS = 512;
const ICON_SIZE = 32;
const svgElements = new Set([
  "svg",
  "g",
  "path",
  "rect",
  "circle",
  "ellipse",
  "line",
  "polyline",
  "polygon",
]);
const svgAttributes = new Set([
  "viewBox",
  "width",
  "height",
  "x",
  "y",
  "x1",
  "x2",
  "y1",
  "y2",
  "cx",
  "cy",
  "r",
  "rx",
  "ry",
  "d",
  "points",
  "fill",
  "fill-rule",
  "fill-opacity",
  "stroke",
  "stroke-width",
  "stroke-linecap",
  "stroke-linejoin",
  "stroke-opacity",
  "stroke-dasharray",
  "stroke-dashoffset",
  "opacity",
  "transform",
  "preserveAspectRatio",
]);

function sanitizedSvg(bytes: Uint8Array): Blob {
  const source = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  if (/<!DOCTYPE|<!ENTITY/i.test(source))
    throw new Error("SVG declarations are unsupported.");
  const doc = new DOMParser().parseFromString(source, "image/svg+xml");
  const root = doc.documentElement;
  if (
    doc.querySelector("parsererror") ||
    root.localName !== "svg" ||
    root.namespaceURI !== "http://www.w3.org/2000/svg"
  )
    throw new Error("Invalid SVG icon.");
  const walker = doc.createTreeWalker(root, NodeFilter.SHOW_ALL);
  while (walker.nextNode()) {
    if (
      walker.currentNode.nodeType !== Node.ELEMENT_NODE &&
      walker.currentNode.nodeType !== Node.TEXT_NODE &&
      walker.currentNode.nodeType !== Node.COMMENT_NODE
    )
      throw new Error("SVG processing instructions are unsupported.");
  }
  const nodes = Array.from(root.querySelectorAll("*"));
  if (nodes.length > 512) throw new Error("SVG icon is too complex.");
  for (const el of [root, ...nodes]) {
    if (el.namespaceURI !== root.namespaceURI || !svgElements.has(el.localName))
      throw new Error(
        "SVG supports basic shapes only; scripts, images, text and references are unsupported.",
      );
    for (const attr of Array.from(el.attributes)) {
      if (
        el === root &&
        attr.name === "xmlns" &&
        attr.value === root.namespaceURI
      )
        continue;
      if (
        !svgAttributes.has(attr.name) ||
        attr.namespaceURI ||
        attr.value.length > 32768 ||
        /url\s*\(|[<>]|[\x00-\x1f]/i.test(attr.value)
      )
        throw new Error(`Unsupported SVG attribute: ${attr.name}`);
      if (
        ["fill", "stroke"].includes(attr.name) &&
        !/^(#[a-f\d]{3,8}|[a-z]+)$/i.test(attr.value)
      )
        throw new Error(
          "SVG colors must be plain colors; references are unsupported.",
        );
    }
  }
  const viewBox = root
    .getAttribute("viewBox")
    ?.trim()
    .split(/[\s,]+/)
    .map(Number);
  if (
    viewBox &&
    (viewBox.length !== 4 ||
      viewBox.some((n) => !Number.isFinite(n) || Math.abs(n) > 100000) ||
      viewBox[2] <= 0 ||
      viewBox[3] <= 0)
  )
    throw new Error("Invalid SVG viewBox.");
  const size = (key: string, fallback: number) => {
    const raw = root.getAttribute(key);
    if (raw && !/^\d+(?:\.\d+)?(?:px)?$/.test(raw))
      throw new Error("SVG dimensions must use pixels.");
    const n = raw ? parseFloat(raw) : fallback;
    if (!Number.isFinite(n) || n <= 0 || n > MAX_ICON_PIXELS)
      throw new Error(
        `SVG icons can be up to ${MAX_ICON_PIXELS} × ${MAX_ICON_PIXELS} pixels.`,
      );
    return n;
  };
  root.setAttribute("width", String(size("width", viewBox?.[2] ?? 32)));
  root.setAttribute("height", String(size("height", viewBox?.[3] ?? 32)));
  // Serialize only the inspected root; never insert the input document into the UI.
  return new Blob([new XMLSerializer().serializeToString(root)], {
    type: "image/svg+xml",
  });
}

export async function normalizeIcon(file: File): Promise<string> {
  if (file.size > MAX_ICON_BYTES)
    throw new Error("Choose a PNG, ICO or SVG up to 4 MB.");
  const bytes = new Uint8Array(await file.arrayBuffer());
  const view = new DataView(bytes.buffer);
  const signature = [137, 80, 78, 71, 13, 10, 26, 10];
  let blob: Blob;
  if (/\.svg$/i.test(file.name)) {
    blob = sanitizedSvg(bytes);
  } else if (/\.ico$/i.test(file.name)) {
    if (
      bytes.length < 22 ||
      view.getUint16(0, true) !== 0 ||
      view.getUint16(2, true) !== 1
    )
      throw new Error("Invalid ICO icon.");
    const count = view.getUint16(4, true);
    if (!count || count > 64 || bytes.length < 6 + count * 16)
      throw new Error("Invalid ICO directory.");
    for (let i = 0; i < count; i++) {
      const p = 6 + i * 16,
        length = view.getUint32(p + 8, true),
        offset = view.getUint32(p + 12, true);
      if (!length || offset < 6 + count * 16 || offset + length > bytes.length)
        throw new Error("Invalid ICO frame.");
      const frame = new DataView(bytes.buffer, offset, length);
      const w = bytes[p] || 256,
        h = bytes[p + 1] || 256;
      const png =
        length >= 33 && signature.every((v, j) => bytes[offset + j] === v);
      if (png) {
        if (frame.getUint32(16) !== w || frame.getUint32(20) !== h)
          throw new Error("ICO PNG dimensions do not match its directory.");
      } else if (
        length < 40 ||
        frame.getUint32(0, true) < 40 ||
        frame.getInt32(4, true) !== w ||
        frame.getInt32(8, true) !== h * 2 ||
        frame.getUint32(16, true) !== 0
      ) {
        throw new Error(
          "ICO requires bounded PNG or uncompressed bitmap frames.",
        );
      }
    }
    blob = new Blob([bytes], { type: "image/x-icon" });
  } else {
    if (
      bytes.length < 33 ||
      signature.some((v, i) => bytes[i] !== v) ||
      new TextDecoder().decode(bytes.slice(12, 16)) !== "IHDR"
    )
      throw new Error("Choose a PNG, ICO or SVG icon.");
    if (
      !view.getUint32(16) ||
      !view.getUint32(20) ||
      view.getUint32(16) > MAX_ICON_PIXELS ||
      view.getUint32(20) > MAX_ICON_PIXELS
    )
      throw new Error(
        `PNG icons can be up to ${MAX_ICON_PIXELS} × ${MAX_ICON_PIXELS} pixels.`,
      );
    blob = new Blob([bytes], { type: "image/png" });
  }
  const url = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error("Cannot read normalized icon."));
    reader.readAsDataURL(blob);
  });
  const image = new Image();
  try {
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error("Icon decoding timed out.")),
        5000,
      );
      image.onload = () => {
        clearTimeout(timer);
        resolve();
      };
      image.onerror = () => {
        clearTimeout(timer);
        reject(new Error("Cannot decode this icon."));
      };
      image.src = url;
    });
    const width = image.naturalWidth,
      height = image.naturalHeight;
    if (
      !width ||
      !height ||
      width > MAX_ICON_PIXELS ||
      height > MAX_ICON_PIXELS
    )
      throw new Error(
        `Icons can be up to ${MAX_ICON_PIXELS} × ${MAX_ICON_PIXELS} pixels.`,
      );
    // Halve large images step by step before the final draw to avoid aliasing.
    let source: CanvasImageSource = image,
      w = width,
      h = height;
    while (Math.max(w, h) > ICON_SIZE * 2) {
      const step = document.createElement("canvas");
      step.width = Math.max(1, Math.round(w / 2));
      step.height = Math.max(1, Math.round(h / 2));
      const sctx = step.getContext("2d");
      if (!sctx) throw new Error("Image conversion is unavailable.");
      sctx.imageSmoothingQuality = "high";
      sctx.drawImage(source, 0, 0, step.width, step.height);
      source = step;
      w = step.width;
      h = step.height;
    }
    const canvas = document.createElement("canvas");
    canvas.width = ICON_SIZE;
    canvas.height = ICON_SIZE;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("Image conversion is unavailable.");
    ctx.imageSmoothingQuality = "high";
    const ratio = Math.min(ICON_SIZE / w, ICON_SIZE / h);
    ctx.drawImage(
      source,
      (ICON_SIZE - w * ratio) / 2,
      (ICON_SIZE - h * ratio) / 2,
      w * ratio,
      h * ratio,
    );
    const result = canvas.toDataURL("image/png");
    if (result.length > 11000) throw new Error("Converted icon is too large.");
    return result;
  } finally {
    image.src = "";
  }
}
