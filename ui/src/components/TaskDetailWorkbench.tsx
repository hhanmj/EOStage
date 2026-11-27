import { FrozenProductPreview } from "./FrozenProductPreview";
import { useI18n } from "@/i18n";
import { SourceMutationStatusNotice } from "@/components/SourceMutationStatusNotice";
import { useSourceMutationStatus } from "@/lib/useSourceMutationStatus";
import { useReacquireStatus } from "@/lib/useReacquireStatus";
import { NisarSubsetWorkspace, type NisarSubsetWorkspacePreflight } from "@/components/NisarSubsetWorkspace";
import { loadNisarAppendCandidates } from "@/lib/nisarAppendWorkspace";
import { hasPendingNisarRecovery } from "@/lib/sourceOperationCoordination";
import { deriveWrappedAoiEnvelope, continuousDisplayBbox } from "@/lib/antimeridian";
import type { DesktopCoreRestoreSnapshot, DesktopNisarAppendContextV2 } from "@/lib/desktopShellBridge";
import type { NisarSubsetWorkspaceDraft, NisarSubsetWorkspaceRow } from "@/lib/nisarSubsetWorkspace";
import { taskFrozenSubsetAoi } from "@/lib/taskFrozenAoi";
import { AoiDrawMap } from "@/components/AoiDrawMap";
import { formatByteSize } from "@/lib/formatByteSize";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { Activity, ExternalLink, FolderOpen, Gauge, LockKeyhole, X } from "lucide-react";

import { TaskPageDetail } from "@/components/TaskPageDetail";
import { TaskSourceBatchResults } from "@/components/TaskSourceBatchResults";
import { DownloadDestinationConfirmation } from "@/components/DownloadDestinationConfirmation";
import { ReacquireConfirmation } from "@/components/ReacquireConfirmation";
import {
  acquireSourceMutationInFlight,
  beginSourceMutation,
  finishSourceMutation,
  getSourceMutationGate,
  isSourceMutationInFlight,
  isSourceMutationStorageCorrupted,
  isNisarSourceMutation,
  nisarDirectResponseContainsTarget,
  noteSourceMutationSuccess,
  releaseSourceMutationInFlight,
  sourceMutationEpoch,
  sourceMutationGateVersion,
  stageRetrySourceMutation,
  subscribeSourceMutationGate,
  VALID_DOWNLOAD_LAYOUTS,
  type FrozenSourceMutation,
  type SourceMutationGate,
} from "@/lib/taskSourceMutationGate";
import {
  getPendingReacquireRecord,
  getSourceSuccessEpoch,
  reacquireGateVersion,
  isStorageCorrupted,
  subscribeReacquireGate,
  releaseInFlightLock,
  stageReacquireRequest,
  tryAcquireInFlightLock,
  tryCommitReacquireSuccess,
  updateReacquireLastError,
  type FrozenReacquireRequest,
} from "@/lib/reacquireRequestGate";
import { presentLocalReuse } from "@/lib/localReusePresentation";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  appendApplicationBurstAssetTask,
  appendApplicationTaskFromSourceBatch,
  appendNisarSubsetBatchTaskV2,
  readNisarSubsetAppendContextV2,
  preflightNisarSubsetAppendV2,
  createNisarSubsetSuccessorTaskV2,
  startNewlyConfirmedTask,
  createApplicationHistoricalBurstAssetTask,
  createApplicationTaskFromHistoricalSourceBatch,
  formatBridgeError,
  loadApplicationTaskSourceBatchPage,
  updateApplicationDownloadTask,
  type TaskEventProjection,
} from "@/lib/bridge";
import { asfRestrictedAccessPresentation } from "@/lib/asfRestrictedAccess";
import {
  createDownloadAcquisitionDraft,
  type ConfirmedDownloadTaskDraft,
  type DownloadAcquisitionDraft,
} from "@/lib/downloadDestinationConfirmation";
import type { DownloadLayout, StoreTaskSourceBatchPage } from "@/lib/generated/v3-contracts";
import { OpticalDeliveryReport } from "@/components/OpticalDeliveryReport";
import {
  createTaskHistoricalUiState,
  taskHistoricalAction,
  updateTaskHistoricalUiState,
} from "@/lib/taskHistoricalContext";
import {
  etaSeconds,
  taskItemHasCurrentFailure,
  taskPercent,
  type TaskPagePresentation,
} from "@/lib/taskPagePresentation";

type Tab = "search_results" | "download_status" | "events" | "task_info";

function uniqueId(prefix: string): string {
  const randomUuid = globalThis.crypto?.randomUUID?.();
  return randomUuid ? `${prefix}-${randomUuid}` : `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function formatBytes(value: number | null): string {
  return formatByteSize(value, "总量未知");
}

function formatElapsed(milliseconds: number): string {
  const seconds = Math.max(0, milliseconds) / 1_000;
  if (seconds < 60) return `${seconds.toFixed(seconds < 10 ? 1 : 0)} 秒`;
  const minutes = Math.floor(seconds / 60);
  const remainder = Math.floor(seconds % 60);
  return `${minutes} 分 ${remainder} 秒`;
}

function itemStatusLabel(status: string): string {
  switch (status.toLowerCase()) {
    case "active":
    case "running":
    case "downloading":
      return "下载中";
    case "queued":
    case "waiting":
    case "pending":
      return "等待";
    case "paused":
      return "已暂停";
    case "failed":
      return "失败";
    case "interrupted":
      return "已中断";
    default:
      return status;
  }
}

function eventBadgeLabel(event: TaskEventProjection): string {
  switch (event.event_kind) {
    case "started": return "开始";
    case "progress": return "进度";
    case "paused": return "暂停";
    case "resumed": return "继续";
    case "completed": return "完成";
    case "failed": return "失败";
    case "cancelled": return "结束";
    case "retried": return "重试";
    default: return event.level === "error" ? "错误" : event.level === "warning" ? "注意" : "信息";
  }
}

function formatEventTime(value: string): string {
  const timestamp = new Date(value);
  return Number.isNaN(timestamp.getTime()) ? value : timestamp.toLocaleString();
}

const NISAR_LIFECYCLE_STAGES = ["准备", "规划", "下载", "生成", "校验", "输出"] as const;

type NisarLifecycleStage = (typeof NISAR_LIFECYCLE_STAGES)[number];

function jsonRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function isNisarSubsetItem(item: TaskPagePresentation["items"][number]): boolean {
  return item.metadata?.delivery_kind === "remote_derived_subset_v2";
}

function isNisarSubsetProgressTask(
  taskType: string | null | undefined,
  page: TaskPagePresentation | null,
): boolean {
  const persistentType = taskType?.trim().toLowerCase();
  if (persistentType) return persistentType === "nisar_subset";
  return page?.items.some(isNisarSubsetItem) ?? false;
}

function subsetFactBytes(value: number | null): string {
  return value === null ? "不可用" : formatBytes(value);
}

function NisarSubsetTransferFacts({
  sourceTotalBytes,
  networkBytesReceived,
  reusableVerifiedBytes,
  reusedVerifiedBytes,
  networkLabel = "网络已接收",
}: {
  sourceTotalBytes: number | null;
  networkBytesReceived: number | null;
  reusableVerifiedBytes: number | null;
  reusedVerifiedBytes: number | null;
  networkLabel?: string;
}) {
  return (
    <div data-nisar-subset-transfer-facts>
      <dl className="grid grid-cols-2 gap-x-3 gap-y-1.5 text-[11px]">
        <div><dt className="text-muted-foreground">{networkLabel}</dt><dd className="font-medium tabular-nums">{subsetFactBytes(networkBytesReceived)}</dd></div>
        <div><dt className="text-muted-foreground">源文件总量（参考）</dt><dd className="font-medium tabular-nums">{subsetFactBytes(sourceTotalBytes)}</dd></div>
      </dl>
      <details className="mt-2 text-[11px] text-muted-foreground">
        <summary className="cursor-pointer select-none font-medium text-foreground">续传详情</summary>
        <dl className="mt-2 grid grid-cols-2 gap-x-3 gap-y-1.5">
          <div><dt>已保存并校验的数据</dt><dd className="font-medium tabular-nums text-foreground">{subsetFactBytes(reusableVerifiedBytes)}</dd></div>
          <div><dt>本次续传跳过的下载量</dt><dd className="font-medium tabular-nums text-foreground">{subsetFactBytes(reusedVerifiedBytes)}</dd></div>
        </dl>
        <p className="mt-1.5 text-[10px]">仅统计影像数据块，不含预读的元数据；任务级汇总。</p>
      </details>
    </div>
  );
}

function nisarLifecycleStatus(
  item: TaskPagePresentation["items"][number],
  stage: NisarLifecycleStage,
): string {
  if (item.status === "completed") return "已完成";
  const prepared = jsonRecord(item.metadata.nisar_subset_prepared_intent_v2_envelope) !== null;
  const planned = jsonRecord(item.metadata.nisar_subset_plan_v2_envelope) !== null;
  const failureStage = item.failure?.stage ?? "";
  const failedHere = taskItemHasCurrentFailure(item) && (
    (stage === "准备" && failureStage === "准备元数据")
    || (stage === "规划" && failureStage === "规划 AOI 子集")
    || (stage === "下载" && failureStage === "下载数据范围")
    || (stage === "生成" && failureStage === "生成 HDF5")
    || (stage === "校验" && failureStage === "校验")
    || (stage === "输出" && failureStage === "输出")
  );
  if (failedHere) return "失败";
  if (stage === "准备") return prepared ? "已准备" : "未记录";
  if (stage === "规划") return planned ? "已规划" : "未记录";
  if (stage === "下载") {
    if (["active", "running", "downloading"].includes(item.status)) return "进行中";
    if (item.bytesCompleted > 0) return "已传输部分";
    return "未开始";
  }
  return "未开始";
}

async function executeFrozenMutation(
  frozen: FrozenSourceMutation,
): Promise<{ ok: boolean; [key: string]: unknown }> {
  switch (frozen.kind) {
    case "append_source_batch":
      return appendApplicationTaskFromSourceBatch(frozen.request);
    case "burst_append":
      return appendApplicationBurstAssetTask(frozen.request);
    case "successor_source_batch":
      return createApplicationTaskFromHistoricalSourceBatch(frozen.request);
    case "burst_successor":
      return createApplicationHistoricalBurstAssetTask(frozen.request);
    case "nisar_append":
      return appendNisarSubsetBatchTaskV2(frozen.request);
    case "nisar_successor":
      return createNisarSubsetSuccessorTaskV2(frozen.request);
  }
}

async function verifyMembership(gate: SourceMutationGate): Promise<boolean> {
  if (gate.frozenMutation?.kind === "nisar_append" || gate.frozenMutation?.kind === "nisar_successor") return false;
  const remaining = new Set(gate.itemIds);
  let total: number | null = null;
  const seen = new Set<string>();
  for (let index = 0; index < 50; index += 1) {
    const result = await loadApplicationTaskSourceBatchPage({
      task_id: gate.targetTaskId,
      page_index: index,
      page_size: 200,
    });
    if (
      !result.ok
      || result.page.task_id !== gate.targetTaskId
      || result.page.source_batch_id !== gate.sourceBatchId
      || result.page.page_index !== index
      || result.page.page_size !== 200
      || result.page.task_revision < gate.minRevision
      || result.page.total_count > 10_000
      || (total !== null && result.page.total_count !== total)
    ) return false;
    total = result.page.total_count;
    const expected = Math.min(200, Math.max(0, total - index * 200));
    if (result.page.items.length !== expected) return false;
    for (const [offset, entry] of result.page.items.entries()) {
      if (entry.ordinal !== index * 200 + offset || seen.has(entry.item.item_id)) return false;
      seen.add(entry.item.item_id);
      if (entry.belongs_to_task) remaining.delete(entry.item.item_id);
    }
    if (remaining.size === 0) return true;
    if ((index + 1) * 200 >= total) return false;
  }
  return false;
}

export function TaskDetailWorkbench({
  taskId,
  revision,
  title,
  status,
  taskMutable = false,
  datasetLabel,
  concurrency,
  downloadLayout = "direct",
  newTaskConcurrency,
  newTaskDownloadLayout,
  newTaskPolicyReady = true,
  resumeSupported,
  outputDir,
  completedCount,
  activeCount,
  waitingCount,
  pausedCount,
  failedCount,
  transferredBytes,
  totalBytes,
  taskType = null,
  sourceTotalBytes = null,
  networkBytesReceived = null,
  reusableVerifiedBytes = null,
  reusedVerifiedBytes = null,
  bytesPerSecond,
  transferActiveMillis,
  page,
  events,
  pageBusy = false,
  eventBusy = false,
  error = null,
  hasMoreEvents = false,
  onClose,
  onOpenOutput,
  onOpenRestrictedSupport,
  onChooseNewTaskOutput = async () => null,
  onPageIndexChange,
  onLoadOlderEvents,
  onHistoricalMutationApplied,
}: {
  taskId: string;
  revision: number;
  title: string;
  status: string;
  taskMutable?: boolean;
  datasetLabel: string;
  concurrency: number | null;
  downloadLayout?: DownloadLayout;
  newTaskConcurrency?: number;
  newTaskDownloadLayout?: DownloadLayout;
  newTaskPolicyReady?: boolean;
  resumeSupported: boolean;
  outputDir: string;
  completedCount: number;
  activeCount: number;
  waitingCount: number;
  pausedCount: number;
  failedCount: number;
  transferredBytes: number;
  totalBytes: number | null;
  taskType?: string | null;
  sourceTotalBytes?: number | null;
  networkBytesReceived?: number | null;
  reusableVerifiedBytes?: number | null;
  reusedVerifiedBytes?: number | null;
  bytesPerSecond: number;
  transferActiveMillis: number;
  page: TaskPagePresentation | null;
  events: readonly TaskEventProjection[];
  pageBusy?: boolean;
  eventBusy?: boolean;
  error?: string | null;
  hasMoreEvents?: boolean;
  onClose(): void;
  onOpenOutput(): void;
  onOpenRestrictedSupport(url: string): void;
  onChooseNewTaskOutput?: () => Promise<string | null>;
  onPageIndexChange(pageIndex: number): void;
  onLoadOlderEvents?(): void;
  onHistoricalMutationApplied?(): void;
}) {
  const { locale } = useI18n();
  const frozenSubsetAoi = taskFrozenSubsetAoi(page?.items ?? []);
  const [tab, setTab] = useState<Tab>("task_info");
  const [historical, setHistorical] = useState(() => createTaskHistoricalUiState());
  const [sourcePage, setSourcePage] = useState<StoreTaskSourceBatchPage | null>(null);
  const [sourceBusy, setSourceBusy] = useState(false);
  const [sourceError, setSourceError] = useState<string | null>(null);
  const [mutationBusy, setMutationBusy] = useState(false);
  const [itemMutationId, setItemMutationId] = useState<string | null>(null);
  // Downloader D3: the item whose 重新下载 awaits confirmation.
  const [reacquireTarget, setReacquireTarget] = useState<{
    taskId: string;
    itemId: string;
    displayName: string;
    epoch: number;
    anchorEl?: HTMLElement | null;
  } | null>(null);
  const drawerRef = useRef<HTMLElement | null>(null);
  const [reacquireError, setReacquireError] = useState<string | null>(null);
  const reacquireGeneration = useRef(0);
  const currentReacquireTask = useRef(taskId);
  currentReacquireTask.current = taskId;
  // A terminal Burst task creates successors through its own command; the
  // source-batch successor that carries reacquire_item_ids does not apply.
  const reacquireAvailable = taskMutable || datasetLabel.trim().toUpperCase() !== "SLC-BURST";
  const [mutationNotice, setMutationNotice] = useState<string | null>(null);
  const [successorConfirmation, setSuccessorConfirmation] = useState<{
    draft: DownloadAcquisitionDraft;
    burstSource: boolean;
    epoch: number;
  } | null>(null);
  const sourceGeneration = useRef(0);
  const appendGeneration = useRef(0);
  const mutationGeneration = useRef(0);
  const activeTaskIdRef = useRef(taskId);
  activeTaskIdRef.current = taskId;
  const [, setGateVersion] = useState(0);
  useEffect(() => subscribeSourceMutationGate(() => setGateVersion((value) => value + 1)), []);
  useEffect(() => subscribeReacquireGate(() => setGateVersion((value) => value + 1)), []);
  useSyncExternalStore(subscribeSourceMutationGate, sourceMutationGateVersion, () => 0);
  useSyncExternalStore(subscribeReacquireGate, reacquireGateVersion, () => 0);
  const pendingGate = getSourceMutationGate(taskId);
  const mutationStatus = useSourceMutationStatus(taskId, pendingGate);
  const pendingReacquire = getPendingReacquireRecord(taskId);
  const reacquireStatus = useReacquireStatus(taskId, pendingReacquire);
  const isSourceLocked = mutationBusy || isSourceMutationInFlight(taskId);
  const [appendSession, setAppendSession] = useState<{
    context: DesktopNisarAppendContextV2;
    rows: NisarSubsetWorkspaceRow[];
    draft: NisarSubsetWorkspaceDraft;
    outputRoot: string;
    successorTaskId: string;
    createdAt: string;
    epoch: number;
  } | null>(null);
  const [appendError, setAppendError] = useState<string | null>(null);
  const [historicalPreview, setHistoricalPreview] = useState<{ batchId: string; itemId: string; productLabel: string } | null>(null);

  const loadSourcePage = useCallback(async (pageIndex: number, pageSize: number) => {
    const generation = ++sourceGeneration.current;
    const requestedPageIndex = Math.max(0, Math.trunc(pageIndex));
    const requestedPageSize = Math.min(200, Math.max(1, Math.trunc(pageSize) || 24));
    setSourceBusy(true);
    setSourceError(null);
    try {
      const result = await loadApplicationTaskSourceBatchPage({
        task_id: taskId,
        page_index: requestedPageIndex,
        page_size: requestedPageSize,
      });
      if (generation !== sourceGeneration.current || taskId !== activeTaskIdRef.current) return;
      if (!result.ok) {
        setSourceError(formatBridgeError(result));
        return;
      }
      if (result.page.task_id !== taskId || result.page.page_index !== requestedPageIndex
        || result.page.page_size !== requestedPageSize) {
        setSourceError("来源结果的任务或分页身份不一致，请重新加载。");
        return;
      }
      setSourcePage(result.page);
    } catch (loadError) {
      if (generation === sourceGeneration.current && taskId === activeTaskIdRef.current) setSourceError(formatBridgeError(loadError));
    } finally {
      if (generation === sourceGeneration.current && taskId === activeTaskIdRef.current) setSourceBusy(false);
    }
  }, [taskId]);

  useEffect(() => {
    setTab("task_info");
    setHistorical(createTaskHistoricalUiState());
    setSourcePage(null);
    setSourceBusy(false);
    setSourceError(null);
    setMutationBusy(false);
    setMutationNotice(null);
    setSuccessorConfirmation(null);
    setAppendSession(null);
    setAppendError(null);
    setHistoricalPreview(null);
    appendGeneration.current += 1;
    reacquireGeneration.current += 1;
    mutationGeneration.current += 1;
    setItemMutationId(null);
    // 切 task 立刻使旧确认失效
    setReacquireTarget(null);
    setReacquireError(null);
    const { corrupted, reason } = isStorageCorrupted();
    if (corrupted) {
      setReacquireError(`重新下载记录存储已损坏或不可用（${reason}）。已严格锁定，禁止盲目操作。`);
    } else {
      const pending = getPendingReacquireRecord(taskId);
      if (pending) {
        setReacquireTarget({
          taskId,
          itemId: pending.itemId,
          displayName: pending.displayName,
          epoch: getSourceSuccessEpoch(taskId),
        });
        const errDesc = pending.lastError ? `：${pending.lastError}` : '';
        setReacquireError(
          `[待确认请求] 此前已发起重新下载，但响应结果尚未确认${errDesc}。` +
            `可直接点击“重新下载”重试原目标（目标任务ID: ${pending.targetTaskId}），或先取消弹窗。`,
        );
      }
    }
    void loadSourcePage(0, 24);
    return () => {
      sourceGeneration.current += 1;
      appendGeneration.current += 1;
      reacquireGeneration.current += 1;
      mutationGeneration.current += 1;
    };
  }, [loadSourcePage, taskId]);

  async function openNisarAppendWorkspace(selectedIds: string[] = []) {
    if (mutationBusy || isSourceMutationInFlight(taskId) || getSourceMutationGate(taskId)
      || getPendingReacquireRecord(taskId) || hasPendingNisarRecovery(taskId)
      || isStorageCorrupted().corrupted || isSourceMutationStorageCorrupted()) return;
    const generation = ++appendGeneration.current;
    const epoch = sourceMutationEpoch(taskId);
    setHistoricalPreview(null);
    setMutationBusy(true);
    setSourceError(null);
    setAppendError(null);
    try {
      const result = await readNisarSubsetAppendContextV2({ task_id: taskId });
      if (!result.ok) throw new Error(formatBridgeError(result));
      const context = result.context;
      const rows = await loadNisarAppendCandidates(taskId, context.source_batch_id, context.task_revision);
      if (generation !== appendGeneration.current || taskId !== activeTaskIdRef.current) return;
      if (epoch !== sourceMutationEpoch(taskId) || isSourceMutationInFlight(taskId)
        || getSourceMutationGate(taskId) || getPendingReacquireRecord(taskId) || hasPendingNisarRecovery(taskId)
        || isStorageCorrupted().corrupted || isSourceMutationStorageCorrupted()) throw new Error("来源操作状态已变化，请重新打开工作区。");
      if (rows.length === 0) throw new Error("原任务来源批次中没有可追加的新影像。");
      const wanted = new Set(selectedIds);
      const selected = rows.filter((row) => wanted.size === 0 || wanted.has(row.itemId)).map((row) => row.itemId);
      setAppendSession({ context, rows, outputRoot: context.output_root, successorTaskId: uniqueId("nisar-subset-successor"), epoch,
        createdAt: new Date().toISOString(),
        draft: { sourceBatchId: context.source_batch_id, selectedItemIds: selected,
          requestedGeometry: structuredClone(context.frozen_aoi_geojson),
          selectedFrequency: context.selected_frequency, selectedPolarizations: [...context.selected_polarizations] } });
    } catch (error) {
      if (generation === appendGeneration.current && taskId === activeTaskIdRef.current) setSourceError(formatBridgeError(error));
    } finally {
      if (generation === appendGeneration.current && taskId === activeTaskIdRef.current) setMutationBusy(false);
    }
  }

  async function preflightNisarAppend(draft: NisarSubsetWorkspaceDraft): Promise<NisarSubsetWorkspacePreflight> {
    const generation = appendGeneration.current;
    const fresh = await readNisarSubsetAppendContextV2({ task_id: taskId });
    if (!fresh.ok) throw new Error(formatBridgeError(fresh));
    const result = await preflightNisarSubsetAppendV2({ task_id: taskId,
      expected_task_revision: fresh.context.task_revision, source_item_ids: draft.selectedItemIds });
    if (!result.ok) throw new Error(formatBridgeError(result));
    if (generation !== appendGeneration.current || taskId !== activeTaskIdRef.current) throw new Error("追加工作区已关闭。");
    setAppendSession((current) => current ? { ...current, context: fresh.context } : current);
    return { sourceBatchId: result.preflight.source_batch_id, workbenchRevision: result.preflight.task_revision,
      eligibleItems: result.preflight.eligible_items.map((item) => ({ sourceItemId: item.source_item_id,
        effectiveGeometry: item.effective_geometry, availableSelections: item.available_selections })),
      excludedItems: result.preflight.excluded_items.map((item) => ({ sourceItemId: item.source_item_id, reason: item.reason })),
      commonSelections: result.preflight.common_selections };
  }

  async function confirmNisarAppend(draft: NisarSubsetWorkspaceDraft) {
    const session = appendSession;
    if (!session || mutationBusy) return;
    const generation = appendGeneration.current;
    if (!acquireSourceMutationInFlight(taskId)) return;
    let gate: SourceMutationGate | null = null;
    let closedAfterSuccess = false;
    const current = () => generation === appendGeneration.current && taskId === activeTaskIdRef.current;
    try {
      if (!current() || getSourceMutationGate(taskId) || getPendingReacquireRecord(taskId) || hasPendingNisarRecovery(taskId)
        || isStorageCorrupted().corrupted || isSourceMutationStorageCorrupted()
        || session.epoch !== sourceMutationEpoch(taskId)) throw new Error("来源操作状态已变化，请重新打开工作区。");
      setMutationBusy(true);
      setAppendError(null);
      const fresh = await readNisarSubsetAppendContextV2({ task_id: taskId });
      if (!fresh.ok) throw new Error(formatBridgeError(fresh));
      if (!current()) return;
      if (fresh.context.source_batch_id !== session.context.source_batch_id) throw new Error("来源批次已变化，请重新打开工作区。");
      const action = taskHistoricalAction(fresh.context.task_status);
      if (action !== taskHistoricalAction(session.context.task_status)) {
        setAppendSession({ ...session, context: fresh.context });
        throw new Error("原任务状态已变化，请查看更新后的操作并重新确认。");
      }
      const checked = await preflightNisarSubsetAppendV2({ task_id: taskId,
        expected_task_revision: fresh.context.task_revision, source_item_ids: draft.selectedItemIds });
      if (!checked.ok) throw new Error(formatBridgeError(checked));
      if (!current()) return;
      if (getSourceMutationGate(taskId) || getPendingReacquireRecord(taskId) || hasPendingNisarRecovery(taskId)
        || isStorageCorrupted().corrupted || isSourceMutationStorageCorrupted()
        || session.epoch !== sourceMutationEpoch(taskId)) throw new Error("来源操作状态已变化，请重新打开工作区。");
      if (checked.preflight.task_id !== taskId || checked.preflight.source_batch_id !== fresh.context.source_batch_id
        || checked.preflight.task_revision !== fresh.context.task_revision) throw new Error("预检来源身份已变化，请重新确认。");
      const eligible = new Set(checked.preflight.eligible_items.map((item) => item.source_item_id));
      const eligibleIds = draft.selectedItemIds.filter((id) => eligible.has(id));
      if (eligible.size !== checked.preflight.eligible_items.length || eligibleIds.length !== eligible.size
        || new Set(draft.selectedItemIds).size !== draft.selectedItemIds.length) throw new Error("预检影像身份不一致，请重新确认。");
      if (eligibleIds.length === 0) throw new Error("所选新影像均不满足原任务裁剪参数。");
      const latestRows = await loadNisarAppendCandidates(taskId, fresh.context.source_batch_id, checked.preflight.task_revision);
      if (!current()) return;
      const availableIds = new Set(latestRows.map((row) => row.itemId));
      if (eligibleIds.some((id) => !availableIds.has(id))) throw new Error("所选影像已属于原任务或来源已变化，请重新打开工作区。");
      if (getSourceMutationGate(taskId) || getPendingReacquireRecord(taskId) || hasPendingNisarRecovery(taskId)
        || isStorageCorrupted().corrupted || isSourceMutationStorageCorrupted()
        || session.epoch !== sourceMutationEpoch(taskId)) throw new Error("来源操作状态已变化，请重新打开工作区。");
      const now = new Date().toISOString();
      let frozenMutation: FrozenSourceMutation;
      if (action === "append") {
        frozenMutation = { kind: "nisar_append", request: { task_id: taskId,
          expected_task_revision: checked.preflight.task_revision, source_item_ids: eligibleIds,
          event_id: uniqueId("nisar-subset-append"), occurred_at: now } };
      } else if (action === "create_successor") {
        frozenMutation = { kind: "nisar_successor", request: { source_task_id: taskId,
          expected_task_revision: checked.preflight.task_revision, source_item_ids: eligibleIds,
          new_task_id: session.successorTaskId, output_root: session.outputRoot,
          layout: fresh.context.layout, concurrency: fresh.context.concurrency, created_at: session.createdAt,
          recreate_original_items: false } };
      } else throw new Error("原任务当前状态不可追加。");

      gate = beginSourceMutation({ sourceTaskId: taskId,
        targetTaskId: action === "append" ? taskId : session.successorTaskId,
        sourceBatchId: checked.preflight.source_batch_id, itemIds: eligibleIds,
        minRevision: action === "append" ? checked.preflight.task_revision + 1 : 0,
        pending: true, frozenMutation });
      if (!gate) throw new Error("无法保存原始请求，操作未发送。请检查本地存储或选择较少影像。");
      const result = await executeFrozenMutation(gate.frozenMutation!);
      if (!current()) { finishSourceMutation(gate, false); return; }
      if (!nisarDirectResponseContainsTarget(gate, result)) {
        const reason = result.ok ? "响应未确认目标任务；请重试原请求。" : formatBridgeError(result);
        finishSourceMutation(gate, false, reason);
        setAppendError(`${reason} 请关闭当前工作区，在任务详情中选择“重试原请求”。`);
        return;
      }
      noteSourceMutationSuccess(gate);
      if (!finishSourceMutation(gate, true)) throw new Error("请求已成功，但本地门禁未能清除。请重试原请求。");
      appendGeneration.current += 1;
      closedAfterSuccess = true;
      setAppendSession(null);
      setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "clear_selection" }));
      setMutationNotice(action === "append" ? `已追加 ${eligibleIds.length} 景影像。` : `已创建包含 ${eligibleIds.length} 景影像的新裁剪任务。`);
      let startError: string | null = null; if (action === "create_successor") {
        const started = await startNewlyConfirmedTask(session.successorTaskId).catch((error: unknown) => error);
        if (appendGeneration.current !== generation + 1 || taskId !== activeTaskIdRef.current) return;
        if (!started || typeof started !== "object" || !("ok" in started) || started.ok !== true) {
          startError = `新裁剪任务已创建并暂停，启动失败：${formatBridgeError(started)}。可在下载任务中重试启动。`;
        }
      }
      await loadSourcePage(sourcePage?.page_index ?? 0, sourcePage?.page_size ?? 24);
      if (appendGeneration.current === generation + 1 && taskId === activeTaskIdRef.current) { onHistoricalMutationApplied?.(); if (startError) setSourceError(startError); }
    } catch (error) {
      if (gate) finishSourceMutation(gate, false, formatBridgeError(error));
      if (current()) setAppendError(gate
        ? `${formatBridgeError(error)} 请关闭当前工作区，在任务详情中选择“重试原请求”。`
        : formatBridgeError(error));
    } finally {
      releaseSourceMutationInFlight(taskId);
      if ((current() || (closedAfterSuccess && appendGeneration.current === generation + 1)) && taskId === activeTaskIdRef.current) setMutationBusy(false);
    }
  }

  async function submitHistoricalSelection(itemIds: string[]) {
    const uniqueItemIds = Array.from(new Set(itemIds));
    if (!sourcePage || sourcePage.task_id !== taskId || pendingGate || getPendingReacquireRecord(taskId)
      || isStorageCorrupted().corrupted || sourceBusy || sourceError
      || uniqueItemIds.length !== itemIds.length || itemIds.length === 0 || itemIds.length > 250
      || isSourceLocked || isSourceMutationStorageCorrupted() || successorConfirmation) return;
    const nonEmptyItemIds = itemIds as [string, ...string[]];
    const action = taskHistoricalAction(sourcePage.task_status);
    const burstSource = datasetLabel.trim().toUpperCase() === "SLC-BURST"
      || sourcePage.items.some((entry) => (
        String(entry.item.metadata.dataset ?? "").trim().toUpperCase() === "SLC-BURST"
      ));
    const nisarSubsetTask = isNisarSubsetProgressTask(taskType, page)
      || datasetLabel.trim().toUpperCase().includes("NISAR")
      || (page?.items.some(isNisarSubsetItem) ?? false)
      || sourcePage.items.some((entry) => {
        const d = String(entry.item.metadata?.dataset ?? "").toUpperCase();
        const kind = String(entry.item.metadata?.delivery_kind ?? "").toLowerCase();
        const p = String(entry.item.provider_id ?? "").toUpperCase();
        return d.includes("NISAR") || kind === "remote_derived_subset_v2" || p.includes("NISAR");
      });
    if (nisarSubsetTask) { await openNisarAppendWorkspace(itemIds); return; }
    if (action === "unavailable") {
      setSourceError("当前任务状态不允许追加产品或创建后继任务。");
      return;
    }
    if (action === "create_successor" && !burstSource && !newTaskPolicyReady) {
      setSourceError("应用级 SAR 下载设置尚未成功加载，无法创建新的历史后继任务。");
      return;
    }
    setSourceError(null);
    setMutationNotice(null);

    if (action === "create_successor") {
      try {
        const selectedLayout: DownloadLayout = burstSource
          ? "direct"
          : (VALID_DOWNLOAD_LAYOUTS.has(newTaskDownloadLayout as DownloadLayout)
              ? (newTaskDownloadLayout as DownloadLayout)
              : (VALID_DOWNLOAD_LAYOUTS.has(downloadLayout as DownloadLayout)
                  ? (downloadLayout as DownloadLayout)
                  : "direct"));

        setSuccessorConfirmation({
          draft: createDownloadAcquisitionDraft({
            sourceBatchId: sourcePage.source_batch_id,
            sourceBatchRevision: sourcePage.task_revision,
            selectionScope: "checked",
            selectedProductIds: nonEmptyItemIds,
            acquisition: {
              kind: burstSource ? "burst" : "historical_asf",
              frozenParameters: {
                source_task_id: taskId,
                expected_source_task_revision: sourcePage.task_revision,
              },
            },
            destinationPrefill: outputDir,
            layout: selectedLayout,
            taskPolicy: "manual",
            concurrency: Math.min(8, Math.max(1, Math.trunc(newTaskConcurrency ?? concurrency ?? 1) || 1)),
          }),
          burstSource,
          epoch: sourceMutationEpoch(taskId),
        });
      } catch (draftError) {
        setSourceError(formatBridgeError(draftError));
      }
      return;
    }

    // action === "append"
    if (!acquireSourceMutationInFlight(taskId)) {
      setSourceError("当前任务已有正在进行的来源操作，请勿重复点击。");
      return;
    }
    try {
      setMutationBusy(true);
      const currentGen = ++mutationGeneration.current;
      const initialTaskId = taskId;

      const now = new Date().toISOString();
      const frozenMutation: FrozenSourceMutation = burstSource
        ? {
            kind: "burst_append",
            request: {
              request_id: uniqueId("burst-task-source-append"),
              task_id: taskId,
              selected_product_ids: [...itemIds],
              expected_task_revision: sourcePage.task_revision,
              event_id: uniqueId("burst-task-source-append-event"),
              occurred_at: now,
            },
          }
        : {
            kind: "append_source_batch",
            request: {
              task_id: taskId,
              expected_task_revision: sourcePage.task_revision,
              item_ids: [...itemIds] as [string, ...string[]],
              event_id: uniqueId("task-source-append"),
              occurred_at: now,
            },
          };

      const gate = beginSourceMutation({
        sourceTaskId: taskId,
        targetTaskId: taskId,
        sourceBatchId: sourcePage.source_batch_id,
        itemIds: [...itemIds],
        minRevision: sourcePage.task_revision + 1,
        pending: true,
        frozenMutation,
      });

      if (!gate) {
        setMutationBusy(false);
        setSourceError("无法持久化来源门禁或存储不可用，已安全阻止发送。");
        return;
      }

      try {
        let bridgeResult: { ok: boolean; [key: string]: unknown } | null = null;
        let bridgeError: unknown = null;
        try {
          bridgeResult = await executeFrozenMutation(gate.frozenMutation!);
        } catch (err) {
          bridgeError = err;
        }

        const isDirectOk = bridgeResult?.ok === true;
        if (isDirectOk) {
          noteSourceMutationSuccess(gate);
        }

        const membershipFound = await verifyMembership(gate).catch(() => false);

        // Late response protection: if user switched task or component unmounted,
        // retain gate in unconfirmed state and never touch new task UI.
        if (currentGen !== mutationGeneration.current || initialTaskId !== activeTaskIdRef.current) {
          finishSourceMutation(
            gate,
            false,
            isDirectOk ? undefined : (bridgeResult ? formatBridgeError(bridgeResult) : formatBridgeError(bridgeError)),
          );
          return;
        }

        if (isDirectOk && membershipFound) {
          const finished = finishSourceMutation(gate, true);
          if (!finished) {
            setSourceError("未能安全清除门禁记录（存储写入失败）。");
            return;
          }
          setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "clear_selection" }));
          setMutationNotice(`已将 ${itemIds.length} 个新产品追加到当前任务。`);
          await loadSourcePage(sourcePage.page_index, sourcePage.page_size);
          void Promise.resolve().then(() => onHistoricalMutationApplied?.()).catch(console.warn);
        } else {
          const errStr = !isDirectOk
            ? (bridgeResult ? formatBridgeError(bridgeResult) : formatBridgeError(bridgeError))
            : "请求已发送但结果未知，为防止重复新建已冻结门禁。请核对结果或点击重试原请求。";
          finishSourceMutation(gate, false, errStr);
          setSourceError(errStr);
        }
      } catch (mutationErr) {
        finishSourceMutation(gate, false, formatBridgeError(mutationErr));
        if (currentGen === mutationGeneration.current && initialTaskId === activeTaskIdRef.current) {
          setSourceError(formatBridgeError(mutationErr));
        }
      } finally {
        if (currentGen === mutationGeneration.current && initialTaskId === activeTaskIdRef.current) {
          setMutationBusy(false);
        }
      }
    } catch (lockError) {
      if (activeTaskIdRef.current === taskId) {
        setMutationBusy(false);
        setSourceError(formatBridgeError(lockError));
      }
    } finally {
      releaseSourceMutationInFlight(taskId);
    }
  }

  async function onConfirmHistoricalSuccessor(confirmed: ConfirmedDownloadTaskDraft) {
    const action = sourcePage ? taskHistoricalAction(sourcePage.task_status) : "unavailable";
    if (!successorConfirmation || !sourcePage || sourcePage.task_id !== taskId || pendingGate
      || getPendingReacquireRecord(taskId) || isStorageCorrupted().corrupted || isSourceLocked || sourceBusy || sourceError
      || isSourceMutationStorageCorrupted()
      || action !== "create_successor"
      || (!successorConfirmation.burstSource && !newTaskPolicyReady)
      || successorConfirmation.epoch !== sourceMutationEpoch(taskId)
      || confirmed.sourceBatchId !== sourcePage.source_batch_id
      || confirmed.sourceBatchRevision !== sourcePage.task_revision) {
      throw new Error("后继任务参数校验失败或环境已变更，已安全阻止创建。");
    }
    const itemIds = confirmed.selectedProductIds as [string, ...string[]];
    const uniqueItemIds = Array.from(new Set(itemIds));
    if (uniqueItemIds.length !== itemIds.length || uniqueItemIds.length === 0 || uniqueItemIds.length > 250) {
      throw new Error("所选产品数量无效或存在重复。");
    }
    if (!VALID_DOWNLOAD_LAYOUTS.has(confirmed.layout)
      || !Number.isSafeInteger(confirmed.concurrency)
      || confirmed.concurrency < 1 || confirmed.concurrency > 8
      || (successorConfirmation.burstSource && confirmed.layout !== "direct")) {
      throw new Error("后继任务下载布局或并发参数无效。");
    }

    if (!acquireSourceMutationInFlight(taskId)) {
      throw new Error("当前任务已有正在进行的来源操作，请勿重复点击。");
    }
    try {
      setMutationBusy(true);
      setSourceError(null);
      setMutationNotice(null);
      const currentGen = ++mutationGeneration.current;
      const initialTaskId = taskId;
      const newTaskId = uniqueId("download-task");
      const now = new Date().toISOString();

      const frozenMutation: FrozenSourceMutation = successorConfirmation.burstSource
        ? {
            kind: "burst_successor",
            request: {
              request_id: uniqueId("burst-task-source-successor"),
              source_task_id: taskId,
              expected_source_task_revision: confirmed.sourceBatchRevision,
              selected_product_ids: [...itemIds],
              new_task_id: newTaskId,
              output_root: confirmed.outputRoot,
              layout: "direct",
              concurrency: confirmed.concurrency,
              created_at: now,
            },
          }
        : {
            kind: "successor_source_batch",
            request: {
              source_task_id: taskId,
              expected_source_task_revision: confirmed.sourceBatchRevision,
              new_task_id: newTaskId,
              item_ids: [...itemIds] as [string, ...string[]],
              output_root: confirmed.outputRoot,
              layout: confirmed.layout,
              concurrency: confirmed.concurrency,
              created_at: now,
            },
          };

      const gate = beginSourceMutation({
        sourceTaskId: taskId,
        targetTaskId: newTaskId,
        sourceBatchId: sourcePage.source_batch_id,
        itemIds: [...itemIds],
        minRevision: 0,
        pending: true,
        frozenMutation,
      });

      if (!gate) {
        setMutationBusy(false);
        throw new Error("无法持久化来源门禁或存储不可用，已安全阻止发送。");
      }

      try {
        let bridgeResult: { ok: boolean; [key: string]: unknown } | null = null;
        let bridgeError: unknown = null;
        try {
          bridgeResult = await executeFrozenMutation(gate.frozenMutation!);
        } catch (err) {
          bridgeError = err;
        }

        const isDirectOk = bridgeResult?.ok === true;
        if (isDirectOk) {
          noteSourceMutationSuccess(gate);
        }

        const membershipFound = await verifyMembership(gate).catch(() => false);

        // Late response protection: if user switched task or component unmounted,
        // retain gate in unconfirmed state and never touch new task UI.
        if (currentGen !== mutationGeneration.current || initialTaskId !== activeTaskIdRef.current) {
          finishSourceMutation(
            gate,
            false,
            isDirectOk ? undefined : (bridgeResult ? formatBridgeError(bridgeResult) : formatBridgeError(bridgeError)),
          );
          return;
        }

        if (isDirectOk && membershipFound) {
          const finished = finishSourceMutation(gate, true);
          if (!finished) throw new Error("未能安全清除门禁记录（存储写入失败）。");
          setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "clear_selection" }));
          setMutationNotice("已从同一来源检索批次创建新任务；原任务保持终态。");
          setSuccessorConfirmation(null);
          await loadSourcePage(sourcePage.page_index, sourcePage.page_size);
          void Promise.resolve().then(() => onHistoricalMutationApplied?.()).catch(console.warn);
        } else {
          const errStr = !isDirectOk
            ? (bridgeResult ? formatBridgeError(bridgeResult) : formatBridgeError(bridgeError))
            : "后继任务创建结果未确认，为防止重复新建已冻结门禁。请核对结果或点击重试原请求。";
          finishSourceMutation(gate, false, errStr);
          throw new Error(errStr);
        }
      } catch (err) {
        finishSourceMutation(gate, false, formatBridgeError(err));
        if (currentGen === mutationGeneration.current && initialTaskId === activeTaskIdRef.current) {
          setSourceError(formatBridgeError(err));
        }
        throw err;
      } finally {
        if (currentGen === mutationGeneration.current && initialTaskId === activeTaskIdRef.current) {
          setMutationBusy(false);
        }
      }
    } catch (lockError) {
      if (activeTaskIdRef.current === taskId) {
        setMutationBusy(false);
        setSourceError(formatBridgeError(lockError));
      }
      throw lockError;
    } finally {
      releaseSourceMutationInFlight(taskId);
    }
  }

  async function handleRetryFrozenGate(gate: SourceMutationGate) {
    if (gate.sourceTaskId !== taskId || !gate.frozenMutation || isSourceLocked || isSourceMutationStorageCorrupted()) return;
    if (!acquireSourceMutationInFlight(taskId)) {
      setSourceError("未能获取同会话锁，重试已阻止。");
      return;
    }
    try {

      const staged = stageRetrySourceMutation(gate);
      if (!staged || !staged.frozenMutation) {
        setSourceError("门禁存储不可用或无法以同一身份重新暂存请求。");
        return;
      }

      setMutationBusy(true);
      setSourceError(null);
      setMutationNotice(null);
      const currentGen = ++mutationGeneration.current;
      const initialTaskId = taskId;

      try {
        let bridgeResult: { ok: boolean; [key: string]: unknown } | null = null;
        let bridgeError: unknown = null;
        try {
          bridgeResult = await executeFrozenMutation(staged.frozenMutation);
        } catch (err) {
          bridgeError = err;
        }

        const isDirectOk = bridgeResult?.ok === true;
        if (isDirectOk) {
          noteSourceMutationSuccess(staged);
        }

        const membershipFound = isNisarSourceMutation(staged)
          ? nisarDirectResponseContainsTarget(staged, bridgeResult)
          : await verifyMembership(staged).catch(() => false);

        if (currentGen !== mutationGeneration.current || initialTaskId !== activeTaskIdRef.current) {
          finishSourceMutation(
            staged,
            false,
            isDirectOk ? undefined : (bridgeResult ? formatBridgeError(bridgeResult) : formatBridgeError(bridgeError)),
          );
          return;
        }

        if (isDirectOk && membershipFound) {
          const finished = finishSourceMutation(staged, true);
          if (!finished) {
            setSourceError("未能安全清除门禁记录（存储写入失败）。");
            return;
          }
          setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "clear_selection" }));
          setMutationNotice(isNisarSourceMutation(staged)
            ? `原请求已成功。目标任务：${staged.targetTaskId}；如需启动后继任务，请在下载任务中操作。`
            : `任务 ${staged.targetTaskId} 成员已确认成功（重试成功）。`);
          setSuccessorConfirmation(null);
          await loadSourcePage(sourcePage?.page_index ?? 0, sourcePage?.page_size ?? 24);
          void Promise.resolve().then(() => onHistoricalMutationApplied?.()).catch(console.warn);
        } else {
          const errStr = !isDirectOk
            ? (bridgeResult ? formatBridgeError(bridgeResult) : formatBridgeError(bridgeError))
            : "重试结果仍未确认。请核对结果或稍后重试。";
          finishSourceMutation(staged, false, errStr);
          setSourceError(errStr);
        }
      } catch (unknownErr) {
        finishSourceMutation(staged, false, formatBridgeError(unknownErr));
        if (currentGen === mutationGeneration.current && initialTaskId === activeTaskIdRef.current) {
          setSourceError(formatBridgeError(unknownErr));
        }
      } finally {
        if (currentGen === mutationGeneration.current && initialTaskId === activeTaskIdRef.current) {
          setMutationBusy(false);
        }
      }
    } catch (lockError) {
      if (activeTaskIdRef.current === taskId) {
        setMutationBusy(false);
        setSourceError(formatBridgeError(lockError));
      }
    } finally {
      releaseSourceMutationInFlight(taskId);
    }
  }

  async function handleCheckOutcome(gate: SourceMutationGate) {
    if (isNisarSourceMutation(gate)) return;
    if (gate.sourceTaskId !== taskId || isSourceLocked) return;
    if (!acquireSourceMutationInFlight(taskId)) {
      setSourceError("未能获取同会话锁，核对已阻止。");
      return;
    }
    const checkTaskId = taskId;
    let checkGeneration = mutationGeneration.current;
    try {
      checkGeneration = ++mutationGeneration.current;
      setMutationBusy(true);
      setSourceError(null);
      const confirmed = await verifyMembership(gate).catch(() => false);
      if (checkGeneration !== mutationGeneration.current || checkTaskId !== activeTaskIdRef.current) {
        return;
      }
      if (confirmed) {
        if (gate.directSuccess === true) {
          if (!finishSourceMutation(gate, true)) {
            setSourceError("未能安全清除门禁记录（存储写入失败）。");
            return;
          }
          setSourceError(null);
          setSuccessorConfirmation(null);
          setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "clear_selection" }));
          void loadSourcePage(sourcePage?.page_index ?? 0, sourcePage?.page_size ?? 24);
          void Promise.resolve().then(() => onHistoricalMutationApplied?.()).catch(console.warn);
          setMutationNotice(`已确认目标任务 ${gate.targetTaskId} 包含所选产品，门禁已解除。`);
        } else {
          setMutationNotice(
            `目标任务 ${gate.targetTaskId} 当前包含所选产品。由于此前未获得直接成功确认（写入归属未证明），门禁保持保留以防重复写入；如需重试请使用重试原请求。`,
          );
        }
      } else {
        setSourceError("核对未确认：目标任务中尚未包含全部所选产品。");
      }
    } finally {
      releaseSourceMutationInFlight(checkTaskId);
      if (checkGeneration === mutationGeneration.current && checkTaskId === activeTaskIdRef.current) {
        setMutationBusy(false);
      }
    }
  }

  /**
   * Downloader D3: download a reused item again with strict safety protections:
   * 1. Check storage corruption / fail closed (no unsubstantiated forced override).
   * 2. Synchronous in-flight lock on same WebView to protect against double clicks.
   * 3. Reuse same frozen request on retry; freeze & stage complete request before sending.
   * 4. Generation check for switching task / unmount isolation.
   * 5. Clean up only upon direct ok:true response containing target task_id in core.tasks.
   * 6. Retain pending record on network error, ok:false, switch task, or unmount.
   */
  async function confirmReacquire() {
    if (!reacquireTarget || itemMutationId !== null) return;
    if (reacquireTarget.taskId !== taskId) {
      setReacquireTarget(null);
      setReacquireError("该确认已过期，请重新选择文件。");
      return;
    }
    const { itemId, displayName } = reacquireTarget;

    const { corrupted, reason } = isStorageCorrupted();
    if (corrupted) {
      setReacquireError(`本地存储损坏（${reason}）。已严格锁定重新下载，禁止盲目操作。`);
      return;
    }

    if (!tryAcquireInFlightLock(taskId)) {
      setReacquireError("该任务已有正在进行的重新下载请求，请勿重复点击。");
      return;
    }

    try {
      const pending = getPendingReacquireRecord(taskId);
      if (!pending && reacquireTarget.epoch !== getSourceSuccessEpoch(taskId)) {
        setReacquireTarget(null);
        setReacquireError("该确认已过期，请重新选择文件。");
        return;
      }
      if (!pending && (getSourceMutationGate(taskId) || isSourceMutationStorageCorrupted())) {
        setReacquireError("当前任务存在未确认的来源请求或来源门禁不可用，请先处理来源请求。");
        return;
      }
      let frozenRequest: FrozenReacquireRequest;
      let targetTaskId: string;

      if (pending) {
        if (pending.itemId !== itemId) {
          setReacquireError(
            `当前任务存在尚未确认的重新下载请求 (文件: ${pending.displayName}，目标 ID: ${pending.targetTaskId})。` +
              "为防止重复下载或状态错乱，不允许切换目标发起盲重试，必须重试原目标或等待系统确认。",
          );
          return;
        }
        frozenRequest = pending.request;
        targetTaskId = pending.targetTaskId;
      } else {
        const now = new Date().toISOString();
        if (taskMutable) {
          const eventId = uniqueId("local-reacquire");
          frozenRequest = {
            kind: "active",
            task_id: taskId,
            expected_revision: revision,
            command: { kind: "reacquire_items", item_ids: [itemId] },
            event_id: eventId,
            occurred_at: now,
          };
          targetTaskId = taskId;
        } else {
          const newTaskId = uniqueId("download-task");
          frozenRequest = {
            kind: "terminal",
            source_task_id: taskId,
            expected_source_task_revision: revision,
            new_task_id: newTaskId,
            item_ids: [itemId],
            output_root: outputDir,
            layout: downloadLayout,
            concurrency: 1,
            created_at: now,
            reacquire_item_ids: [itemId],
          };
          targetTaskId = newTaskId;
        }

        const stageResult = stageReacquireRequest({
          sourceTaskId: taskId,
          itemId,
          displayName,
          targetTaskId,
          request: frozenRequest,
          status: "pending_confirmation",
          createdAt: now,
        });

        if (!stageResult.ok) {
          setReacquireError(stageResult.error);
          return;
        }
      }

      const currentGeneration = reacquireGeneration.current;
      const entryEpoch = getSourceSuccessEpoch(taskId);

      setItemMutationId(itemId);
      setReacquireError(null);
      setMutationNotice(null);

      let callResult: DesktopCoreRestoreSnapshot | null = null;
      let bridgeException: unknown = null;

      try {
        if (frozenRequest.kind === "active") {
          callResult = await updateApplicationDownloadTask({
            task_id: frozenRequest.task_id,
            expected_revision: frozenRequest.expected_revision,
            command: frozenRequest.command,
            event_id: frozenRequest.event_id,
            occurred_at: frozenRequest.occurred_at,
          });
        } else {
          callResult = await createApplicationTaskFromHistoricalSourceBatch({
            source_task_id: frozenRequest.source_task_id,
            expected_source_task_revision: frozenRequest.expected_source_task_revision,
            new_task_id: frozenRequest.new_task_id,
            item_ids: frozenRequest.item_ids,
            output_root: frozenRequest.output_root,
            layout: frozenRequest.layout,
            concurrency: frozenRequest.concurrency,
            created_at: frozenRequest.created_at,
            reacquire_item_ids: frozenRequest.reacquire_item_ids,
          });
        }
      } catch (err) {
        bridgeException = err;
      }

      // 切 task 或卸载后代际隔离：立刻丢弃，不污染新任务
      if (currentGeneration !== reacquireGeneration.current || currentReacquireTask.current !== taskId) {
        return;
      }

      // 成功代际检查：若已过时则放弃旧确认
      if (getSourceSuccessEpoch(taskId) !== entryEpoch) {
        setItemMutationId(null);
        return;
      }

      setItemMutationId(null);

      if (bridgeException !== null) {
        const errText = formatBridgeError(bridgeException);
        updateReacquireLastError(taskId, errText);
        setReacquireError(
          `重新下载结果待确认：请求已发送，但未收到明确确认（${errText}）。` +
            `为防止创建重复任务，原请求已保留；可点击“重新下载”重试确认原目标（目标任务ID: ${targetTaskId}）。`,
        );
        return;
      }

      if (!callResult || !callResult.ok) {
        const errText = formatBridgeError(callResult);
        updateReacquireLastError(taskId, errText);
        setReacquireError(
          `重新下载结果待确认：服务返回失败（${errText}）。` +
            `为防止盲目生成新任务，原请求已保留；可按同一请求重试原目标（目标任务ID: ${targetTaskId}）。` +
            (frozenRequest.kind === "active" ? "（注：活动任务路径因版本或状态可能不保证重放成功）" : ""),
        );
        return;
      }

      const commitResult = tryCommitReacquireSuccess(taskId, frozenRequest, callResult);
      if (!commitResult.committed) {
        const reason = commitResult.reason ?? "服务响应未能确认目标任务状态";
        updateReacquireLastError(taskId, reason);
        setReacquireError(
          `重新下载结果待确认：已收到服务响应，但未能完成清理确认（${reason}）。` +
            `原请求已安全保留，可点击重试以再次确认原目标（目标任务ID: ${targetTaskId}）。`,
        );
        return;
      }

      setReacquireTarget(null);
      setMutationNotice(
        frozenRequest.kind === "active"
          ? "已请求重新下载：" + displayName
          : "已创建重新下载任务：" + displayName + "；原任务保持终态。",
      );
      void Promise.resolve().then(() => onHistoricalMutationApplied?.()).catch(console.warn);
    } finally {
      releaseInFlightLock(taskId);
    }
  }

  async function retryNisarItem(itemId: string) {
    const item = page?.items.find((candidate) => candidate.itemId === itemId);
    if (
      !item
      || !taskMutable
      || !isNisarSubsetItem(item)
      || !["failed", "interrupted"].includes(item.status)
      || !item.retryable
      || itemMutationId !== null
    ) return;
    setItemMutationId(itemId);
    setSourceError(null);
    setMutationNotice(null);
    try {
      const now = new Date().toISOString();
      const result = await updateApplicationDownloadTask({
        task_id: taskId,
        expected_revision: revision,
        command: { kind: "retry_items", item_ids: [itemId] },
        event_id: uniqueId("nisar-subset-retry"),
        occurred_at: now,
      });
      if (!result.ok) throw new Error(formatBridgeError(result));
      setMutationNotice("已提交影像重试：" + item.displayName);
      onHistoricalMutationApplied?.();
    } catch (retryError) {
      setSourceError("重试影像失败：" + formatBridgeError(retryError));
    } finally {
      setItemMutationId(null);
    }
  }

  useEffect(() => {
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (drawerRef.current && (!previousFocus || !drawerRef.current.contains(previousFocus))) {
      drawerRef.current.focus();
    }
    return () => {
      if (previousFocus && previousFocus.isConnected) {
        previousFocus.focus();
      }
    };
  }, []);

  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (
        event.key === "Escape"
        && !event.defaultPrevented
        && !event.isComposing
        && !successorConfirmation
        && !reacquireTarget
        && !appendSession
      ) {
        onClose();
      }
    };
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [onClose, successorConfirmation, reacquireTarget, appendSession]);

  const tabs: Array<[Tab, string]> = [
    ["task_info", "概览"],
    ["download_status", "下载文件"],
    ["search_results", "来源结果"],
    ["events", "事件"],
  ];
  const nisarSubsetProgress = isNisarSubsetProgressTask(taskType, page);
  const percent = nisarSubsetProgress ? null : taskPercent(transferredBytes, totalBytes);
  const eta = nisarSubsetProgress ? null : etaSeconds(transferredBytes, totalBytes, bytesPerSecond);
  const restrictedAccess = page?.items
    .filter(taskItemHasCurrentFailure)
    .map((item) => asfRestrictedAccessPresentation(datasetLabel, item.failure?.code))
    .find((value) => value !== null) ?? null;
  const visibleStatus = restrictedAccess?.badge ?? status;
  const boundedConcurrency = concurrency === null
    ? "—"
    : Math.max(1, Math.trunc(concurrency) || 1);
  const failedItems = page?.items.filter(taskItemHasCurrentFailure).slice(0, 3) ?? [];

  const sourceAction = taskHistoricalAction(sourcePage?.task_status ?? status);
  const isBurstSource = datasetLabel.trim().toUpperCase() === "SLC-BURST"
    || (sourcePage?.items.some((entry) => (
      String(entry.item.metadata.dataset ?? "").trim().toUpperCase() === "SLC-BURST"
    )) ?? false);

  let disabledReason: string | null = null;
  if (isSourceMutationStorageCorrupted() || isStorageCorrupted().corrupted) {
    disabledReason = "来源门禁本地存储已损坏或不可用，已安全阻止新操作。";
  } else if (pendingGate) {
    disabledReason = "当前任务存在尚未确认的来源请求，已暂停新操作。请核对或重试原请求。";
  } else if (hasPendingNisarRecovery(taskId)) {
    disabledReason = "当前任务存在待确认或不可读取的 NISAR 恢复记录。请在经典工作台的原归档任务中选择“继续”核对。";
  } else if (getPendingReacquireRecord(taskId)) {
    disabledReason = "当前任务存在尚未确认的重新下载请求，请先核对或重试原请求。";
  } else if (isSourceMutationInFlight(taskId)) {
    disabledReason = "正在提交来源请求…";
  } else if (historical.selectedIds.length > 250) {
    disabledReason = "单次提交不能超过 250 个产品。";
  } else if (sourceAction === "unavailable") {
    disabledReason = "当前任务状态不允许追加产品或创建后继任务。";
  } else if (sourceAction === "create_successor" && !isBurstSource && !newTaskPolicyReady) {
    disabledReason = "应用级 SAR 下载设置尚未成功加载，无法创建新的历史后继任务。";
  }

  return (
    <div className="fixed inset-0 z-[1200] pointer-events-none" data-task-workbench-backdrop>
      <section
        ref={drawerRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="false"
        aria-label={`${title} 任务详情`}
        className="pointer-events-auto absolute inset-y-0 right-0 flex h-full w-[min(88vw,1080px)] min-h-0 flex-col overflow-hidden border-l border-sp-line bg-background shadow-sp-modal outline-none"
        data-task-detail-workbench
      >
        <header className="flex items-start justify-between gap-4 border-b bg-muted/25 px-5 py-4">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="truncate text-base font-semibold">{title}</h2>
              <Badge variant="outline">{visibleStatus}</Badge>
              <span className="font-mono text-[11px] text-muted-foreground">rev {revision}</span>
            </div>
            <div className="mt-1 truncate text-xs text-muted-foreground" title={outputDir}>
              {taskId} · {outputDir || "输出目录尚未建立"}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <Button type="button" variant="outline" size="sm" onClick={onOpenOutput}>
              <FolderOpen className="h-4 w-4" />
              打开输出目录
            </Button>
            <Button type="button" variant="ghost" size="icon" aria-label="关闭任务详情" onClick={onClose}>
              <X className="h-4 w-4" />
            </Button>
          </div>
        </header>

        <div className="grid gap-3 border-b bg-muted/10 px-5 py-3 text-xs lg:grid-cols-[minmax(280px,1.25fr)_minmax(360px,1.7fr)_minmax(240px,1fr)]">
          <section className="min-w-0 rounded-sp-card border bg-card px-3 py-2.5" aria-label="任务总体进度">
            <div className="flex items-center justify-between gap-3">
              <div className="flex min-w-0 items-center gap-2">
                <Gauge className="h-4 w-4 shrink-0 text-primary" />
                <span className="truncate font-medium" title={datasetLabel}>{datasetLabel || "下载任务"}</span>
              </div>
              <span className="shrink-0 text-lg font-semibold tabular-nums">{percent === null ? "—" : `${Math.round(percent)}%`}</span>
            </div>
            {nisarSubsetProgress ? (
              <div className="mt-2">
                <NisarSubsetTransferFacts
                  sourceTotalBytes={sourceTotalBytes}
                  networkBytesReceived={networkBytesReceived}
                  reusableVerifiedBytes={reusableVerifiedBytes}
                  reusedVerifiedBytes={reusedVerifiedBytes}
                />
                <div className="mt-1.5 text-right text-[11px] text-muted-foreground">{visibleStatus}</div>
              </div>
            ) : (
              <>
                <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted" aria-hidden="true">
                  <div className="h-full rounded-full bg-primary" style={{ width: `${percent ?? 0}%` }} />
                </div>
                <div className="mt-2 flex min-w-0 justify-between gap-3 text-[11px] text-muted-foreground">
                  <span className="truncate" title={totalBytes === null ? formatBytes(transferredBytes) : `${formatBytes(transferredBytes)} / ${formatBytes(totalBytes)}`}>
                    {totalBytes === null ? `${formatBytes(transferredBytes)} 已传输` : `${formatBytes(transferredBytes)} / ${formatBytes(totalBytes)}`}
                  </span>
                  <span className="shrink-0">{visibleStatus}</span>
                </div>
              </>
            )}
          </section>

          <section className="grid grid-cols-5 overflow-hidden rounded-sp-card border bg-card" aria-label="任务项目状态">
            {[
              ["完成", completedCount], ["活动", activeCount], ["等待", waitingCount],
              ["暂停", pausedCount], ["失败", failedCount],
            ].map(([label, value], index) => (
              <div key={label} className={index === 0 ? "px-2 py-2.5 text-center" : "border-l px-2 py-2.5 text-center"}>
                <div className="text-[10px] text-muted-foreground">{label}</div>
                <div className="mt-1 text-base font-semibold tabular-nums">{value}</div>
              </div>
            ))}
          </section>

          <section className="grid grid-cols-2 gap-x-4 gap-y-2 rounded-sp-card border bg-card px-3 py-2.5" aria-label="任务传输状态">
            <div>
              <div className="flex items-center gap-1 text-[10px] text-muted-foreground"><Activity className="h-3 w-3" /> 当前速率</div>
              <div className="mt-1 font-medium tabular-nums">{formatBytes(bytesPerSecond)}/s</div>
            </div>
            <div>
              <div className="text-[10px] text-muted-foreground">真实传输</div>
              <div className="mt-1 font-medium tabular-nums">{formatElapsed(transferActiveMillis)}</div>
            </div>
            <div>
              <div className="text-[10px] text-muted-foreground">并发</div>
              <div className="mt-1 font-medium tabular-nums">{boundedConcurrency}</div>
            </div>
            <div>
              <div className="text-[10px] text-muted-foreground">预计剩余</div>
              <div className="mt-1 font-medium tabular-nums">{nisarSubsetProgress ? "不可用" : eta === null ? "—" : formatElapsed(eta * 1_000)}</div>
            </div>
          </section>
        </div>

        {page && /Landsat|Sentinel-2/i.test(datasetLabel) && <OpticalDeliveryReport taskId={taskId} fileCount={page.totalCount} />}
        <nav className="flex gap-1 border-b px-5 py-2" aria-label="任务详情分区">
          {tabs.map(([id, label]) => (
            <Button key={id} type="button" size="sm" variant={tab === id ? "default" : "ghost"} onClick={() => setTab(id)}>
              {label}
            </Button>
          ))}
        </nav>

        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          {error && <div role="alert" className="mb-3 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">{error}</div>}
          {sourceError && <div role="alert" className="mb-3 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">{sourceError}</div>}
          {mutationNotice && <div role="status" className="mb-3 rounded-lg border border-primary/25 bg-primary/5 px-3 py-2 text-sm">{mutationNotice}</div>}
          {failedItems.length > 0 && (
            <section role="alert" className="mb-3 rounded-xl border border-destructive/30 bg-destructive/5 p-4" data-task-failure-summary>
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div>
                  <div className="font-semibold text-destructive">{failedItems.length} 个文件执行失败</div>
                  <div className="mt-1 text-xs text-muted-foreground">失败原因直接显示在概览；可进入“下载文件”查看每个文件的完整诊断。</div>
                </div>
                <Button type="button" size="sm" variant="outline" onClick={() => setTab("download_status")}>查看失败文件</Button>
              </div>
              <div className="mt-3 grid gap-2 lg:grid-cols-3">
                {failedItems.map((item) => (
                  <div key={item.itemId} className="min-w-0 rounded-lg border bg-background/80 px-3 py-2 text-xs">
                    <div className="truncate font-medium" title={item.displayName}>{item.displayName}</div>
                    <div className="mt-1 text-destructive">{item.failure!.reasonZh}</div>
                    {item.failure!.stage !== "未知阶段" && (
                      <div className="mt-1 text-muted-foreground">失败阶段：{item.failure!.stage}</div>
                    )}
                    <div className="mt-1 font-mono text-[10px] text-muted-foreground">{item.failure!.code}</div>
                  </div>
                ))}
              </div>
            </section>
          )}
          {restrictedAccess && (
            <section role="alert" className="mb-3 rounded-xl border border-warning/40 bg-warning/10 px-4 py-3 text-sm">
              <div className="flex items-start gap-3">
                <LockKeyhole className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
                <div className="min-w-0 flex-1">
                  <div className="font-medium">{restrictedAccess.title}</div>
                  <p className="mt-1 leading-5 text-muted-foreground">{restrictedAccess.message}</p>
                  <Button
                    type="button"
                    variant="ghost"
                    className="mt-1 h-auto px-0 py-0"
                    onClick={() => onOpenRestrictedSupport(restrictedAccess.supportUrl)}
                  >
                    <ExternalLink className="h-3.5 w-3.5" />
                    {restrictedAccess.supportLabel}
                  </Button>
                </div>
              </div>
            </section>
          )}
          {tab === "search_results" && (
            <>
            {pendingGate && !pendingGate.pending && (
              <div role="alert" className="mb-3 space-y-2 rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm">
                <p className="font-medium text-destructive">未确认结果，新来源操作已暂停。目标任务：{pendingGate.targetTaskId}</p>
                {pendingGate.error && <p className="text-xs text-destructive">{pendingGate.error}</p>}
                <SourceMutationStatusNotice result={mutationStatus} language={locale} />
                <div className="flex flex-wrap items-center gap-3">
                  {!isNisarSourceMutation(pendingGate) && <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={isSourceLocked}
                    onClick={() => void handleCheckOutcome(pendingGate)}
                  >
                    核对结果
                  </Button>}
                  {pendingGate.frozenMutation && (
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={isSourceLocked}
                      onClick={() => void handleRetryFrozenGate(pendingGate)}
                    >
                      重试原请求
                    </Button>
                  )}
                </div>
              </div>
            )}
            {page?.items.some(isNisarSubsetItem) && (
              <section className="mb-3 rounded-lg border bg-card p-3" aria-label="任务冻结 AOI">
                <div className="mb-2 flex flex-wrap items-center gap-2 text-xs">
                  <LockKeyhole className="h-3.5 w-3.5" />
                  <span className="font-semibold">任务冻结 AOI（Task Frozen AOI）</span>
                  <Button type="button" variant="outline" size="sm" disabled={isSourceLocked || !!pendingGate
                    || !!getPendingReacquireRecord(taskId) || hasPendingNisarRecovery(taskId)
                    || isStorageCorrupted().corrupted || isSourceMutationStorageCorrupted()}
                    onClick={() => void openNisarAppendWorkspace()}>追加数据</Button>
                  {frozenSubsetAoi && <span>Frequency {frozenSubsetAoi.frequency} · {frozenSubsetAoi.polarizations.join(" / ")}</span>}
                </div>
                {frozenSubsetAoi ? (
                  <div className="h-52 overflow-hidden rounded-md">
                    <AoiDrawMap bbox={frozenSubsetAoi.bbox} requestedGeometry={frozenSubsetAoi.geometry}
                      drawActive={false} minHeight={208} />
                  </div>
                ) : <p className="text-xs text-muted-foreground">任务保存的裁剪范围暂不可用。</p>}
              </section>
            )}
            <div className={historicalPreview ? "grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(280px,0.8fr)]" : ""}>
              <TaskSourceBatchResults
                page={sourcePage}
                selectedIds={historical.selectedIds}
                filter={historical.filter}
                sort={historical.sort}
                busy={sourceBusy}
                mutationBusy={isSourceLocked}
                disabledReason={disabledReason}
                onToggleItem={(itemId) => setHistorical((current) =>
                  updateTaskHistoricalUiState(current, { kind: "toggle_item", itemId }))}
                onFilterChange={(filter) => setHistorical((current) =>
                  updateTaskHistoricalUiState(current, { kind: "set_filter", filter }))}
                onSortChange={(sort) => setHistorical((current) =>
                  updateTaskHistoricalUiState(current, { kind: "set_sort", sort }))}
                onPageIndexChange={(pageIndex) => {
                  setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "set_page", pageIndex }));
                  void loadSourcePage(pageIndex, historical.pageSize);
                }}
                onSubmit={(itemIds) => void submitHistoricalSelection(itemIds)}
                onPreview={(item) => { if (sourcePage) setHistoricalPreview({ batchId: sourcePage.source_batch_id,
                  itemId: item.item_id, productLabel: item.display_name }); }}
              />
              {historicalPreview && <div className="h-[420px] min-w-0 overflow-hidden rounded-md border lg:sticky lg:top-0">
                <FrozenProductPreview {...historicalPreview} onClose={() => setHistoricalPreview(null)} />
              </div>}
            </div>
            </>
          )}
          {tab === "task_info" && (
            <div className="text-sm">
              <section className="rounded-xl border bg-card p-4" data-task-overview-summary>
                <div className="flex items-end justify-between gap-4">
                  <div className="min-w-0">
                    <div className="text-[11px] text-muted-foreground">{nisarSubsetProgress ? "裁剪传输事实" : "总体进度"}</div>
                    <div className="mt-1 truncate text-base font-semibold tabular-nums">
                      {nisarSubsetProgress
                        ? `网络已接收 ${subsetFactBytes(networkBytesReceived)}`
                        : totalBytes === null
                        ? formatBytes(transferredBytes)
                        : `${formatBytes(transferredBytes)} / ${formatBytes(totalBytes)}`}
                    </div>
                  </div>
                  <div className="shrink-0 text-3xl font-semibold tabular-nums" data-task-overview-progress>
                    {percent === null ? "—" : `${Math.round(percent)}%`}
                  </div>
                </div>
                {!nisarSubsetProgress && (
                  <div className="mt-3 h-2 overflow-hidden rounded-full bg-muted" aria-hidden="true">
                    <div
                      className="h-full rounded-full bg-primary transition-[width] duration-200"
                      style={{ width: `${percent ?? 0}%` }}
                    />
                  </div>
                )}
                {nisarSubsetProgress && (
                  <div className="mt-4 rounded-lg border bg-muted/15 p-3">
                    <NisarSubsetTransferFacts
                      sourceTotalBytes={sourceTotalBytes}
                      networkBytesReceived={networkBytesReceived}
                      reusableVerifiedBytes={reusableVerifiedBytes}
                      reusedVerifiedBytes={reusedVerifiedBytes}
                    />
                  </div>
                )}
                <dl className="mt-4 grid grid-cols-2 gap-x-4 gap-y-3 text-xs sm:grid-cols-4">
                  <div className="min-w-0">
                    <dt className="text-muted-foreground">数据集</dt>
                    <dd className="mt-0.5 truncate font-medium" title={datasetLabel}>{datasetLabel || "—"}</dd>
                  </div>
                  <div>
                    <dt className="text-muted-foreground">当前速率</dt>
                    <dd className="mt-0.5 font-medium tabular-nums">{formatBytes(bytesPerSecond)}/s</dd>
                  </div>
                  <div data-task-overview-eta>
                    <dt className="text-muted-foreground">预计剩余</dt>
                    <dd className="mt-0.5 font-medium tabular-nums">{eta === null ? "不可用" : formatElapsed(eta * 1_000)}</dd>
                  </div>
                  <div>
                    <dt className="text-muted-foreground">真实传输用时</dt>
                    <dd className="mt-0.5 font-medium tabular-nums">{formatElapsed(transferActiveMillis)}</dd>
                  </div>
                  <div>
                    <dt className="text-muted-foreground">并发</dt>
                    <dd className="mt-0.5 font-medium tabular-nums">并发 {boundedConcurrency}</dd>
                  </div>
                  <div>
                    <dt className="text-muted-foreground">断点续传</dt>
                    <dd className="mt-0.5 font-medium">{resumeSupported ? "支持 .part" : "不支持"}</dd>
                  </div>
                  <div>
                    <dt className="text-muted-foreground">生命周期</dt>
                    <dd className="mt-0.5 font-medium">{visibleStatus}</dd>
                  </div>
                </dl>
                <div className="mt-4 grid gap-1 border-t pt-3 text-[11px] text-muted-foreground sm:grid-cols-[auto_minmax(0,1fr)]">
                  <span className="font-mono">{taskId}</span>
                  <span className="truncate sm:text-right" title={outputDir}>{outputDir || "输出目录尚未建立"}</span>
                </div>
              </section>

            </div>
          )}
          {tab === "download_status" && (
            page ? <TaskPageDetail page={page} busy={pageBusy} onPageIndexChange={onPageIndexChange}
              nisarSubsetProgress={nisarSubsetProgress}
              renderItemDetails={(item) => <>
                {presentLocalReuse(item.localReuse, item.status)?.offerReacquire && reacquireAvailable && (
                  <div className="mt-1">
                    {(() => {
                      const pending = getPendingReacquireRecord(taskId);
                      const isPendingItem = pending && pending.itemId === item.itemId;
                      return (
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          disabled={itemMutationId !== null}
                          onClick={(e) => {
                            const triggerEl = e.currentTarget;
                            const { corrupted, reason } = isStorageCorrupted();
                            if (corrupted) {
                              setReacquireError(`本地存储已损坏或不可用（${reason}）。已锁定重新下载。`);
                              setReacquireTarget({ taskId, itemId: item.itemId, displayName: item.displayName, epoch: getSourceSuccessEpoch(taskId), anchorEl: triggerEl });
                              return;
                            }
                            if (pending) {
                              if (pending.itemId === item.itemId) {
                                const errDesc = pending.lastError ? `：${pending.lastError}` : "";
                                setReacquireError(
                                  `[待确认请求] 此前已发起重新下载，但响应结果尚未确认${errDesc}。` +
                                    `将重试原目标（目标任务ID: ${pending.targetTaskId}），不会重复生成任务。`,
                                );
                              } else {
                                setReacquireError(
                                  `当前任务已有尚未确认的重新下载请求 (文件: ${pending.displayName}，目标 ID: ${pending.targetTaskId})。` +
                                    "为防止重复下载或状态错乱，不允许切换目标，必须先确认或重试原目标任务。",
                                );
                              }
                            } else {
                              setReacquireError(null);
                            }
                            setReacquireTarget({ taskId, itemId: item.itemId, displayName: item.displayName, epoch: getSourceSuccessEpoch(taskId), anchorEl: triggerEl });
                          }}
                        >
                          {isPendingItem ? "重新下载 (待确认)" : "重新下载"}
                        </Button>
                      );
                    })()}
                  </div>
                )}
                {isNisarSubsetItem(item) && (
                  <div className="mt-2 space-y-2" data-nisar-item-lifecycle>
                    <div className="grid grid-cols-3 gap-1 sm:grid-cols-6">
                      {NISAR_LIFECYCLE_STAGES.map((stage) => (
                        <div key={stage} className="rounded border bg-card px-1.5 py-1 text-center">
                          <div className="font-medium">{stage}</div>
                          <div className="mt-0.5 text-[10px] text-muted-foreground">{nisarLifecycleStatus(item, stage)}</div>
                        </div>
                      ))}
                    </div>
                    {["failed", "interrupted"].includes(item.status) && item.retryable && (
                      taskMutable ? <Button type="button" variant="outline" size="sm" disabled={itemMutationId !== null}
                        onClick={() => void retryNisarItem(item.itemId)}>
                        {itemMutationId === item.itemId ? "重试中…" : "重试此影像"}
                      </Button>
                      : <p className="text-[11px] text-muted-foreground">此任务当前不可直接重试；已归档记录与下载断点保持保留。</p>
                    )}
                  </div>
                )}
                {taskItemHasCurrentFailure(item) && item.failure && (
                  <div className="mt-1 rounded-md border border-destructive/20 bg-destructive/5 px-2 py-1.5 text-[11px]" data-task-failure-diagnostic>
                    <div className="font-medium text-destructive">{item.failure.reasonZh}</div>
                    {item.failure.stage !== "未知阶段" && (
                      <div className="mt-0.5 text-muted-foreground">失败阶段：<span>{item.failure.stage}</span></div>
                    )}
                    <div className="mt-0.5 flex flex-wrap gap-x-3 gap-y-0.5 text-muted-foreground">
                      <span className="font-mono">错误码：{item.failure.code}</span>
                      {!nisarSubsetProgress && <span>已传输：{formatBytes(item.failure.transferredBytes)}</span>}
                      <span>尝试次数：{item.failure.attempt}</span>
                      <span>可恢复：{item.failure.retryable ? "是" : "否"}</span>
                    </div>
                  </div>
                )}
              </>} />
              : <div className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">{pageBusy ? "正在读取任务项目…" : "暂无任务项目页"}</div>
          )}
          {tab === "events" && (
            <div className="space-y-2" data-task-event-list>
              {events.length === 0 && <div className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">{eventBusy ? "正在读取事件…" : "暂无结构化事件"}</div>}
              {events.map((event) => (
                <article key={event.sequence} className="grid gap-1 rounded-lg border px-3 py-2 text-xs sm:grid-cols-[150px_80px_1fr]">
                  <time className="whitespace-nowrap font-mono text-muted-foreground">{formatEventTime(event.occurred_at)}</time>
                  <Badge variant="outline" className="w-fit">{eventBadgeLabel(event)}</Badge>
                  <div>
                    <div className="font-medium">{event.message_zh}</div>
                    <div className="mt-1 flex flex-wrap gap-2 text-muted-foreground">
                      {event.item_label && <span>{event.item_label}</span>}
                      {event.code && <span className="font-mono">{event.code}</span>}
                    </div>
                  </div>
                </article>
              ))}
              {hasMoreEvents && <Button type="button" variant="outline" size="sm" disabled={eventBusy} onClick={onLoadOlderEvents}>加载更早事件</Button>}
            </div>
          )}
        </div>
      </section>
      {appendSession && (
        <NisarSubsetWorkspace
          key={appendSession.context.task_id}
          initialDraft={appendSession.draft}
          rows={appendSession.rows}
          sourceBatchLabel={appendSession.context.source_batch_id}
          currentAoiName="任务冻结 AOI"
          mapBbox={(() => {
            const envelope = deriveWrappedAoiEnvelope(appendSession.context.frozen_aoi_geojson);
            return envelope ? continuousDisplayBbox(envelope) : null;
          })()}
          taskContext={{ taskId: appendSession.context.task_id, status: appendSession.context.task_status,
            revision: appendSession.context.task_revision, outputRoot: appendSession.outputRoot,
            successor: taskHistoricalAction(appendSession.context.task_status) === "create_successor" }}
          onChooseOutputDirectory={async () => {
            try {
              const outputRoot = await onChooseNewTaskOutput();
              if (outputRoot) setAppendSession((current) => current ? { ...current, outputRoot } : current);
            } catch (error) { setAppendError(formatBridgeError(error)); }
          }}
          reviewBusy={mutationBusy}
          reviewError={appendError}
          onPreflight={preflightNisarAppend}
          onReview={confirmNisarAppend}
          onClose={() => { appendGeneration.current += 1; setAppendSession(null); setAppendError(null); setMutationBusy(false); }}
        />
      )}
      {reacquireTarget && (
        <ReacquireConfirmation
          displayName={reacquireTarget.displayName}
          anchorEl={reacquireTarget.anchorEl}
          busy={itemMutationId !== null}
          error={reacquireError}
          onCancel={() => {
            setReacquireTarget(null);
            setReacquireError(null);
          }}
          onConfirm={() => void confirmReacquire()}
        >
          {pendingReacquire && reacquireTarget.taskId === taskId && reacquireTarget.itemId === pendingReacquire.itemId && (
            <SourceMutationStatusNotice result={reacquireStatus} language={locale} />
          )}
        </ReacquireConfirmation>
      )}
      {successorConfirmation && (
        <DownloadDestinationConfirmation
          key={`${successorConfirmation.draft.sourceBatchId}:${successorConfirmation.draft.sourceBatchRevision}:${successorConfirmation.draft.selectedProductIds.join(",")}`}
          draft={successorConfirmation.draft}
          chooseDirectory={onChooseNewTaskOutput}
          onCancel={() => setSuccessorConfirmation(null)}
          onConfirm={onConfirmHistoricalSuccessor}
        />
      )}
    </div>
  );
}
