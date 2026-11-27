import {
  loadApplicationWorkbenchPage,
  validateApplicationWorkbenchPageBinding,
  type BoundedWorkbenchPageResult,
  type DownloadBatchContext,
  type SceneRow,
} from "./bridge.ts";
import type { CoreAoiDisplayAuthorityProjection } from "./coreRestoreProjection.ts";
import type { DesktopWorkbenchPageRequest } from "./desktopShellBridge.ts";
import { isNisarRslcProduct } from "./nisarSubset.ts";

const DEFAULT_NISAR_SUBSET_BATCH_PAGE_SIZE = 50;
const MAX_NISAR_SUBSET_BATCH_PAGE_ITEMS = 250;

type NisarSubsetPageLoader = (
  request: DesktopWorkbenchPageRequest,
  aoiEnvelope?: CoreAoiDisplayAuthorityProjection["envelope"],
) => Promise<BoundedWorkbenchPageResult>;

export type CompleteNisarSubsetBatchItem = {
  itemId: string;
  providerId: string;
  scene: SceneRow;
};

export type CompleteNisarSubsetBatchResult =
  | {
      ok: true;
      batchId: string;
      revision: number;
      items: CompleteNisarSubsetBatchItem[];
      checkedItemIds: string[];
    }
  | { ok: false; error: string };

export async function loadCompleteNisarSubsetBatch({
  baseContext,
  aoiAuthority,
  pageSize = DEFAULT_NISAR_SUBSET_BATCH_PAGE_SIZE,
  loadPage = loadApplicationWorkbenchPage,
}: {
  baseContext: DownloadBatchContext;
  aoiAuthority: Pick<CoreAoiDisplayAuthorityProjection, "region_id" | "revision" | "envelope"> | null;
  pageSize?: number;
  loadPage?: NisarSubsetPageLoader;
}): Promise<CompleteNisarSubsetBatchResult> {
  const revision = baseContext.revision;
  if (!Number.isSafeInteger(revision) || (revision ?? -1) < 0) {
    return { ok: false, error: "当前冻结批次缺少有效版本，无法安全打开裁剪工作区。" };
  }
  const boundedPageSize = Math.min(
    MAX_NISAR_SUBSET_BATCH_PAGE_ITEMS,
    Math.max(1, Math.trunc(pageSize) || DEFAULT_NISAR_SUBSET_BATCH_PAGE_SIZE),
  );
  const items: CompleteNisarSubsetBatchItem[] = [];
  const checkedItemIds: string[] = [];
  const seenItemIds = new Set<string>();
  let totalCount: number | null = null;
  let checkedCount: number | null = null;
  let providerIdentity: string | null = null;
  const frozenCheckedItemIds = baseContext.checked_item_ids === undefined
    ? null
    : new Set(baseContext.checked_item_ids);
  const frozenProviderIdentity = typeof baseContext.query.provider === "string"
    ? baseContext.query.provider
    : null;

  try {
    for (let pageIndex = 0; ; pageIndex += 1) {
      const request = {
        batch_id: baseContext.batch_id,
        expected_revision: revision as number,
        page_index: pageIndex,
        page_size: boundedPageSize,
      };
      const page = await loadPage(request, aoiAuthority?.envelope);
      if (!validateApplicationWorkbenchPageBinding(page, request, baseContext, aoiAuthority)) {
        return { ok: false, error: "裁剪批次分页响应与当前批次、revision 或 AOI authority 不一致。" };
      }
      if (!page.ok) {
        return { ok: false, error: "完整冻结批次加载失败，请重试。" };
      }
      if (
        baseContext.checked_count !== undefined
        && page.page.checked_count !== baseContext.checked_count
      ) {
        return { ok: false, error: "裁剪批次勾选项目数与入口冻结 checked_count 不一致，已停止打开工作区。" };
      }
      if (totalCount === null) {
        totalCount = page.page.total_count;
        checkedCount = page.page.checked_count;
        if (baseContext.total_count !== undefined && baseContext.total_count !== totalCount) {
          return { ok: false, error: "裁剪批次总数与当前冻结批次绑定不一致。" };
        }
      } else if (page.page.total_count !== totalCount || page.page.checked_count !== checkedCount) {
        return { ok: false, error: "裁剪批次分页计数在加载期间发生变化，已停止打开工作区。" };
      }

      const expectedCount = Math.min(boundedPageSize, totalCount - items.length);
      if (page.page.items.length !== expectedCount || page.scenes.length !== expectedCount) {
        return { ok: false, error: "裁剪批次分页不完整，已停止打开工作区。" };
      }
      for (let offset = 0; offset < page.page.items.length; offset += 1) {
        const entry = page.page.items[offset]!;
        const scene = page.scenes[offset]!;
        const expectedOrdinal = items.length;
        const itemId = entry.item.item_id;
        const providerId = entry.item.provider_id;
        if (
          entry.ordinal !== expectedOrdinal
          || !itemId
          || !providerId
          || seenItemIds.has(itemId)
        ) {
          return { ok: false, error: "裁剪批次包含重复项目或顺序绑定无效，已停止打开工作区。" };
        }
        const projectedProvider = scene.sar_product?.provider ?? scene.provider;
        if (
          projectedProvider !== providerId
          || (providerIdentity !== null && providerId !== providerIdentity)
          || (frozenProviderIdentity !== null && providerId !== frozenProviderIdentity)
        ) {
          return { ok: false, error: "裁剪批次项目的 provider 绑定不一致，已停止打开工作区。" };
        }
        if (!isNisarRslcProduct(scene.sar_product)) {
          return { ok: false, error: "裁剪批次包含非 NISAR RSLC 项目，已停止打开工作区。" };
        }
        providerIdentity ??= providerId;
        seenItemIds.add(itemId);
        items.push({ itemId, providerId, scene: structuredClone(scene) });
        if (entry.checked) checkedItemIds.push(itemId);
      }
      if (items.length === totalCount) break;
    }
  } catch {
    return { ok: false, error: "完整冻结批次加载失败，请重试。" };
  }

  if (checkedCount === null || checkedItemIds.length !== checkedCount) {
    return { ok: false, error: "裁剪批次勾选项目数与冻结 checked_count 不一致，已停止打开工作区。" };
  }
  if (
    frozenCheckedItemIds !== null
    && (
      frozenCheckedItemIds.size !== checkedItemIds.length
      || checkedItemIds.some((itemId) => !frozenCheckedItemIds.has(itemId))
    )
  ) {
    return { ok: false, error: "裁剪批次勾选项目集合与入口冻结 checked_item_ids 不一致，已停止打开工作区。" };
  }

  return {
    ok: true,
    batchId: baseContext.batch_id,
    revision: revision as number,
    items,
    checkedItemIds,
  };
}