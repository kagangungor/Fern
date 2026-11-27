import type { AiImage } from "./types";

export const AI_MAX_IMAGES = 4;

const AI_MAX_RAW_BYTES = 25 * 1024 * 1024;
const AI_MAX_EDGE_PX = 1280;
const AI_JPEG_QUALITY = 0.82;

const IMAGE_EXT_RE = /\.(png|jpe?g|webp|gif|bmp|avif)$/i;

let imageSeq = 0;

function nextImageId(): string {
  imageSeq += 1;
  return `ai-img-${Date.now()}-${imageSeq}`;
}

function isImageFile(file: File): boolean {
  if (file.type && file.type.toLowerCase().startsWith("image/")) return true;
  return IMAGE_EXT_RE.test(file.name);
}

export function imageFilesFrom(list: FileList | File[] | null | undefined): File[] {
  if (!list) return [];
  const arr = Array.isArray(list) ? list : Array.from(list);
  return arr.filter(isImageFile);
}

export function imageFilesFromDataTransfer(data: DataTransfer | null | undefined): File[] {
  if (!data) return [];
  const fromFiles = imageFilesFrom(data.files);
  if (fromFiles.length > 0) return fromFiles;

  if (!data.items) return [];
  const out: File[] = [];
  for (const item of Array.from(data.items)) {
    if (item.kind !== "file") continue;
    if (item.type && !item.type.toLowerCase().startsWith("image/")) continue;
    const file = item.getAsFile();
    if (file && isImageFile(file)) {
      out.push(file);
    }
  }
  return out;
}

export function dataUrlToAiImage(dataUrl: string, name = "screenshot.jpg"): AiImage {
  return {
    id: nextImageId(),
    name,
    dataUrl,
  };
}

function loadImageElement(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("image-decode-failed"));
    img.src = src;
  });
}

export async function fileToAiImage(file: File): Promise<AiImage> {
  if (!isImageFile(file)) {
    throw new Error("image-unsupported");
  }
  if (file.size > AI_MAX_RAW_BYTES) {
    throw new Error("image-too-large");
  }

  const objectUrl = URL.createObjectURL(file);
  const canvas = document.createElement("canvas");
  try {
    const img = await loadImageElement(objectUrl);
    const naturalW = img.naturalWidth || img.width;
    const naturalH = img.naturalHeight || img.height;
    if (!naturalW || !naturalH) {
      throw new Error("image-decode-failed");
    }

    const longEdge = Math.max(naturalW, naturalH);
    const scale = longEdge > AI_MAX_EDGE_PX ? AI_MAX_EDGE_PX / longEdge : 1;
    const targetW = Math.max(1, Math.round(naturalW * scale));
    const targetH = Math.max(1, Math.round(naturalH * scale));

    canvas.width = targetW;
    canvas.height = targetH;

    const ctx = canvas.getContext("2d");
    if (!ctx) {
      throw new Error("image-canvas-unavailable");
    }

    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, targetW, targetH);
    ctx.drawImage(img, 0, 0, targetW, targetH);

    const dataUrl = canvas.toDataURL("image/jpeg", AI_JPEG_QUALITY);
    return {
      id: nextImageId(),
      name: file.name || "image.jpg",
      dataUrl,
    };
  } finally {
    canvas.width = 0;
    canvas.height = 0;
    URL.revokeObjectURL(objectUrl);
  }
}
