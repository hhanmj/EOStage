/**
 * 新工作台 Landsat C2 L2 (Microsoft Planetary Computer) 下载确认卡。
 * 复用既有 DownloadDestinationConfirmation 组件（内容/交付形式显式选择）与
 * 既有 landsat / optical 下载命令；不新建第二种状态或选择系统。
 * 官方归档与展开文件集由用户显式选择（项目既定规则），无任何无感替换。
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { useI18n } from "@/i18n";
import type { AccountId } from "@/shell/api/types";

import { DownloadDestinationConfirmation } from "@/components/DownloadDestinationConfirmation";
import {
  createDownloadAcquisitionDraft,
  type ConfirmedDownloadTaskDraft,
  type DownloadAcquisitionDraft,
} from "@/lib/downloadDestinationConfirmation";
import {
  ensureUsgsCredentialSession,
  hasSavedProviderLogin,
} from "@/lib/providerCredentialSessions";
import {
  formatBridgeError,
  getCredentialStatus,
  getLandsatAssetDetails,
  getUsgsSessionStatus,
  isTauriDesktopHost,
  pickDirectory,
  startLandsatAssetBatchDownload,
  startLandsatBundleBatchDownload,
  startLandsatAssetDownload,
  startLandsatBundleDownload,
  type DownloadBatchContext,
  type SceneRow,
} from "@/lib/bridge";
import type { DesktopLandsatAssetDetail } from "@/lib/desktopShellBridge";
import { explainErrorCode } from "@/shell/api/errorExplanations";
import {
  defaultAccountActionLabel,
  inferRequiredAccountFromCode,
  resolveAccountActionFromError,
} from "@/shell/api/opticalCreateError";
import { getActiveDatasetProvider } from "./ManualConditionsPanel";

const MPC_LANDSAT_PROVIDER_ID = "microsoft.planetary-computer";
const LANDSAT_C2_L2_LOGICAL_DATASET = "optical.landsat.c2.l2";

export type LandsatDownloadRequest = {
  sourceGeneration?: number;
  /** 已冻结的检索批次（当前来源视图）。 */
  batchContext: DownloadBatchContext;
  /** 选中的检索景 ID（行内为 1 个；勾选批量可多个）。 */
  itemIds: readonly string[];
  /** 预填输出目录（存储设置里的默认输出根）。 */
  outputRoot: string;
  /** 下载并发数（沿用存储设置）。 */
  concurrency: number;
  /** 任务策略名（沿用存储设置）。 */
  taskPolicy: string;
  /** 是否来自 Agent 独立会话批次。 */
  isAgentSessionBatch?: boolean;
};

function createClientDownloadTaskId() {
  const randomId = globalThis.crypto?.randomUUID?.();
  return `task-ui-${randomId || `${Date.now()}-${Math.random().toString(16).slice(2)}`}`;
}

/** 从冻结批次景行判定是否为 MPC Landsat C2 L2 检索结果。 */
export function isMpcLandsatScene(scene: {
  provider?: string | null;
  native_metadata?: Record<string, unknown> | null;
}): boolean {
  if (scene.provider !== MPC_LANDSAT_PROVIDER_ID) return false;
  const metadata = scene.native_metadata ?? {};
  return metadata.logical_dataset === LANDSAT_C2_L2_LOGICAL_DATASET
    && metadata.source_kind === "landsat"
    && metadata.product_kind === "optical_scene";
}

type PreparationState =
  | { state: "idle" }
  | { state: "preparing" }
  | { state: "failed"; error: string };

function createInitialLandsatDraft(
  request: LandsatDownloadRequest,
  options?: {
    usgsAvailable?: boolean;
    assets?: DesktopLandsatAssetDetail[];
    productFolder?: string;
  },
): DownloadAcquisitionDraft {
  const batch = request.batchContext;
  const assets = options?.assets ?? [];
  // 多景批量在代表景文件清单拉取完成前先保持 batch_complete_files=true，避免闪现「无文件清单」；一旦代表景清单就绪即解锁波段/文件勾选。
  const batchCompleteFiles = request.itemIds.length > 1 && assets.length === 0;
  const productFolder = options?.productFolder ?? "";
  const usgsAvailable = options?.usgsAvailable ?? true;
  const preferredProvider = getActiveDatasetProvider("science.landsat.c2-l2.v1");
  const initialDeliveryForm =
    preferredProvider === "usgs" && usgsAvailable
      ? "official_archive"
      : "complete_asset_set";
  return createDownloadAcquisitionDraft({
    sourceBatchId: batch.batch_id,
    sourceBatchRevision: batch.revision ?? 0,
    selectionScope: request.itemIds.length > 1 ? "checked" : "exact_item",
    selectedProductIds: [...request.itemIds],
    acquisition: {
      kind: "landsat",
      frozenParameters: {
        delivery_form: initialDeliveryForm,
        official_archive_available: usgsAvailable,
        complete_asset_set_available: true,
        content_selection: "all",
        selected_asset_keys: assets.map((asset) => asset.asset_key),
        available_assets: assets,
        product_folder_name: productFolder,
        batch_complete_files: batchCompleteFiles,
      },
    },
    destinationPrefill: request.outputRoot,
    layout: "direct",
    taskPolicy: request.taskPolicy,
    concurrency: request.concurrency,
  });
}

export function LandsatDownloadConfirmation({
  request,
  isCurrent,
  onClose,
  onTaskCreated,
  onDeferToDownloadCenter,
  activeResourceId,
  selectedScenes,
  duplicateMatches,
  duplicatePolicy,
  onDuplicatePolicyChange,
  onOutputRootChange,
  onOpenAccountSettings,
}: {
  request: LandsatDownloadRequest;
  isCurrent?: () => boolean;
  onClose: () => void;
  onTaskCreated: (result: { admittedIds: readonly string[]; rejectedCount: number | null }) => void | Promise<void>;
  onDeferToDownloadCenter?: (
    confirmed: ConfirmedDownloadTaskDraft,
    execute: () => Promise<void>,
    requestId?: string,
  ) => void;
  activeResourceId?: string | null;
  selectedScenes?: SceneRow[];
  duplicateMatches?: { scene_id: string; path: string; size_bytes?: number }[];
  duplicatePolicy?: "skip" | "overwrite";
  onDuplicatePolicyChange?: (policy: "skip" | "overwrite") => void;
  onOutputRootChange?: (dir: string) => void;
  onOpenAccountSettings?: (accountId?: AccountId) => void;
}) {
  const { t } = useI18n();
  const [draft, setDraft] = useState<DownloadAcquisitionDraft>(() => createInitialLandsatDraft(request));
  const [prepareError, setPrepareError] = useState<string | null>(null);
  const [preparation, setPreparation] = useState<PreparationState>({ state: "idle" });

  const begin = useCallback(async () => {
    setPrepareError(null);
    const batch = request.batchContext;
    if (!isTauriDesktopHost()) {
      setPrepareError(t("mapWorkbench.landsat.desktopOnly"));
      return;
    }
    const usgsAvailable = await (async () => {
      try {
        const [creds, session] = await Promise.all([
          getCredentialStatus(),
          getUsgsSessionStatus(),
        ]);
        if (!creds.ok) return false;
        if (session.ok && session.status.authenticated) return true;
        return hasSavedProviderLogin(creds.usgs);
      } catch {
        return false;
      }
    })();

    // 单景与多景批量均读取代表景（itemIds[0]）的文件清单，绝不阻塞确认卡首屏展示。
    let assets: DesktopLandsatAssetDetail[] = [];
    let productFolder = "";
    if (request.itemIds.length >= 1) {
      try {
        const details = await getLandsatAssetDetails({
          request_id: `landsat-asset-details-${Date.now()}`,
          batch_id: batch.batch_id,
          item_id: request.itemIds[0],
        });
        if (details.ok) {
          assets = details.details.assets;
          productFolder = details.details.display_id;
        }
      } catch {
        // 清单失败不阻塞完整产品交付；部分选择在确认卡上保持不可用。
      }
    }
    if (assets.length > 0 || !usgsAvailable) {
      setDraft(createInitialLandsatDraft(request, { usgsAvailable, assets, productFolder }));
    }
  }, [request, t]);

  // 同一请求只读一次清单：后端会拒绝重复的清单请求（开发模式下效果会执行两次）
  const startedRef = useRef<LandsatDownloadRequest | null>(null);
  useEffect(() => {
    if (startedRef.current === request) return;
    startedRef.current = request;
    setDraft(createInitialLandsatDraft(request));
    begin().catch(() => setPrepareError(t("mapWorkbench.landsat.prepareFailed")));
  }, [begin, request, t]);

  if (prepareError) {
    return (
      <div
        role="dialog"
        aria-modal="true"
        className="fixed inset-0 z-50 flex items-center justify-center bg-black/25 p-4"
      >
        <div className="w-full max-w-md rounded-sp-modal border border-sp-line bg-card p-4 shadow-sp-modal">
          <div className="text-sm font-medium">{t("mapWorkbench.landsat.title")}</div>
          <p role="alert" className="mt-3 text-sm text-destructive">{prepareError}</p>
          <div className="mt-4 flex justify-end gap-2">
            <button
              type="button"
              className="rounded-sp-sm border border-sp-line px-3 py-1.5 text-sm"
              onClick={onClose}
            >
              {t("mapWorkbench.landsat.close")}
            </button>
          </div>
        </div>
      </div>
    );
  }

  const submit = async (
    confirmed: ConfirmedDownloadTaskDraft,
    preassignedTaskId?: string,
    preassignedRequestId?: string,
  ) => {
    const assertCurrent = () => {
      if (isCurrent && !isCurrent()) throw new Error(t("mapWorkbench.desktop.rowSelectionExpired"));
    };
    assertCurrent();
    const parameters = confirmed.acquisition.frozenParameters;
    const deliveryForm = String(parameters.delivery_form);
    const contentSelection = String(parameters.content_selection ?? "all");
    if (deliveryForm === "official_archive") {
      const creds = await getCredentialStatus();
      const session = await getUsgsSessionStatus();
      const ensured = session.ok && session.status.authenticated
        ? ({ ok: true, value: session.status, note: t("mapWorkbench.landsat.sessionReady") } as const)
        : creds.ok
          ? await ensureUsgsCredentialSession(creds.usgs)
          : null;
      // ensureUsgsCredentialSession resolves { ok, value, note } where value is
      // the session status; it may also return the raw not-ok bridge result.
      // Only a result that is neither ok nor carries an authenticated status
      // is a real failure — a successful restore must never surface as an error.
      const rawStatus = ensured !== null && !ensured.ok && "status" in ensured
        ? (ensured as { status?: { authenticated?: boolean } }).status
        : null;
      const ensuredAuthenticated = ensured !== null
        && (ensured.ok ? ensured.value.authenticated : Boolean(rawStatus?.authenticated));
      if (!ensuredAuthenticated) {
        const specificError =
          (ensured && !ensured.ok && ensured.error.trim()) ||
          (!creds.ok ? formatBridgeError(creds).trim() : "") ||
          "";
        const specificCode =
          (ensured && !ensured.ok && "code" in ensured && typeof ensured.code === "string" ? ensured.code : undefined) ||
          (!creds.ok && "code" in creds && typeof creds.code === "string" ? creds.code : undefined) ||
          "USGS_M2M_AUTH_SESSION_FAILED";
        const requiredAccount = inferRequiredAccountFromCode(specificCode);
        throw Object.assign(new Error(specificError || t("mapWorkbench.landsat.sessionUnavailable")), {
          code: specificCode,
          ...(requiredAccount === "usgs_m2m"
            ? {
                actionAccountId: "usgs_m2m" as const,
                actionLabel: defaultAccountActionLabel("usgs_m2m"),
              }
            : {}),
        });
      }
    }
    const selectedAssetKeys = Array.isArray(parameters.selected_asset_keys)
      ? parameters.selected_asset_keys.filter(
          (value): value is string => typeof value === "string" && Boolean(value.trim()),
        )
      : [];
    const taskId = preassignedTaskId || createClientDownloadTaskId();
    const batch = request.batchContext;
    assertCurrent();
    const created = confirmed.selectedProductIds.length > 1 || confirmed.selectionScope === "checked"
      ? deliveryForm === "official_archive"
        ? await startLandsatBundleBatchDownload(
            confirmed.outputRoot,
            confirmed.selectedProductIds,
            batch,
            taskId,
            preassignedRequestId || `landsat-batch-${taskId}`,
            confirmed.concurrency,
          )
        : await startLandsatAssetBatchDownload(
            confirmed.outputRoot,
            confirmed.selectedProductIds,
            batch,
            taskId,
            preassignedRequestId || `landsat-batch-${taskId}`,
            confirmed.concurrency,
            contentSelection === "all" ? [] : selectedAssetKeys,
          )
      : deliveryForm === "official_archive"
        ? await startLandsatBundleDownload(
            confirmed.outputRoot,
            confirmed.selectedProductIds[0] ?? request.itemIds[0],
            confirmed.concurrency,
            batch,
            taskId,
            preassignedRequestId || `landsat-single-${taskId}`,
            false,
          )
        : await startLandsatAssetDownload(
            confirmed.outputRoot,
            confirmed.selectedProductIds[0] ?? request.itemIds[0],
            confirmed.concurrency,
            batch,
            taskId,
            preassignedRequestId || `landsat-single-${taskId}`,
            contentSelection === "all" ? [] : selectedAssetKeys,
          );
    if (!created.ok) {
      const admission = (created as { admission?: { admitted_count: number; requested_count: number } }).admission;
      if (admission && admission.admitted_count === 0) {
        throw new Error(t("mapWorkbench.landsat.allPreparationFailed"));
      }
      // 优先按错误码给出规范中文；未知码才退回后端原文
      const explained = explainErrorCode(created.code);
      const message = explained || created.error || t("mapWorkbench.landsat.createFailed");
      const accountAction = resolveAccountActionFromError(
        { code: created.code, error: created.error || message },
        deliveryForm === "official_archive" ? "science.landsat.c2-l2.v1" : null,
      );
      throw Object.assign(new Error(message), {
        ...(created.code ? { code: created.code } : {}),
        ...(accountAction.actionAccountId ? { actionAccountId: accountAction.actionAccountId } : {}),
        ...(accountAction.actionLabel ? { actionLabel: accountAction.actionLabel } : {}),
      });
    }
    const isBatch = confirmed.selectedProductIds.length > 1 || confirmed.selectionScope === "checked";
    const admission = "admission" in created ? created.admission : undefined;
    const rejections = Array.isArray(admission?.rejections) ? admission.rejections : [];
    const rejected = new Set(rejections.map((item) => item?.source_item_id));
    const admissionValid = Boolean(admission && Array.isArray(admission.rejections)
      && Number.isSafeInteger(admission.requested_count) && Number.isSafeInteger(admission.admitted_count)
      && Number.isSafeInteger(admission.rejected_count) && admission.admitted_count >= 0
      && admission.rejected_count >= 0 && admission.requested_count === confirmed.selectedProductIds.length
      && new Set(confirmed.selectedProductIds).size === confirmed.selectedProductIds.length
      && rejections.length === admission.rejected_count && rejected.size === admission.rejected_count
      && admission.admitted_count + admission.rejected_count === admission.requested_count
      && [...rejected].every((id) => typeof id === "string" && confirmed.selectedProductIds.includes(id)));
    const admittedIds = isBatch
      ? admissionValid
        ? confirmed.selectedProductIds.filter((id) => !rejected.has(id))
        : []
      : [...confirmed.selectedProductIds];
    await onTaskCreated({ admittedIds, rejectedCount: isBatch ? (admissionValid ? admission?.rejected_count ?? null : null) : 0 });
  };

  return (
    <>
      <DownloadDestinationConfirmation
        key={`${draft.sourceBatchId}:${draft.sourceBatchRevision}:${draft.selectedProductIds.join(",")}:${Array.isArray(draft.acquisition.frozenParameters.available_assets) ? draft.acquisition.frozenParameters.available_assets.length : 0}:${String(draft.acquisition.frozenParameters.official_archive_available)}`}
        draft={draft}
        activeResourceId={activeResourceId ?? "science.landsat.c2-l2.v1"}
        selectedScenes={selectedScenes}
        duplicateMatches={duplicateMatches}
        duplicatePolicy={duplicatePolicy}
        onDuplicatePolicyChange={onDuplicatePolicyChange}
        onOutputRootChange={onOutputRootChange}
        onOpenAccountSettings={onOpenAccountSettings}
        chooseDirectory={async () => {
          const picked = await pickDirectory(t("mapWorkbench.desktop.chooseTaskOutput"));
          if (!picked.ok) throw new Error(t("mapWorkbench.desktop.chooseDirectoryFailed"));
          return picked.path?.trim() || null;
        }}
        onCancel={onClose}
        onConfirm={async (confirmed) => {
          if (onDeferToDownloadCenter) {
            const taskId = createClientDownloadTaskId();
            const isBatch = confirmed.selectedProductIds.length > 1 || confirmed.selectionScope === "checked";
            const requestId = isBatch ? `landsat-batch-${taskId}` : `landsat-single-${taskId}`;
            onDeferToDownloadCenter(confirmed, () => submit(confirmed, taskId, requestId), isBatch ? requestId : undefined);
            onClose();
            return;
          }
          setPreparation({ state: "preparing" });
          try {
            await submit(confirmed);
            setPreparation({ state: "idle" });
            onClose();
          } catch (cause) {
            const message = cause instanceof Error && cause.message.trim()
              ? cause.message
              : t("mapWorkbench.landsat.createFailed");
            setPreparation({ state: "failed", error: message });
            throw cause;
          }
        }}
      />
      {preparation.state === "preparing" && (
        <div role="status" className="fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-xl border border-border/70 bg-card px-4 py-2 text-sm shadow-lg">
          {t("mapWorkbench.landsat.creating")}
        </div>
      )}
      {preparation.state === "failed" && (
        <div role="alert" className="fixed bottom-4 left-1/2 z-50 max-w-lg -translate-x-1/2 rounded-xl border border-destructive/40 bg-destructive/10 px-4 py-2 text-sm text-destructive shadow-lg">
          {preparation.error}
        </div>
      )}
    </>
  );
}
