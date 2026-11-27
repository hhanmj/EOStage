import { loadApplicationTaskSourceBatchPage, formatBridgeError } from "./bridge.ts";
import { projectFrozenItemToScene } from "./applicationServiceProjection.ts";
import type { NisarSubsetWorkspaceRow } from "./nisarSubsetWorkspace.ts";

/** Read the task's immutable source catalogue in bounded pages, never the active workbench. */
export async function loadNisarAppendCandidates(taskId: string, sourceBatchId: string, expectedRevision: number): Promise<NisarSubsetWorkspaceRow[]> {
  const rows: NisarSubsetWorkspaceRow[] = [];
  const seen = new Set<string>();
  let total: number | null = null;
  for (let pageIndex = 0; pageIndex < 50; pageIndex += 1) {
    const result = await loadApplicationTaskSourceBatchPage({ task_id: taskId, page_index: pageIndex, page_size: 200 });
    if (!result.ok) throw new Error(formatBridgeError(result));
    const page = result.page;
    if (page.task_id !== taskId || page.source_batch_id !== sourceBatchId || page.task_revision !== expectedRevision || page.page_index !== pageIndex
      || page.page_size !== 200 || page.total_count < 0 || page.total_count > 10000
      || (total !== null && total !== page.total_count)) throw new Error("原任务来源批次已变化，请重新打开追加工作区。");
    total = page.total_count;
    const expectedCount = Math.min(200, Math.max(0, total - pageIndex * 200));
    if (page.items.length !== expectedCount) throw new Error("原任务来源批次尚未完整读取，请重试。");
    for (const [offset, entry] of page.items.entries()) {
      if (entry.ordinal !== pageIndex * 200 + offset) throw new Error("原任务来源分页顺序不一致，请重试。");
      if (seen.has(entry.item.item_id)) throw new Error("原任务来源批次出现重复影像，请重新打开。");
      seen.add(entry.item.item_id);
      if (entry.belongs_to_task) continue;
      const product = projectFrozenItemToScene(entry.item)?.sar_product;
      if (!product) throw new Error("原任务来源影像缺少有效的产品信息，无法安全追加。");
      rows.push({ itemId: entry.item.item_id, product, subsetEligible: true,
        blockedReason: null, overlapState: "unchecked", availableSelections: [] });
    }
    if (seen.size === total) return rows;
  }
  throw new Error("原任务来源批次超出可读取范围。");
}
