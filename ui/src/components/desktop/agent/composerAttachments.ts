/**
 * composerAttachments.ts
 * Multimodal attachment parsing, validation, and wire projection matching
 * `SendContext.attachments` (`SendAttachment` with `deny_unknown_fields`).
 */

export type WireAgentAttachmentKind = "image" | "text" | "file";

export interface WireAgentAttachment {
  kind: WireAgentAttachmentKind;
  name: string;
  mime_type: string;
  data_base64?: string;
  text_content?: string;
  size_bytes?: number;
}

export interface ComposerAttachment {
  id: string;
  kind: WireAgentAttachmentKind;
  name: string;
  mimeType: string;
  sizeBytes: number;
  previewUrl?: string;
  dataBase64?: string;
  textContent?: string;
}

export const MAX_COMPOSER_ATTACHMENTS = 6;
export const MAX_IMAGE_BASE64_CHARS = 8 * 1024 * 1024;
export const MAX_TEXT_CONTENT_CHARS = 65_536;
export const MAX_FILE_READ_BYTES = 6 * 1024 * 1024;

export const SUPPORTED_IMAGE_MIME_TYPES: ReadonlySet<string> = new Set([
  "image/png",
  "image/jpeg",
  "image/jpg",
  "image/webp",
  "image/gif",
]);

const IMAGE_EXT_TO_MIME: Readonly<Record<string, string>> = {
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".webp": "image/webp",
  ".gif": "image/gif",
};

const TEXT_EXTENSIONS: ReadonlySet<string> = new Set([
  ".txt",
  ".md",
  ".markdown",
  ".csv",
  ".tsv",
  ".json",
  ".geojson",
  ".xml",
  ".kml",
  ".yaml",
  ".yml",
  ".log",
  ".py",
  ".ts",
  ".tsx",
  ".js",
  ".jsx",
  ".rs",
  ".sql",
  ".toml",
  ".ini",
  ".cfg",
  ".wkt",
  ".prj",
  ".sh",
  ".html",
  ".css",
]);

const DATA_FILE_EXTENSIONS: ReadonlySet<string> = new Set([
  ".geojson",
  ".kml",
  ".json",
  ".csv",
  ".tsv",
  ".xml",
  ".wkt",
  ".prj",
]);

export const COMPOSER_ATTACHMENT_ACCEPT = [
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/gif",
  ".txt",
  ".md",
  ".csv",
  ".tsv",
  ".json",
  ".geojson",
  ".xml",
  ".kml",
  ".yaml",
  ".yml",
  ".log",
  ".py",
  ".ts",
  ".js",
  ".rs",
  ".wkt",
].join(",");

function fileExtension(name: string): string {
  const clean = name.trim().split(/[\\/]/).pop() || "";
  const idx = clean.lastIndexOf(".");
  return idx > 0 ? clean.slice(idx).toLowerCase() : "";
}

export function sanitizeAttachmentBasename(
  rawName: string | null | undefined,
  fallback: string = "attachment.txt"
): string {
  const base = (rawName || "").trim().split(/[\\/]/).pop()?.trim() || "";
  const safe = (base || fallback).replace(/[\u0000-\u001f\u007f]/g, "");
  if (!safe) return fallback;
  return safe.length > 160 ? safe.slice(0, 160) : safe;
}

export function resolveImageMimeType(file: Pick<File, "name" | "type">): string | null {
  const rawType = (file.type || "").trim().toLowerCase();
  if (SUPPORTED_IMAGE_MIME_TYPES.has(rawType)) {
    return rawType === "image/jpg" ? "image/jpeg" : rawType;
  }
  const ext = fileExtension(file.name || "");
  return IMAGE_EXT_TO_MIME[ext] ?? null;
}

export function isSupportedImageFile(file: Pick<File, "name" | "type">): boolean {
  return resolveImageMimeType(file) !== null;
}

export function isTextReadableFile(file: Pick<File, "name" | "type">): boolean {
  const rawType = (file.type || "").trim().toLowerCase();
  if (
    rawType.startsWith("text/") ||
    rawType === "application/json" ||
    rawType === "application/geo+json" ||
    rawType === "application/xml" ||
    rawType === "application/javascript" ||
    rawType === "application/x-yaml"
  ) {
    return true;
  }
  return TEXT_EXTENSIONS.has(fileExtension(file.name || ""));
}

export function inferAttachmentMimeType(file: Pick<File, "name" | "type">): string {
  const imgMime = resolveImageMimeType(file);
  if (imgMime) return imgMime;
  const rawType = (file.type || "").trim().toLowerCase();
  if (rawType) return rawType;
  const ext = fileExtension(file.name || "");
  if (ext === ".geojson") return "application/geo+json";
  if (ext === ".json") return "application/json";
  if (ext === ".xml" || ext === ".kml") return "application/xml";
  if (ext === ".csv") return "text/csv";
  if (ext === ".md" || ext === ".markdown") return "text/markdown";
  if (TEXT_EXTENSIONS.has(ext)) return "text/plain";
  return "application/octet-stream";
}

export function formatAttachmentSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(bytes >= 10 * 1024 ? 0 : 1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function extractClipboardFiles(clipboardData: DataTransfer | null | undefined): File[] {
  if (!clipboardData) return [];
  const files: File[] = [];
  const seenKeys = new Set<string>();

  const pushUnique = (file: File | null) => {
    if (!file) return;
    const key = `${file.name}::${file.size}::${file.type}::${file.lastModified}`;
    if (seenKeys.has(key)) return;
    seenKeys.add(key);
    files.push(file);
  };

  if (clipboardData.items && clipboardData.items.length > 0) {
    for (let i = 0; i < clipboardData.items.length; i++) {
      const item = clipboardData.items[i];
      if (item && item.kind === "file") {
        pushUnique(item.getAsFile());
      }
    }
  }
  if (clipboardData.files && clipboardData.files.length > 0) {
    for (let i = 0; i < clipboardData.files.length; i++) {
      pushUnique(clipboardData.files[i]);
    }
  }
  return files;
}

async function readAsDataUrl(file: File, fallbackMime: string): Promise<string> {
  if (typeof file.arrayBuffer === "function") {
    const buffer = await file.arrayBuffer();
    const bytes = new Uint8Array(buffer);
    let binary = "";
    const chunkSize = 0x8000;
    for (let i = 0; i < bytes.length; i += chunkSize) {
      binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
    }
    const mime = (file.type || fallbackMime || "application/octet-stream").trim();
    return `data:${mime};base64,${btoa(binary)}`;
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === "string") resolve(reader.result);
      else reject(new Error("无法读取图片数据"));
    };
    reader.onerror = () => reject(new Error("读取图片失败"));
    reader.readAsDataURL(file);
  });
}

function readAsText(file: File): Promise<string> {
  if (typeof file.text === "function") {
    return file.text();
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === "string") resolve(reader.result);
      else reject(new Error("无法读取文本内容"));
    };
    reader.onerror = () => reject(new Error("读取文件失败"));
    reader.readAsText(file);
  });
}

export async function readFileAsComposerAttachment(
  file: File
): Promise<{ attachment?: ComposerAttachment; error?: string }> {
  const imageMime = resolveImageMimeType(file);
  const id = `att-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;

  if (imageMime) {
    if (file.size > MAX_FILE_READ_BYTES) {
      return {
        error: `图片「${sanitizeAttachmentBasename(file.name, "pasted-image.png")}」超过 6 MB 限制`,
      };
    }
    const ext = imageMime.split("/")[1] || "png";
    const name = sanitizeAttachmentBasename(file.name, `pasted-image.${ext}`);
    try {
      const dataUrl = await readAsDataUrl(file, imageMime);
      if (dataUrl.length > MAX_IMAGE_BASE64_CHARS) {
        return { error: `图片「${name}」编码后超过传输上限，请压缩后再粘贴` };
      }
      return {
        attachment: {
          id,
          kind: "image",
          name,
          mimeType: imageMime,
          sizeBytes: file.size,
          previewUrl: dataUrl,
          dataBase64: dataUrl,
        },
      };
    } catch {
      return { error: `读取图片「${name}」失败` };
    }
  }

  const name = sanitizeAttachmentBasename(file.name, "attachment.txt");
  const mimeType = inferAttachmentMimeType(file);
  const ext = fileExtension(name);
  const kind: WireAgentAttachmentKind = DATA_FILE_EXTENSIONS.has(ext) ? "file" : "text";

  if (isTextReadableFile(file)) {
    try {
      const rawText = await readAsText(file);
      const clipped =
        rawText.length > MAX_TEXT_CONTENT_CHARS
          ? `${rawText.slice(0, MAX_TEXT_CONTENT_CHARS - 24)}\n...[内容过长已截断]`
          : rawText;
      return {
        attachment: {
          id,
          kind,
          name,
          mimeType,
          sizeBytes: file.size,
          textContent: clipped,
        },
      };
    } catch {
      return { error: `读取文件「${name}」失败` };
    }
  }

  return {
    error: `暂不支持直接附加「${name}」，支持 PNG/JPG/WebP/GIF 图片或文本/JSON/GeoJSON/KML/CSV 文件`,
  };
}

/**
 * Strictly projects ComposerAttachment[] into the backend `SendAttachment` wire contract
 * (`deny_unknown_fields`: ONLY `kind`, `name`, `mime_type`, optional `data_base64`, `text_content`, `size_bytes`).
 */
export function toWireAttachments(
  items: readonly ComposerAttachment[]
): WireAgentAttachment[] {
  return items.slice(0, MAX_COMPOSER_ATTACHMENTS).map((item) => {
    const wire: WireAgentAttachment = {
      kind: item.kind,
      name: sanitizeAttachmentBasename(item.name),
      mime_type: item.mimeType,
    };
    if (item.kind === "image" && item.dataBase64) {
      wire.data_base64 = item.dataBase64;
    } else if (item.textContent !== undefined) {
      wire.text_content = item.textContent;
    }
    if (typeof item.sizeBytes === "number" && Number.isFinite(item.sizeBytes) && item.sizeBytes >= 0) {
      wire.size_bytes = Math.round(item.sizeBytes);
    }
    return wire;
  });
}
