export type PreviewBatchReference = {
  batch_id?: string | null;
  source_batch_id?: string | null;
};

export class PreviewSaveDirectorySession {
  private readonly directories = new Map<string, string>();

  get(batchId: string): string | null {
    const key = batchId.trim();
    return key ? this.directories.get(key) ?? null : null;
  }

  remember(batchId: string, directory: string): void {
    const key = batchId.trim();
    const value = directory.trim();
    if (!key || !value) return;
    this.directories.set(key, value);
  }

  forget(batchId: string): void {
    const key = batchId.trim();
    if (key) this.directories.delete(key);
  }
}

export function resolvePreviewSaveBatchId(
  currentBatchId: string,
  taskBatch: PreviewBatchReference | null | undefined,
): string {
  return (
    taskBatch?.source_batch_id?.trim()
    || taskBatch?.batch_id?.trim()
    || currentBatchId.trim()
    || "unbound-search"
  );
}

export function previewProductFileBase(value: string): string {
  const withoutExtension = value
    .trim()
    .replace(/\.(zip|tar|tif|tiff|jpg|jpeg|png|webp)$/i, "");
  const safe = withoutExtension
    .replace(/[<>:"/\\|?*\u0000-\u001f]+/g, "_")
    .replace(/_+/g, "_")
    .replace(/^[ ._]+|[ ._]+$/g, "")
    .slice(0, 180);
  return safe || "product";
}
