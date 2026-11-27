import { useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useI18n } from "@/i18n";
import { SourceMutationStatusNotice } from "@/components/SourceMutationStatusNotice";
import { isSettingsModalOpen } from "@/shell/api/settingsModalLayer";
import { getSourceMutationGate, isSourceMutationStorageCorrupted } from "@/lib/taskSourceMutationGate";
import { getPendingReacquireRecord, isStorageCorrupted as isReacquireStorageCorrupted } from "@/lib/reacquireRequestGate";
import { acquireSourceOperation, releaseSourceOperation, noteSourceOperationSuccess, sourceOperationEpoch,
  sourceOperationSnapshot, subscribeSourceOperation, NISAR_RECOVERY_STORAGE_PREFIX } from "@/lib/sourceOperationCoordination";
import {
  createNisarArchivedRecoveryV2,
  createNisarSubsetSuccessorTaskV2,
  formatBridgeError,
  queryNisarArchivedRecoveryStatus,
  queryNisarSuccessorStatus,
  readNisarArchivedRecoveryContextV2,
  readNisarSubsetAppendContextV2,
} from "../lib/bridge";
import type {
  DesktopNisarAppendContextV2,
  DesktopNisarArchivedRecoveryContextV2,
  DesktopNisarArchivedRecoveryV2Request,
  DesktopNisarSuccessorTaskV2Request,
  DesktopSourceMutationStatusResult,
} from "../lib/desktopShellBridge";

const PENDING_STORAGE_KEY_PREFIX = NISAR_RECOVERY_STORAGE_PREFIX;
// Shared by component instances in this WebView, including an unmounted sender.
const inFlightSources = new Set<string>();
const sourceCompletionEpoch = new Map<string, number>();
const sourceAttempts = new Map<string, number>();
const sourceRecordVersions = new Map<string, number>();
const sourceListeners = new Set<() => void>();
function notifySources(): void {
  for (const listener of sourceListeners) {
    try { listener(); } catch { /* A listener cannot interrupt a write. */ }
  }
}
function subscribeSources(listener: () => void): () => void {
  sourceListeners.add(listener);
  return () => { sourceListeners.delete(listener); };
}
function sourceSnapshot(sourceTaskId: string): string {
  return `${sourceAttempts.get(sourceTaskId) ?? 0}:${sourceRecordVersions.get(sourceTaskId) ?? 0}:${inFlightSources.has(sourceTaskId) ? 1 : 0}`;
}
function noteSourceRecordChange(sourceTaskId: string): void {
  sourceRecordVersions.set(sourceTaskId, (sourceRecordVersions.get(sourceTaskId) ?? 0) + 1);
  notifySources();
}

function getStorageKey(sourceTaskId: string): string {
  return `${PENDING_STORAGE_KEY_PREFIX}${sourceTaskId}`;
}

export type PendingNisarRecoveryRecord =
  | {
      kind: "archived_recovery";
      sourceTaskId: string;
      targetTaskId: string;
      createdAt: string;
      outputRoot: string;
      status: "unknown";
      savedAt: number;
      request: DesktopNisarArchivedRecoveryV2Request;
    }
  | {
      kind: "fallback_recreate";
      sourceTaskId: string;
      targetTaskId: string;
      createdAt: string;
      outputRoot: string;
      originalItemsCount: number;
      status: "unknown";
      savedAt: number;
      request: DesktopNisarSuccessorTaskV2Request;
    };

export type PendingRecordLoadResult =
  | { status: "empty" }
  | { status: "valid"; record: PendingNisarRecoveryRecord }
  | { status: "corrupted"; error: string };

function isValidArchivedRecoveryRequest(
  req: unknown,
  sourceTaskId: string,
  targetTaskId: string,
  createdAt: string
): req is DesktopNisarArchivedRecoveryV2Request {
  if (!req || typeof req !== "object") return false;
  const r = req as Record<string, unknown>;
  return (
    r.source_task_id === sourceTaskId &&
    r.new_task_id === targetTaskId &&
    r.created_at === createdAt &&
    typeof r.source_item_id === "string" &&
    r.source_item_id.length > 0 &&
    typeof r.expected_source_revision === "number" &&
    Number.isSafeInteger(r.expected_source_revision) && r.expected_source_revision >= 0
  );
}

function isValidFallbackRequest(
  req: unknown,
  sourceTaskId: string,
  targetTaskId: string,
  createdAt: string,
  outputRoot: string
): req is DesktopNisarSuccessorTaskV2Request {
  if (!req || typeof req !== "object") return false;
  const r = req as Record<string, unknown>;
  return (
    r.source_task_id === sourceTaskId &&
    r.new_task_id === targetTaskId &&
    r.created_at === createdAt &&
    r.output_root === outputRoot &&
    typeof r.expected_task_revision === "number" &&
    Number.isSafeInteger(r.expected_task_revision) && r.expected_task_revision >= 0 &&
    Array.isArray(r.source_item_ids) &&
    r.source_item_ids.length > 0 &&
    r.source_item_ids.every((id) => typeof id === "string" && id.length > 0) &&
    ["direct", "sentinel1_slc", "sentinel_orbit_aux_poeorb", "sar_dataset_processing"].includes(String(r.layout)) &&
    typeof r.concurrency === "number" &&
    Number.isInteger(r.concurrency) && r.concurrency >= 1 && r.concurrency <= 8 &&
    r.recreate_original_items === true
  );
}

function loadPendingRecord(sourceTaskId: string): PendingRecordLoadResult {
  try {
    if (typeof window === "undefined" || !window.localStorage) {
      return { status: "corrupted", error: "本地存储不可用" };
    }
    const raw = window.localStorage.getItem(getStorageKey(sourceTaskId));
    if (raw === null) return { status: "empty" };

    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      return { status: "corrupted", error: "本地记录 JSON 损坏" };
    }

    if (!parsed || typeof parsed !== "object") {
      return { status: "corrupted", error: "本地记录非有效对象" };
    }

    const rec = parsed as Record<string, unknown>;
    if (rec.sourceTaskId !== sourceTaskId) {
      return { status: "corrupted", error: "源任务 ID 不匹配" };
    }
    if (typeof rec.targetTaskId !== "string" || rec.targetTaskId.length === 0) {
      return { status: "corrupted", error: "目标任务 ID 缺失" };
    }
    if (typeof rec.createdAt !== "string" || rec.createdAt.length === 0) {
      return { status: "corrupted", error: "创建时间戳缺失" };
    }
    if (typeof rec.outputRoot !== "string" || rec.outputRoot.length === 0) {
      return { status: "corrupted", error: "输出目录缺失" };
    }
    if (rec.status !== "unknown") {
      return { status: "corrupted", error: "记录状态非 unknown" };
    }
    if (typeof rec.savedAt !== "number" || !Number.isFinite(rec.savedAt)) {
      return { status: "corrupted", error: "保存时间戳缺失或无效" };
    }

    if (rec.kind === "archived_recovery") {
      if (!isValidArchivedRecoveryRequest(rec.request, sourceTaskId, rec.targetTaskId, rec.createdAt)) {
        return { status: "corrupted", error: "归档恢复请求体结构损坏或字段不一致" };
      }
      return {
        status: "valid",
        record: rec as PendingNisarRecoveryRecord,
      };
    }

    if (rec.kind === "fallback_recreate") {
      if (!isValidFallbackRequest(rec.request, sourceTaskId, rec.targetTaskId, rec.createdAt, rec.outputRoot)) {
        return { status: "corrupted", error: "Fallback 请求体结构损坏或字段不一致" };
      }
      if (
        typeof rec.originalItemsCount !== "number" ||
        rec.originalItemsCount !== (rec.request as DesktopNisarSuccessorTaskV2Request).source_item_ids.length
      ) {
        return { status: "corrupted", error: "景数统计与请求体不一致" };
      }
      return {
        status: "valid",
        record: rec as PendingNisarRecoveryRecord,
      };
    }

    return { status: "corrupted", error: `未知记录类型: ${String(rec.kind)}` };
  } catch (err) {
    return { status: "corrupted", error: err instanceof Error ? err.message : "未知存储读取异常" };
  }
}

// 局限提示：localStorage 不是跨 WebView 原子锁，仅用于当前客户端本地会话的保守保护，不能声称跨窗口或跨实例原子安全
function persistPendingRecord(record: PendingNisarRecoveryRecord): boolean {
  try {
    if (typeof window === "undefined" || !window.localStorage) return false;
    window.localStorage.setItem(getStorageKey(record.sourceTaskId), JSON.stringify(record));
    noteSourceRecordChange(record.sourceTaskId);
    return true;
  } catch {
    return false;
  }
}

function samePendingRequest(a: PendingNisarRecoveryRecord, b: PendingNisarRecoveryRecord): boolean {
  return a.kind === b.kind && a.sourceTaskId === b.sourceTaskId &&
    a.targetTaskId === b.targetTaskId && a.createdAt === b.createdAt &&
    a.outputRoot === b.outputRoot && JSON.stringify(a.request) === JSON.stringify(b.request);
}

function removeMatchingPendingRecord(record: PendingNisarRecoveryRecord): boolean {
  try {
    if (typeof window === "undefined" || !window.localStorage) return false;
    const current = loadPendingRecord(record.sourceTaskId);
    if (current.status !== "valid" || !samePendingRequest(current.record, record)) return false;
    window.localStorage.removeItem(getStorageKey(record.sourceTaskId));
    noteSourceRecordChange(record.sourceTaskId);
    return window.localStorage.getItem(getStorageKey(record.sourceTaskId)) === null;
  } catch {
    return false;
  }
}

export function NisarArchivedRecoveryConfirmation({ sourceTaskId, newTaskId, onClose, onCreated, pendingOnly = false }: {
  sourceTaskId: string; newTaskId: string; onClose: () => void; onCreated: (taskId: string) => void; pendingOnly?: boolean;
}) {
  const { locale } = useI18n();
  const [loadResult, setLoadResult] = useState<PendingRecordLoadResult>(() => loadPendingRecord(sourceTaskId));
  const [context, setContext] = useState<DesktopNisarArchivedRecoveryContextV2 | null>(null);
  const [fresh, setFresh] = useState<DesktopNisarAppendContextV2 | null>(null);
  const [error, setError] = useState<string | null>(() => {
    const initial = loadPendingRecord(sourceTaskId);
    if (initial.status === "corrupted") {
      return `本地存储状态异常或记录损坏（${initial.error}）。为防止重复提交任务已锁定，请保留原任务与 checkpoint。`;
    }
    if (initial.status === "valid") {
      return `检测到此前已提交创建请求（目标任务 ID：${initial.record.targetTaskId}），但结果暂不明确。创建结果暂不明确，请重试此确认以核对同一任务；不要新建重复任务。`;
    }
    return null;
  });
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    const onEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented || event.isComposing || isSettingsModalOpen()) return;
      event.preventDefault();
      event.stopPropagation();
      if (!busy) onClose();
    };
    window.addEventListener("keydown", onEscape, true);
    return () => window.removeEventListener("keydown", onEscape, true);
  }, [busy, onClose]);
  const [observation, setObservation] = useState<{
    token: object;
    result: DesktopSourceMutationStatusResult;
  } | null>(null);

  const mountedRef = useRef(true);
  const sendingRef = useRef(false);
  const loadedEpochRef = useRef({ sourceTaskId, epoch: sourceCompletionEpoch.get(sourceTaskId) ?? 0 });
  const loadedSharedEpochRef = useRef({ sourceTaskId, epoch: sourceOperationEpoch(sourceTaskId) });
  const activeSessionRef = useRef({ sourceTaskId, newTaskId, generation: 0 });
  useLayoutEffect(() => {
    if (activeSessionRef.current.sourceTaskId !== sourceTaskId || activeSessionRef.current.newTaskId !== newTaskId) {
      activeSessionRef.current = { sourceTaskId, newTaskId, generation: activeSessionRef.current.generation + 1 };
      loadedEpochRef.current = { sourceTaskId, epoch: sourceCompletionEpoch.get(sourceTaskId) ?? 0 };
      loadedSharedEpochRef.current = { sourceTaskId, epoch: sourceOperationEpoch(sourceTaskId) };
    }
  }, [sourceTaskId, newTaskId]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const defaultCreatedAt = useMemo(() => new Date().toISOString(), [sourceTaskId]);
  const effectiveCreatedAt = loadResult.status === "valid" && loadResult.record.sourceTaskId === sourceTaskId
    ? loadResult.record.createdAt
    : defaultCreatedAt;

  const isCorrupted = loadResult.status === "corrupted";
  const pendingCurrent = !isCorrupted && loadResult.status === "valid" && loadResult.record.sourceTaskId === sourceTaskId
    ? loadResult.record
    : null;
  const operationSnapshot = useSyncExternalStore(subscribeSources, () => sourceSnapshot(sourceTaskId), () => "0:0:0");
  const sharedSnapshot = useSyncExternalStore(subscribeSourceOperation, () => sourceOperationSnapshot(sourceTaskId), () => "0:0");
  const recordVersion = operationSnapshot.split(":")[1];
  const observationToken = useMemo(() => ({}), [sourceTaskId, newTaskId, pendingCurrent, busy, operationSnapshot, sharedSnapshot]);

  // A second confirmation in this WebView may save or remove the same record.
  // Refreshing it must not reset the separate successful-operation epoch.
  useEffect(() => {
    const current = loadPendingRecord(sourceTaskId);
    setLoadResult(current);
    if (current.status === "corrupted") {
      setError("本地待确认记录不可用或已损坏，已阻止创建；请保留原任务与 checkpoint。");
    }
  }, [sourceTaskId, recordVersion]);

  const contextCurrent = !isCorrupted && !pendingCurrent && context?.source_task_id === sourceTaskId && context?.new_task_id === newTaskId;
  const freshCurrent = !isCorrupted && !pendingCurrent && fresh?.task_id === sourceTaskId
    && ["failed", "cancelled", "completed"].includes(fresh.task_status)
    && !!fresh.original_item_ids?.length;
  const freshOutputRoot = freshCurrent
    ? `${fresh!.output_root.replace(/[\\/]+$/, "")}${fresh!.output_root.includes("\\") ? "\\" : "/"}${newTaskId}`
    : "";

  const currentMutationStatus = pendingCurrent && !busy && operationSnapshot.endsWith(":0") && sharedSnapshot.endsWith(":0")
    && observation?.token === observationToken ? observation.result : null;

  // Query the stored request without changing the gate. Its result is a snapshot.
  useEffect(() => {
    if (!pendingCurrent || busy || !operationSnapshot.endsWith(":0") || !sharedSnapshot.endsWith(":0")
      || sourceSnapshot(sourceTaskId) !== operationSnapshot || sourceOperationSnapshot(sourceTaskId) !== sharedSnapshot) return;
    let active = true;
    const boundSourceTaskId = sourceTaskId;
    const boundGeneration = activeSessionRef.current.generation;
    const boundRecord = pendingCurrent;
    const preCheck = loadPendingRecord(boundSourceTaskId);
    if (preCheck.status !== "valid" || !samePendingRequest(preCheck.record, boundRecord)) return;
    const save = (result: DesktopSourceMutationStatusResult) => {
      if (!mountedRef.current || !active || activeSessionRef.current.generation !== boundGeneration
        || activeSessionRef.current.sourceTaskId !== boundSourceTaskId
        || sendingRef.current || sourceSnapshot(boundSourceTaskId) !== operationSnapshot
        || sourceOperationSnapshot(boundSourceTaskId) !== sharedSnapshot) return;
      const postCheck = loadPendingRecord(boundSourceTaskId);
      if (postCheck.status === "valid" && samePendingRequest(postCheck.record, boundRecord)) {
        setObservation({ token: observationToken, result });
      }
    };
    const query = boundRecord.kind === "archived_recovery"
      ? queryNisarArchivedRecoveryStatus(boundRecord.request)
      : queryNisarSuccessorStatus(boundRecord.request);
    void query.then(save).catch(() => save({ ok: false, error: "Status query unavailable", code: "V3_APPLICATION_UNAVAILABLE" }));
    return () => { active = false; };
  }, [sourceTaskId, pendingCurrent, busy, operationSnapshot, sharedSnapshot, observationToken]);

  useEffect(() => {
    let active = true;
    const generation = activeSessionRef.current.generation;
    const isCurrent = () => active && activeSessionRef.current.generation === generation;
    const res = loadPendingRecord(sourceTaskId);
    setLoadResult(res);
    setContext(null);
    setFresh(null);

    if (res.status === "corrupted") {
      // 存储不可读、损坏或校验不通过：fail closed，绝不读取 context 或生成新目标
      setError(`本地存储状态异常或记录损坏（${res.error}）。为防止重复提交任务已锁定，请保留原任务与 checkpoint。`);
      return () => { active = false; };
    }

    if (res.status === "valid") {
      // 请求结果不明时保留记录，关闭重开也不得生成新目标或静默覆盖，提供明确的中文待核对提示
      setError(
        `检测到此前已提交创建请求（目标任务 ID：${res.record.targetTaskId}），但结果暂不明确。创建结果暂不明确，请重试此确认以核对同一任务；不要新建重复任务。`
      );
      return () => { active = false; };
    }

    if (pendingOnly) {
      setError("待确认记录已移除或不可用。请关闭窗口并核对原任务，未创建新任务。");
      return () => { active = false; };
    }

    setError(null);
    void readNisarArchivedRecoveryContextV2(sourceTaskId, newTaskId).then((result) => {
      if (!isCurrent()) return;
      if (result.ok) setContext(result.context);
      else if (result.code === "NISAR_ARCHIVED_RECOVERY_SOURCE_INELIGIBLE") {
        void readNisarSubsetAppendContextV2({ task_id: sourceTaskId }).then((fallback) => {
          if (!isCurrent()) return;
          if (fallback.ok && ["failed", "cancelled", "completed"].includes(fallback.context.task_status)
            && fallback.context.original_item_ids?.length) setFresh(fallback.context);
          else setError(`无法安全继承该任务：${formatBridgeError(result)} 原任务和 checkpoint 已保留。`);
        }).catch(() => { if (isCurrent()) setError(`无法安全继承该任务：${formatBridgeError(result)} 原任务和 checkpoint 已保留。`); });
      } else setError(`无法安全继承该任务：${formatBridgeError(result)} 原任务和 checkpoint 已保留。`);
    }).catch(() => { if (isCurrent()) setError("无法读取归档来源，请稍后重试；未创建任务。"); });
    return () => { active = false; };
  }, [sourceTaskId, newTaskId, pendingOnly]);

  async function confirm() {
    const isPending = !!pendingCurrent;
    if (pendingOnly && !isPending) return;
    if (isCorrupted || (!isPending && !contextCurrent && !freshCurrent) || busy || sendingRef.current) return;
    if (!isPending && (loadedEpochRef.current.sourceTaskId !== sourceTaskId ||
      loadedEpochRef.current.epoch !== (sourceCompletionEpoch.get(sourceTaskId) ?? 0))) {
      setError("该来源已有其他确认完成，请关闭窗口并重新核对任务后再操作。");
      return;
    }
    if (inFlightSources.has(sourceTaskId)) {
      setError("该来源已有请求正在处理，请等待后再核对。");
      return;
    }
    sendingRef.current = true;
    setBusy(true);
    setError(null);

    const boundSourceTaskId = sourceTaskId;
    const boundGeneration = activeSessionRef.current.generation;
    let targetTaskId: string;
    let candidateRecord: PendingNisarRecoveryRecord;

    if (isPending && pendingCurrent) {
      targetTaskId = pendingCurrent.targetTaskId;
      candidateRecord = pendingCurrent;
    } else if (contextCurrent && context) {
      targetTaskId = context.new_task_id;
      candidateRecord = {
        kind: "archived_recovery",
        sourceTaskId: context.source_task_id,
        targetTaskId: context.new_task_id,
        createdAt: effectiveCreatedAt,
        outputRoot: context.output_root,
        status: "unknown",
        savedAt: Date.now(),
        request: {
          source_task_id: context.source_task_id,
          source_item_id: context.source_item_id,
          expected_source_revision: context.expected_source_revision,
          new_task_id: context.new_task_id,
          created_at: effectiveCreatedAt,
        },
      };
    } else if (freshCurrent && fresh) {
      targetTaskId = newTaskId;
      candidateRecord = {
        kind: "fallback_recreate",
        sourceTaskId,
        targetTaskId: newTaskId,
        createdAt: effectiveCreatedAt,
        outputRoot: freshOutputRoot,
        originalItemsCount: fresh.original_item_ids!.length,
        status: "unknown",
        savedAt: Date.now(),
        request: {
          source_task_id: sourceTaskId,
          expected_task_revision: fresh.task_revision,
          source_item_ids: fresh.original_item_ids!,
          new_task_id: newTaskId,
          output_root: freshOutputRoot,
          layout: fresh.layout,
          concurrency: fresh.concurrency,
          created_at: effectiveCreatedAt,
          recreate_original_items: true,
        },
      };
    } else {
      sendingRef.current = false;
      setBusy(false);
      return;
    }

    // 发送前重新同步读取持久化记录，防止覆盖其他组件或会话已经写入的不同待处理记录
    const latest = loadPendingRecord(boundSourceTaskId);
    if (latest.status === "corrupted") {
      setError(`本地存储状态异常或读取受限（${latest.error}）。已阻止发送，请保留原任务与 checkpoint。`);
      sendingRef.current = false;
      setBusy(false);
      return;
    }
    if (isPending && latest.status === "empty") {
      setLoadResult(latest);
      setError("待核对记录已被移除，请重新打开确认窗口并核对原目标任务。");
      sendingRef.current = false;
      setBusy(false);
      return;
    }
    if (latest.status === "valid") {
      const isSame = samePendingRequest(latest.record, candidateRecord);
      if (!isSame) {
        setLoadResult(latest);
        setError(`检测到已存在其他待处理请求（目标任务 ID：${latest.record.targetTaskId}）。已阻止覆盖原记录，请先核对该请求。`);
        sendingRef.current = false;
        setBusy(false);
        return;
      }
    }

    if (!acquireSourceOperation(boundSourceTaskId)) {
      setError("该来源已有请求正在处理，请等待后再核对。");
      sendingRef.current = false;
      setBusy(false);
      return;
    }
    // Existing records may be retried even if another old record also exists.
    if (!isPending && (loadedSharedEpochRef.current.sourceTaskId !== boundSourceTaskId
      || loadedSharedEpochRef.current.epoch !== sourceOperationEpoch(boundSourceTaskId)
      || isSourceMutationStorageCorrupted() || getSourceMutationGate(boundSourceTaskId)
      || isReacquireStorageCorrupted().corrupted || getPendingReacquireRecord(boundSourceTaskId))) {
      releaseSourceOperation(boundSourceTaskId);
      setError("该来源的操作状态已变化或有待确认请求，请先核对后重新打开。");
      sendingRef.current = false;
      setBusy(false);
      return;
    }

    // 发送前持久化完整冻结请求身份和目标 ID，以 sourceTaskId 隔离；存储失败不得发送
    const saved = persistPendingRecord(candidateRecord);
    if (!saved) {
      releaseSourceOperation(boundSourceTaskId);
      setError("无法持久化保存请求身份，已阻止发送以防止重复提交；原任务和 checkpoint 已保留。");
      sendingRef.current = false;
      setBusy(false);
      return;
    }

    // 乐观比对校验写结果
    const verified = loadPendingRecord(boundSourceTaskId);
    if (
      verified.status !== "valid" ||
      !samePendingRequest(verified.record, candidateRecord)
    ) {
      releaseSourceOperation(boundSourceTaskId);
      setError("本地状态持久化校验失败或存在并发冲突，已阻止发送。请保留原任务与 checkpoint。");
      sendingRef.current = false;
      setBusy(false);
      return;
    }
    setLoadResult(verified);
    sendingRef.current = true;
    inFlightSources.add(boundSourceTaskId);
    sourceAttempts.set(boundSourceTaskId, (sourceAttempts.get(boundSourceTaskId) ?? 0) + 1);
    notifySources();

    try {
      const result = candidateRecord.kind === "archived_recovery"
        ? await createNisarArchivedRecoveryV2(candidateRecord.request)
        : await createNisarSubsetSuccessorTaskV2(candidateRecord.request);

      const isStillCurrentSource = mountedRef.current && activeSessionRef.current.generation === boundGeneration;

      if (!result.ok) {
        // result.ok=false 包括 transport unavailable，不能一概展示确定创建失败，要提示结果可能不明确并保留冻结请求
        if (isStillCurrentSource) {
          setError(`未能确认创建结果（${formatBridgeError(result)}）。已保留原请求，请核对目标任务 ${targetTaskId}；不要新建重复任务。`);
        }
        return;
      }

      const tasks = result.core.tasks;
      const targetVisible = Array.isArray(tasks) && tasks.some((task) =>
        task && typeof task === "object" && (task as Record<string, unknown>).task_id === targetTaskId
      );
      if (targetVisible) {
        sourceCompletionEpoch.set(boundSourceTaskId, (sourceCompletionEpoch.get(boundSourceTaskId) ?? 0) + 1);
        noteSourceOperationSuccess(boundSourceTaskId);
      }
      if (isStillCurrentSource) {
        if (!targetVisible) {
          setError(`已收到响应，但快照中未找到目标任务 ${targetTaskId}；请在下载中心核对结果。`);
          return;
        }
        if (!removeMatchingPendingRecord(candidateRecord)) {
          setError(`创建请求已返回成功，但本地记录未能解除。请关闭并重新打开窗口，核对目标任务 ${targetTaskId}，再重试同一请求。`);
          return;
        }
        setLoadResult({ status: "empty" });
        onCreated(targetTaskId);
      }
    } catch {
      const isStillCurrentSource = mountedRef.current && activeSessionRef.current.generation === boundGeneration;
      if (isStillCurrentSource) {
        setError("创建结果暂不明确，请重试此确认以核对同一任务；不要新建重复任务。");
      }
    } finally {
      inFlightSources.delete(boundSourceTaskId);
      releaseSourceOperation(boundSourceTaskId);
      notifySources();
      sendingRef.current = false;
      if (mountedRef.current) {
        setBusy(false);
      }
    }
  }

  const isFreshMode = pendingCurrent ? pendingCurrent.kind === "fallback_recreate" : freshCurrent;
  const originalItemsCount = pendingCurrent
    ? (pendingCurrent.kind === "fallback_recreate" ? pendingCurrent.originalItemsCount : 0)
    : (fresh?.original_item_ids?.length ?? 0);
  const displayOutputRoot = pendingCurrent
    ? pendingCurrent.outputRoot
    : (contextCurrent ? context!.output_root : (freshCurrent ? freshOutputRoot : ""));
  const canConfirm = !isCorrupted && (pendingOnly ? !!pendingCurrent : (!!pendingCurrent || contextCurrent || freshCurrent)) && !busy;

  return <div className="fixed inset-0 z-[1500] flex items-center justify-center bg-black/40">
    <section role="dialog" aria-modal="true" aria-label="继承归档裁剪断点" className="max-w-xl space-y-4 rounded-xl border border-border bg-card p-6 text-card-foreground shadow-xl">
      <h2 className="text-lg font-semibold">{pendingCurrent ? "核对原恢复请求" : isFreshMode ? "重新创建历史裁剪任务" : "继承归档裁剪断点"}</h2>
      {pendingCurrent
        ? <p>此前的恢复请求结果尚未确认。可按保存的原请求重试，然后在下载中心查看目标任务的当前状态。</p>
        : isFreshMode
        ? <p>原任务没有可安全继承的完整科学断点。可以按冻结的 {originalItemsCount} 景影像、原 AOI、频率和极化重新创建暂停任务。<strong>这是从头开始下载，不会复用原任务的下载进度。</strong></p>
        : <p>原任务继续保留在历史记录中。将按原影像、裁剪区域、频段和极化创建新的暂停任务，校验并复制有效断点后续传。</p>}
      {!pendingCurrent && <p>新任务使用独立输出目录，不覆盖已有结果。点击新任务的“继续”后开始校验和下载。</p>}
      {displayOutputRoot ? <p className="break-all text-sm">{pendingCurrent ? "请求输出目录" : "新输出目录"}：{displayOutputRoot}</p> : !error && <p role="status">正在核对归档来源…</p>}
      {error && <p role="alert" className="text-sm text-red-600">{error}</p>}
      {pendingCurrent && <p className="text-sm">待核对的目标任务 ID：{pendingCurrent.targetTaskId}</p>}
      {currentMutationStatus && (
        <SourceMutationStatusNotice result={currentMutationStatus} language={locale} />
      )}
      <div className="flex justify-end gap-3">
        <button type="button" disabled={busy} onClick={onClose}>取消</button>
        <button type="button" disabled={!canConfirm} onClick={() => void confirm()}>{busy ? "正在处理…" : pendingCurrent ? "重试原请求" : isFreshMode ? "确认重新创建暂停任务（不继承旧断点）" : "确认继承并创建暂停任务"}</button>
      </div>
    </section>
  </div>;
}
