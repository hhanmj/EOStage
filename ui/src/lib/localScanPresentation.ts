import type { CoreTaskLocalScanProjection } from "./coreRestoreProjection.ts";

/**
 * Downloader D3-2: the task card's progress line for the local pre-scan
 * (integrator decision 2026-10-20). While the scan runs it reads
 * "正在扫描已有文件 k/N"; once items reused an existing file it adds
 * "已复用 a", naming how many were reused on size and a readable format only
 * ("其中 b 个未做内容校验"). Returns null when there is nothing to say.
 */
export function localScanProgressText(scan: CoreTaskLocalScanProjection | null | undefined): string | null {
  if (!scan) return null;
  const parts: string[] = [];
  if (scan.state === "scanning") {
    parts.push(`正在扫描已有文件 ${scan.scanned_items}/${scan.total_items}`);
  }
  if (scan.reused_count > 0) {
    parts.push(
      scan.unverified_count > 0
        ? `已复用 ${scan.reused_count}（其中 ${scan.unverified_count} 个未做内容校验）`
        : `已复用 ${scan.reused_count}`,
    );
  }
  return parts.length > 0 ? parts.join(" · ") : null;
}
