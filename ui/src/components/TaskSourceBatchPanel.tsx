import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useI18n } from "@/i18n";
import { hasPendingNisarRecovery } from "@/lib/sourceOperationCoordination";
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
  type FrozenAppendSourceBatchMutation,
  type FrozenBurstAppendMutation,
  type FrozenBurstSuccessorMutation,
  type FrozenSourceMutation,
  type FrozenSuccessorSourceBatchMutation,
  type SourceMutationGate,
} from "@/lib/taskSourceMutationGate";
import { getPendingReacquireRecord, isStorageCorrupted as isReacquireStorageCorrupted,
  reacquireGateVersion, subscribeReacquireGate } from "@/lib/reacquireRequestGate";
import { TaskSourceBatchResults } from "@/components/TaskSourceBatchResults";
import { SourceMutationStatusNotice } from "@/components/SourceMutationStatusNotice";
import { NisarArchivedRecoveryConfirmation } from "@/components/NisarArchivedRecoveryConfirmation";
import { useSourceMutationStatus } from "@/lib/useSourceMutationStatus";
import { FrozenProductPreview } from "@/components/FrozenProductPreview";
import { DownloadDestinationConfirmation } from "@/components/DownloadDestinationConfirmation";
import {
  appendApplicationBurstAssetTask,
  appendApplicationTaskFromSourceBatch,
  appendNisarSubsetBatchTaskV2,
  createNisarSubsetSuccessorTaskV2,
  createApplicationHistoricalBurstAssetTask,
  createApplicationTaskFromHistoricalSourceBatch,
  formatBridgeError,
  loadApplicationTaskSourceBatchPage,
  pickDirectory,
} from "@/lib/bridge";
import {
  createDownloadAcquisitionDraft,
  type ConfirmedDownloadTaskDraft,
  type DownloadAcquisitionDraft,
} from "@/lib/downloadDestinationConfirmation";
import type { DownloadLayout, StoreTaskSourceBatchPage } from "@/lib/generated/v3-contracts";
import {
  createTaskHistoricalUiState,
  taskHistoricalAction,
  updateTaskHistoricalUiState,
} from "@/lib/taskHistoricalContext";

function uniqueId(prefix: string): string {
  const randomUuid = globalThis.crypto?.randomUUID?.();
  return randomUuid ? `${prefix}-${randomUuid}` : `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
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

// This checks current membership, not an event receipt. The sealed source is
// bounded to 10,000 rows and must remain the same across the page reads.
async function verifyMembership(gate: SourceMutationGate): Promise<boolean> {
  if (gate.frozenMutation?.kind === "nisar_append" || gate.frozenMutation?.kind === "nisar_successor") return false;
  const remaining = new Set(gate.itemIds);
  let total: number | null = null;
  const seen = new Set<string>();
  for (let index = 0; index < 50; index += 1) {
    const result = await loadApplicationTaskSourceBatchPage({ task_id: gate.targetTaskId, page_index: index, page_size: 200 });
    if (!result.ok || result.page.task_id !== gate.targetTaskId || result.page.source_batch_id !== gate.sourceBatchId
      || result.page.page_index !== index || result.page.page_size !== 200
      || result.page.task_revision < gate.minRevision || result.page.total_count > 10_000
      || (total !== null && result.page.total_count !== total)) return false;
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

export interface TaskSourceBatchPanelProps {
  taskId: string;
  taskTitle?: string;
  taskStatus: string;
  taskCategory?: string;
  datasetLabel?: string;
  sourceProviderId?: string;
  taskType?: string | null;
  outputDir?: string | null;
  concurrency?: number | null;
  downloadLayout?: DownloadLayout;
  newTaskConcurrency?: number;
  newTaskDownloadLayout?: DownloadLayout;
  newTaskPolicyReady?: boolean;
  onChooseNewTaskOutput?: () => Promise<string | null>;
  onHistoricalMutationApplied?: () => void | Promise<void>;
  onOpenNisarWorkspace?: (selectedIds: string[]) => void | Promise<void>;
  compact?: boolean;
}

function checkIsNisar(
  taskType: string | null | undefined,
  datasetLabel: string | undefined,
  page: StoreTaskSourceBatchPage | null,
): boolean {
  if (taskType?.trim().toLowerCase() === "nisar_subset") return true;
  if (datasetLabel?.trim().toUpperCase().includes("NISAR")) return true;
  if (page?.items.some((entry) => {
    const d = String(entry.item.metadata?.dataset ?? "").toUpperCase();
    const kind = String(entry.item.metadata?.delivery_kind ?? "").toLowerCase();
    const p = String(entry.item.provider_id ?? "").toUpperCase();
    return d.includes("NISAR") || kind === "remote_derived_subset_v2" || p.includes("NISAR");
  })) {
    return true;
  }
  return false;
}

export function TaskSourceBatchPanel({
  taskId,
  taskTitle,
  taskStatus,
  taskCategory,
  datasetLabel,
  sourceProviderId,
  taskType,
  outputDir = null,
  concurrency = null,
  downloadLayout = "direct",
  newTaskConcurrency,
  newTaskDownloadLayout,
  newTaskPolicyReady = false,
  onChooseNewTaskOutput,
  onHistoricalMutationApplied,
  onOpenNisarWorkspace,
  compact = false,
}: TaskSourceBatchPanelProps) {
  const { t, locale } = useI18n();
  const translateRef = useRef(t);
  translateRef.current = t;
  const [historical, setHistorical] = useState(() => createTaskHistoricalUiState());
  const [sourcePage, setSourcePage] = useState<StoreTaskSourceBatchPage | null>(null);
  const [sourceBusy, setSourceBusy] = useState(false);
  const [sourceError, setSourceError] = useState<string | null>(null);
  const [mutationBusy, setMutationBusy] = useState(false);
  const [mutationNotice, setMutationNotice] = useState<string | null>(null);
  const [mutationError, setMutationError] = useState<string | null>(null);
  const [recoveryTarget, setRecoveryTarget] = useState<{ sourceTaskId: string; newTaskId: string } | null>(null);
  const [, setGateVersion] = useState(0);
  const [successorConfirmation, setSuccessorConfirmation] = useState<{
    draft: DownloadAcquisitionDraft;
    isBurst: boolean;
    epoch: number;
  } | null>(null);
  const [historicalPreview, setHistoricalPreview] = useState<{
    batchId: string;
    itemId: string;
    productLabel: string;
  } | null>(null);

  const sourceGeneration = useRef(0);
  const mutationGeneration = useRef(0);
  const activeTaskIdRef = useRef(taskId);
  activeTaskIdRef.current = taskId;
  useEffect(() => { setRecoveryTarget(null); }, [taskId]);
  useEffect(() => subscribeSourceMutationGate(() => setGateVersion((value) => value + 1)), []);
  useEffect(() => subscribeReacquireGate(() => setGateVersion((value) => value + 1)), []);
  useSyncExternalStore(subscribeSourceMutationGate, sourceMutationGateVersion, () => 0);
  useSyncExternalStore(subscribeReacquireGate, reacquireGateVersion, () => 0);
  const pendingGate = getSourceMutationGate(taskId);
  const mutationStatus = useSourceMutationStatus(taskId, pendingGate);
  const isMutationLocked = mutationBusy || isSourceMutationInFlight(taskId);

  const loadSourcePage = useCallback(async (pageIndex: number, pageSize: number) => {
    const generation = ++sourceGeneration.current;
    setSourceBusy(true);
    setSourceError(null);
    try {
      const result = await loadApplicationTaskSourceBatchPage({
        task_id: taskId,
        page_index: Math.max(0, Math.trunc(pageIndex)),
        page_size: Math.min(200, Math.max(1, Math.trunc(pageSize) || 24)),
      });
      if (generation !== sourceGeneration.current || taskId !== activeTaskIdRef.current) return;
      if (!result.ok) {
        setSourceError(formatBridgeError(result));
        return;
      }
      if (result.page.task_id !== taskId || result.page.page_index !== Math.max(0, Math.trunc(pageIndex))
        || result.page.page_size !== Math.min(200, Math.max(1, Math.trunc(pageSize) || 24))) {
        setSourceError(translateRef.current("downloads.sourceBatch.loadFailed"));
        return;
      }
      setSourcePage(result.page);
    } catch (loadError) {
      if (generation === sourceGeneration.current && taskId === activeTaskIdRef.current) {
        setSourceError(formatBridgeError(loadError));
      }
    } finally {
      if (generation === sourceGeneration.current && taskId === activeTaskIdRef.current) {
        setSourceBusy(false);
      }
    }
  }, [taskId]);

  useEffect(() => {
    setHistorical(createTaskHistoricalUiState());
    setSourcePage(null);
    setSourceBusy(false);
    setSourceError(null);
    setMutationBusy(false);
    setMutationNotice(null);
    setMutationError(null);
    setSuccessorConfirmation(null);
    setHistoricalPreview(null);
    sourceGeneration.current += 1;
    mutationGeneration.current += 1;
    void loadSourcePage(0, 24);
    return () => {
      sourceGeneration.current += 1;
      mutationGeneration.current += 1;
    };
  }, [loadSourcePage, taskId]);

  const dataset = datasetLabel?.trim().toUpperCase();
  const isBurst = dataset === "SLC-BURST";
  const isNisar = checkIsNisar(taskType, datasetLabel, sourcePage);
  const action = taskHistoricalAction(sourcePage?.task_status ?? taskStatus);
  // The overview supplies a raw provider ID and task type. Unknown or
  // explicitly derived task kinds must not enter the generic task path.
  const isSupported = taskCategory === "scene_download" && taskType === null
    && sourceProviderId === "asf.catalog" && (dataset === "SENTINEL-1" || isBurst)
    && Boolean(sourcePage) && sourcePage?.task_id === taskId
    && sourcePage.items.every((entry) => entry.item.provider_id === "asf.catalog"
      && String(entry.item.metadata.dataset ?? "").trim().toUpperCase() === dataset);

  let disabledReason: string | null = null;
  if (taskCategory === "dem_convert") {
    disabledReason = t("downloads.sourceBatch.demNotSupported");
  } else if (isNisar && !onOpenNisarWorkspace) {
    disabledReason = t("downloads.sourceBatch.nisarSafetyRejected");
  } else if (!isSupported) {
    disabledReason = t("downloads.sourceBatch.unknownTypeSafetyRejected");
  } else if (isSourceMutationStorageCorrupted() || isReacquireStorageCorrupted().corrupted) {
    disabledReason = t("downloads.sourceBatch.guardUnavailable");
  } else if (pendingGate) {
    disabledReason = t("downloads.sourceBatch.outcomeUnknown");
  } else if (hasPendingNisarRecovery(taskId)) {
    disabledReason = t("downloads.sourceBatch.nisarRecoveryPending");
  } else if (getPendingReacquireRecord(taskId)) {
    disabledReason = t("downloads.sourceBatch.reacquirePending");
  } else if (isSourceMutationInFlight(taskId)) {
    disabledReason = t("downloads.sourceBatch.submitting");
  } else if (historical.selectedIds.length > 250) {
    disabledReason = t("downloads.sourceBatch.largeSelectionNote");
  } else if (action === "unavailable") {
    disabledReason = t("downloads.sourceBatch.unavailableNote");
  } else if (action === "create_successor" && !newTaskPolicyReady) {
    disabledReason = t("downloads.sourceBatch.sarPolicyNotReady");
  }

  async function handleChooseDirectory(): Promise<string | null> {
    if (onChooseNewTaskOutput) {
      return onChooseNewTaskOutput();
    }
    try {
      const picked = await pickDirectory(t("mapWorkbench.desktop.chooseTaskOutput"));
      if (!picked.ok) return null;
      return picked.path?.trim() || null;
    } catch {
      return null;
    }
  }

  async function submitHistoricalSelection(itemIds: string[]) {
    const uniqueItemIds = Array.from(new Set(itemIds));
    if (!sourcePage || pendingGate || getPendingReacquireRecord(taskId) || isReacquireStorageCorrupted().corrupted || sourceBusy || sourceError
      || uniqueItemIds.length !== itemIds.length || itemIds.length === 0 || itemIds.length > 250
      || isMutationLocked || isSourceMutationStorageCorrupted() || successorConfirmation) return;
    const nonEmptyItemIds = itemIds as [string, ...string[]];

    if (taskCategory === "dem_convert") {
      setMutationError(t("downloads.sourceBatch.demNotSupported"));
      return;
    }
    if (isNisar) {
      if (onOpenNisarWorkspace) {
        await onOpenNisarWorkspace(itemIds);
        return;
      }
      setMutationError(t("downloads.sourceBatch.nisarSafetyRejected"));
      return;
    }
    if (!isSupported) {
      setMutationError(t("downloads.sourceBatch.unknownTypeSafetyRejected"));
      return;
    }
    if (action === "unavailable") {
      setMutationError(t("downloads.sourceBatch.unavailableNote"));
      return;
    }
    if (action === "create_successor" && !newTaskPolicyReady) {
      setMutationError(t("downloads.sourceBatch.sarPolicyNotReady"));
      return;
    }

    setMutationError(null);
    setMutationNotice(null);

    if (action === "create_successor") {
      try {
        const selectedLayout: DownloadLayout = isBurst
          ? "direct"
          : (VALID_DOWNLOAD_LAYOUTS.has(newTaskDownloadLayout as DownloadLayout)
              ? (newTaskDownloadLayout as DownloadLayout)
              : (VALID_DOWNLOAD_LAYOUTS.has(downloadLayout as DownloadLayout)
                  ? (downloadLayout as DownloadLayout)
                  : "direct"));

        const draft = createDownloadAcquisitionDraft({
          sourceBatchId: sourcePage.source_batch_id,
          sourceBatchRevision: sourcePage.task_revision,
          selectionScope: "checked",
          selectedProductIds: nonEmptyItemIds,
          acquisition: {
            kind: isBurst ? "burst" : "historical_asf",
            frozenParameters: {
              source_task_id: taskId,
              expected_source_task_revision: sourcePage.task_revision,
            },
          },
          destinationPrefill: outputDir ?? "",
          layout: selectedLayout,
          taskPolicy: "manual",
          concurrency: Math.min(8, Math.max(1, Math.trunc(newTaskConcurrency ?? concurrency ?? 1) || 1)),
        });
        setSuccessorConfirmation({ draft, isBurst, epoch: sourceMutationEpoch(taskId) });
      } catch (draftError) {
        setMutationError(formatBridgeError(draftError));
      }
      return;
    }

    // action === "append"
    if (!acquireSourceMutationInFlight(taskId)) {
      setMutationError(t("downloads.sourceBatch.guardUnavailable"));
      return;
    }
    try {
      setMutationBusy(true);
      const currentGen = ++mutationGeneration.current;
      const initialTaskId = taskId;

      const now = new Date().toISOString();
      const frozenMutation: FrozenSourceMutation = isBurst
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
        setMutationError(t("downloads.sourceBatch.guardUnavailable"));
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
            setMutationError(t("downloads.sourceBatch.guardUnavailable"));
            return;
          }
          setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "clear_selection" }));
          setMutationNotice(t("downloads.sourceBatch.membershipConfirmed", { taskId: gate.targetTaskId }));
          await loadSourcePage(sourcePage.page_index, sourcePage.page_size);
          await Promise.resolve().then(() => onHistoricalMutationApplied?.()).catch(console.warn);
        } else {
          const errStr = !isDirectOk
            ? (bridgeResult ? formatBridgeError(bridgeResult) : formatBridgeError(bridgeError))
            : t("downloads.sourceBatch.outcomeUnknown");
          finishSourceMutation(gate, false, errStr);
          setMutationError(errStr);
        }
      } catch (mutationErr) {
        finishSourceMutation(gate, false, formatBridgeError(mutationErr));
        if (currentGen === mutationGeneration.current && initialTaskId === activeTaskIdRef.current) {
          setMutationError(formatBridgeError(mutationErr));
        }
      } finally {
        if (currentGen === mutationGeneration.current && initialTaskId === activeTaskIdRef.current) {
          setMutationBusy(false);
        }
      }
    } catch (lockError) {
      if (activeTaskIdRef.current === taskId) {
        setMutationBusy(false);
        setMutationError(formatBridgeError(lockError));
      }
    } finally {
      releaseSourceMutationInFlight(taskId);
    }
  }

  async function onConfirmHistoricalSuccessor(confirmed: ConfirmedDownloadTaskDraft) {
    if (!successorConfirmation || !sourcePage || pendingGate || getPendingReacquireRecord(taskId)
      || isReacquireStorageCorrupted().corrupted || isMutationLocked || sourceBusy || sourceError || !isSupported
      || isSourceMutationStorageCorrupted()
      || !newTaskPolicyReady || action !== "create_successor"
      || successorConfirmation.epoch !== sourceMutationEpoch(taskId)
      || confirmed.sourceBatchId !== sourcePage.source_batch_id
      || confirmed.sourceBatchRevision !== sourcePage.task_revision) {
      throw new Error(t("downloads.sourceBatch.unknownTypeSafetyRejected"));
    }
    const itemIds = confirmed.selectedProductIds as [string, ...string[]];
    const uniqueItemIds = Array.from(new Set(itemIds));
    if (uniqueItemIds.length !== itemIds.length || uniqueItemIds.length === 0 || uniqueItemIds.length > 250) {
      throw new Error(t("downloads.sourceBatch.unknownTypeSafetyRejected"));
    }
    if (!VALID_DOWNLOAD_LAYOUTS.has(confirmed.layout)
      || !Number.isSafeInteger(confirmed.concurrency)
      || confirmed.concurrency < 1 || confirmed.concurrency > 8
      || (successorConfirmation.isBurst && confirmed.layout !== "direct")) {
      throw new Error(t("downloads.sourceBatch.unknownTypeSafetyRejected"));
    }

    if (!acquireSourceMutationInFlight(taskId)) {
      throw new Error(t("downloads.sourceBatch.guardUnavailable"));
    }
    try {
      setMutationBusy(true);
      setMutationError(null);
      setMutationNotice(null);
      const currentGen = ++mutationGeneration.current;
      const initialTaskId = taskId;
      const newTaskId = uniqueId("download-task");
      const now = new Date().toISOString();

      const frozenMutation: FrozenSourceMutation = successorConfirmation.isBurst
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
        throw new Error(t("downloads.sourceBatch.guardUnavailable"));
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
          if (!finished) throw new Error(t("downloads.sourceBatch.guardUnavailable"));
          setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "clear_selection" }));
          setMutationNotice(t("downloads.sourceBatch.membershipConfirmed", { taskId: gate.targetTaskId }));
          setSuccessorConfirmation(null);
          await loadSourcePage(sourcePage.page_index, sourcePage.page_size);
          await Promise.resolve().then(() => onHistoricalMutationApplied?.()).catch(console.warn);
        } else {
          const errStr = !isDirectOk
            ? (bridgeResult ? formatBridgeError(bridgeResult) : formatBridgeError(bridgeError))
            : t("downloads.sourceBatch.outcomeUnknown");
          finishSourceMutation(gate, false, errStr);
          throw new Error(errStr);
        }
      } catch (err) {
        finishSourceMutation(gate, false, formatBridgeError(err));
        if (currentGen === mutationGeneration.current && initialTaskId === activeTaskIdRef.current) {
          setMutationError(formatBridgeError(err));
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
        setMutationError(formatBridgeError(lockError));
      }
      throw lockError;
    } finally {
      releaseSourceMutationInFlight(taskId);
    }
  }

  async function handleRetryFrozenGate(gate: SourceMutationGate) {
    if (gate.sourceTaskId !== taskId || !gate.frozenMutation || isMutationLocked || isSourceMutationStorageCorrupted()) return;
    if (!acquireSourceMutationInFlight(taskId)) {
      setMutationError(t("downloads.sourceBatch.guardUnavailable"));
      return;
    }
    try {

      const staged = stageRetrySourceMutation(gate);
      if (!staged || !staged.frozenMutation) {
        setMutationError(t("downloads.sourceBatch.guardUnavailable"));
        return;
      }

      setMutationBusy(true);
      setMutationError(null);
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
            setMutationError(t("downloads.sourceBatch.guardUnavailable"));
            return;
          }
          setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "clear_selection" }));
          setMutationNotice(t(isNisarSourceMutation(staged) ? "downloads.sourceBatch.nisarRetryConfirmed" : "downloads.sourceBatch.membershipConfirmed", { taskId: staged.targetTaskId }));
          setSuccessorConfirmation(null);
          await loadSourcePage(sourcePage?.page_index ?? 0, sourcePage?.page_size ?? 24);
          await Promise.resolve().then(() => onHistoricalMutationApplied?.()).catch(console.warn);
        } else {
          const errStr = !isDirectOk
            ? (bridgeResult ? formatBridgeError(bridgeResult) : formatBridgeError(bridgeError))
            : t("downloads.sourceBatch.outcomeUnknown");
          finishSourceMutation(staged, false, errStr);
          setMutationError(errStr);
        }
      } catch (unknownErr) {
        finishSourceMutation(staged, false, formatBridgeError(unknownErr));
        if (currentGen === mutationGeneration.current && initialTaskId === activeTaskIdRef.current) {
          setMutationError(formatBridgeError(unknownErr));
        }
      } finally {
        if (currentGen === mutationGeneration.current && initialTaskId === activeTaskIdRef.current) {
          setMutationBusy(false);
        }
      }
    } catch (lockError) {
      if (activeTaskIdRef.current === taskId) {
        setMutationBusy(false);
        setMutationError(formatBridgeError(lockError));
      }
    } finally {
      releaseSourceMutationInFlight(taskId);
    }
  }

  async function handleCheckOutcome(gate: SourceMutationGate) {
    if (isNisarSourceMutation(gate)) return;
    if (gate.sourceTaskId !== taskId || isMutationLocked) return;
    if (!acquireSourceMutationInFlight(taskId)) {
      setMutationError(t("downloads.sourceBatch.guardUnavailable"));
      return;
    }
    const checkTaskId = taskId;
    let checkGeneration = mutationGeneration.current;
    try {
      checkGeneration = ++mutationGeneration.current;
      setMutationBusy(true);
      setMutationError(null);
      const confirmed = await verifyMembership(gate).catch(() => false);
      if (checkGeneration !== mutationGeneration.current || checkTaskId !== activeTaskIdRef.current) {
        return;
      }
      if (confirmed) {
        if (gate.directSuccess === true) {
          if (!finishSourceMutation(gate, true)) {
            setMutationError(t("downloads.sourceBatch.guardUnavailable"));
            return;
          }
          setMutationError(null);
          setSuccessorConfirmation(null);
          setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "clear_selection" }));
          void loadSourcePage(0, 24);
          void Promise.resolve().then(() => onHistoricalMutationApplied?.()).catch(console.warn);
        }
        setMutationNotice(t("downloads.sourceBatch.membershipConfirmed", { taskId: gate.targetTaskId }));
      } else {
        setMutationError(t("downloads.sourceBatch.outcomeUnknown"));
      }
    } finally {
      releaseSourceMutationInFlight(checkTaskId);
      if (checkGeneration === mutationGeneration.current && checkTaskId === activeTaskIdRef.current) {
        setMutationBusy(false);
      }
    }
  }

  return (
    <div className="space-y-3" data-task-source-batch-panel>
      {hasPendingNisarRecovery(taskId) && (
        <div role="alert" className="space-y-2 rounded-lg border border-destructive/30 p-3 text-sm">
          <p>{t("downloads.sourceBatch.nisarRecoveryPending")}</p>
          <button type="button" disabled={isMutationLocked} className="underline font-medium"
            onClick={() => setRecoveryTarget({ sourceTaskId: taskId, newTaskId: uniqueId("nisar-recovery") })}>
            {t("downloads.sourceBatch.openNisarRecovery")}
          </button>
        </div>
      )}
      {pendingGate && !pendingGate.pending && (
        <div role="alert" className="space-y-2 rounded-lg border border-destructive/30 p-3 text-sm">
          <p>{t("downloads.sourceBatch.outcomeUnknown")} {pendingGate.targetTaskId}</p>
          {pendingGate.error && <p>{pendingGate.error}</p>}
          <SourceMutationStatusNotice result={mutationStatus} language={locale} />
          <div className="flex flex-wrap items-center gap-3">
            {!isNisarSourceMutation(pendingGate) && <button
              type="button"
              disabled={isMutationLocked}
              className="underline"
              onClick={() => void handleCheckOutcome(pendingGate)}
            >
              {t("downloads.sourceBatch.checkOutcome")}
            </button>}
            {pendingGate.frozenMutation && (
              <button
                type="button"
                disabled={isMutationLocked}
                className="underline font-medium"
                onClick={() => void handleRetryFrozenGate(pendingGate)}
              >
                {t("downloads.sourceBatch.retryFrozenMutation")}
              </button>
            )}
          </div>
        </div>
      )}
      {mutationError && (
        <div role="alert" className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {mutationError}
        </div>
      )}
      {mutationNotice && (
        <div role="status" className="rounded-lg border border-primary/25 bg-primary/5 px-3 py-2 text-sm">
          {mutationNotice}
        </div>
      )}

      <div className={historicalPreview && !compact ? "grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(280px,0.8fr)]" : ""}>
        <TaskSourceBatchResults
          page={sourcePage}
          selectedIds={historical.selectedIds}
          filter={historical.filter}
          sort={historical.sort}
          busy={sourceBusy}
          mutationBusy={isMutationLocked}
          error={sourceError}
          disabledReason={disabledReason}
          onRetryLoad={() => void loadSourcePage(historical.pageIndex, historical.pageSize)}
          onToggleItem={(itemId) =>
            setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "toggle_item", itemId }))
          }
          onFilterChange={(filter) =>
            setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "set_filter", filter }))
          }
          onSortChange={(sort) =>
            setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "set_sort", sort }))
          }
          onPageIndexChange={(pageIndex) => {
            setHistorical((current) => updateTaskHistoricalUiState(current, { kind: "set_page", pageIndex }));
            void loadSourcePage(pageIndex, historical.pageSize);
          }}
          onSubmit={(itemIds) => void submitHistoricalSelection(itemIds)}
          onPreview={(item) => {
            if (sourcePage) {
              setHistoricalPreview({
                batchId: sourcePage.source_batch_id,
                itemId: item.item_id,
                productLabel: item.display_name,
              });
            }
          }}
        />

        {historicalPreview && (
          <div className="h-[420px] min-w-0 overflow-hidden rounded-md border lg:sticky lg:top-0">
            <FrozenProductPreview {...historicalPreview} onClose={() => setHistoricalPreview(null)} />
          </div>
        )}
      </div>

      {successorConfirmation && (
        <DownloadDestinationConfirmation
          key={`${successorConfirmation.draft.sourceBatchId}:${successorConfirmation.draft.sourceBatchRevision}:${successorConfirmation.draft.selectedProductIds.join(",")}`}
          draft={successorConfirmation.draft}
          chooseDirectory={handleChooseDirectory}
          onCancel={() => setSuccessorConfirmation(null)}
          onConfirm={onConfirmHistoricalSuccessor}
        />
      )}
      {recoveryTarget && recoveryTarget.sourceTaskId === taskId && (
        <NisarArchivedRecoveryConfirmation key={recoveryTarget.newTaskId} {...recoveryTarget} pendingOnly
          onClose={() => setRecoveryTarget(null)}
          onCreated={(createdTaskId) => {
            setRecoveryTarget(null);
            if (recoveryTarget.sourceTaskId === activeTaskIdRef.current) {
              setMutationNotice(t("downloads.sourceBatch.nisarRecoveryCreated", { taskId: createdTaskId }));
              void loadSourcePage(0, 24);
              void Promise.resolve().then(() => {
                if (recoveryTarget.sourceTaskId === activeTaskIdRef.current) return onHistoricalMutationApplied?.();
              }).catch(console.warn);
            }
          }} />
      )}
    </div>
  );
}
