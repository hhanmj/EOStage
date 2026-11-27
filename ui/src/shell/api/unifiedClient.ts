/**
 * C1 真实命令客户端统一封装
 * 依据 C1 契约规范与 W4 切片 7 需求：
 * - 纯模块可注入 dependencies.invoke 支撑 100% 桩单元测试
 * - 生产运行时默认绑定 desktopShellBridge 的真实命令
 */

import { t } from "../../i18n/index.ts";
import type {
  SourceDescriptor,
  SourceView,
  UnifiedSearchRequest,
  SearchProgress,
  StoreBatchPage,
  CreateTaskOptions,
  TaskRef,
  TaskRetryFallbackOutcome,
  TaskOverview,
  AccountSessionStatus,
  SearchCancelOutcome,
  BatchAllMatchPreviewRequest,
  BatchAllMatchPreview,
  BatchAllMatchCreateTaskRequest,
  BatchAllMatchTaskOutcome,
  UnifiedTaskItemAction,
  ExistingDownloadCheckItem,
  ExistingDownloadMatch,
} from "./types.ts";
import { parseTaskOverview } from "./taskOverviewParser.ts";
import { parseAccountSessionStatuses } from "../../lib/accountSessions.ts";
import { isTauriDesktopHost, getDesktopShellBridge } from "../../lib/desktopShellBridge.ts";

export type InvokeFn = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

export interface UnifiedClientDependencies {
  invoke: InvokeFn;
}

function defaultInvoke(): InvokeFn {
  return async (cmd: string, args?: Record<string, unknown>): Promise<unknown> => {
    if (!isTauriDesktopHost()) {
      throw new Error(t("shared.unifiedClient.desktopHostRequired"));
    }
    const bridge = getDesktopShellBridge();
    const unified = bridge.application.unified;
    if (!unified) {
      throw new Error(t("shared.unifiedClient.commandUnavailable", { cmd }));
    }

    switch (cmd) {
      case "application_list_sources":
        return unified.listSources();
      case "application_source_views":
        return unified.sourceViews();
      case "application_search_start":
        return args?.bypassCache
          ? unified.searchStart(args?.request as UnifiedSearchRequest, { bypassCache: true })
          : unified.searchStart(args?.request as UnifiedSearchRequest);
      case "application_search_progress":
        return unified.searchProgress(args?.requestId as string);
      case "application_search_cancel":
        return unified.searchCancel(args?.requestId as string);
      case "application_cancel_catalog_search": {
        const requestId = args?.requestId as string;
        if (unified.cancelCatalogSearch) return unified.cancelCatalogSearch(requestId);
        const cancelled = await unified.searchCancel(requestId);
        return { request_id: requestId, cancelled, batch_id: null, state: null };
      }
      case "application_batch_page":
        return unified.batchPage(
          args?.batchId as string,
          args?.revision as number,
          args?.page as number,
          args?.pageSize as number
        );
      case "application_create_task":
        return unified.createTask(
          args?.batchId as string,
          args?.revision as number,
          args?.itemIds as string[],
          args?.options as CreateTaskOptions
        );
      case "application_task_action":
        return unified.taskAction(
          args?.taskId as string,
          args?.revision as number,
          args?.action as any
        );
      case "application_task_item_action":
        if (!unified.taskItemAction) throw new Error(t("shared.unifiedClient.commandUnavailable", { cmd }));
        return unified.taskItemAction(
          args?.taskId as string,
          (args?.revision as number | null | undefined) ?? null,
          args?.itemId as string,
          args?.action as UnifiedTaskItemAction
        );
      case "application_task_delete_history":
        if (!unified.taskDeleteHistory) throw new Error(t("shared.unifiedClient.commandUnavailable", { cmd }));
        return unified.taskDeleteHistory(args?.taskId as string);
      case "application_task_set_pinned":
        if (!unified.taskSetPinned) throw new Error(t("shared.unifiedClient.commandUnavailable", { cmd }));
        return unified.taskSetPinned(args?.taskId as string, Boolean(args?.pinned));
      case "application_list_pinned_tasks":
        if (!unified.listPinnedTasks) throw new Error(t("shared.unifiedClient.commandUnavailable", { cmd }));
        return unified.listPinnedTasks();
      case "application_task_retry_fallback":
        return unified.taskRetryFallback(
          args?.taskId as string,
          (args?.confirmRefreshedBatchId as string | null) ?? null
        );
      case "application_task_retry_fallback_eligibility":
        if (!unified.taskRetryFallbackEligibility) throw new Error(t("shared.unifiedClient.commandUnavailable", { cmd }));
        return unified.taskRetryFallbackEligibility((args?.taskIds as string[]) ?? []);
      case "check_existing_downloaded_scenes":
        if (!unified.checkExistingDownloadedScenes) throw new Error(t("shared.unifiedClient.commandUnavailable", { cmd }));
        return unified.checkExistingDownloadedScenes(
          (args?.items as ExistingDownloadCheckItem[]) ?? [],
          (args?.targetDir as string | null | undefined) ?? null
        );
      case "application_task_overview":
        return unified.taskOverview();
      case "application_account_session_statuses":
        return unified.accountSessionStatuses();
      case "application_test_network_connection":
        return unified.testNetworkConnection();
      case "application_preview_all_match_task":
        if (!unified.previewAllMatchTask) throw new Error(t("shared.unifiedClient.commandUnavailable", { cmd }));
        return unified.previewAllMatchTask(args?.request as BatchAllMatchPreviewRequest);
      case "application_create_all_match_task":
        if (!unified.createAllMatchTask) throw new Error(t("shared.unifiedClient.commandUnavailable", { cmd }));
        return unified.createAllMatchTask(args?.request as BatchAllMatchCreateTaskRequest);
      default:
        throw new Error(`Unknown unified command: ${cmd}`);
    }
  };
}

export class UnifiedClient {
  private invoke: InvokeFn;
  private inFlightPromises = new Map<string, Promise<unknown>>();

  constructor(dependencies?: UnifiedClientDependencies) {
    this.invoke = dependencies?.invoke ?? defaultInvoke();
  }

  private deduplicateInFlight<T>(key: string, factory: () => Promise<T>): Promise<T> {
    const existing = this.inFlightPromises.get(key);
    if (existing) {
      return existing as Promise<T>;
    }
    const promise = factory().finally(() => {
      this.inFlightPromises.delete(key);
    });
    this.inFlightPromises.set(key, promise);
    return promise;
  }

  public async listSources(): Promise<SourceDescriptor[]> {
    return this.deduplicateInFlight("application_list_sources", async () => {
      return (await this.invoke("application_list_sources")) as SourceDescriptor[];
    });
  }

  public async sourceViews(): Promise<SourceView[]> {
    return this.deduplicateInFlight("application_source_views", async () => {
      return (await this.invoke("application_source_views")) as SourceView[];
    });
  }

  public async searchStart(
    request: UnifiedSearchRequest,
    options?: { bypassCache?: boolean },
  ): Promise<SearchProgress> {
    if (!request.aoi_name || typeof request.aoi_name !== "string" || !request.aoi_name.trim()) {
      throw new Error("AOI_NAME_REQUIRED");
    }
    return (await this.invoke(
      "application_search_start",
      options?.bypassCache ? { request, bypassCache: true } : { request },
    )) as SearchProgress;
  }

  public async searchProgress(requestId: string): Promise<SearchProgress | null> {
    const res = await this.invoke("application_search_progress", { requestId });
    return (res as SearchProgress | null) ?? null;
  }

  public async searchCancel(requestId: string): Promise<boolean> {
    return (await this.invoke("application_search_cancel", { requestId })) as boolean;
  }

  /** #22 B slice 3(c): cancel a catalogue search; arrived rows are kept. */
  public async cancelCatalogSearch(requestId: string): Promise<SearchCancelOutcome> {
    return (await this.invoke("application_cancel_catalog_search", { requestId })) as SearchCancelOutcome;
  }

  public async batchPage(
    batchId: string,
    revision: number,
    page: number,
    pageSize: number
  ): Promise<StoreBatchPage> {
    const key = `application_batch_page:${batchId}:${revision}:${page}:${pageSize}`;
    return this.deduplicateInFlight(key, async () => {
      return (await this.invoke("application_batch_page", {
        batchId,
        revision,
        page,
        pageSize,
      })) as StoreBatchPage;
    });
  }

  public async createTask(
    batchId: string,
    revision: number,
    itemIds: string[],
    options: CreateTaskOptions
  ): Promise<TaskRef> {
    return (await this.invoke("application_create_task", {
      batchId,
      revision,
      itemIds,
      options,
    })) as TaskRef;
  }

  public async taskAction(
    taskId: string,
    revision: number,
    action: "pause" | "resume" | "retry" | "cancel" | "archive"
  ): Promise<TaskRef> {
    return (await this.invoke("application_task_action", {
      taskId,
      revision,
      action,
    })) as TaskRef;
  }

  public async taskRetryFallback(
    taskId: string,
    confirmRefreshedBatchId: string | null = null
  ): Promise<TaskRetryFallbackOutcome> {
    return (await this.invoke("application_task_retry_fallback", {
      taskId,
      confirmRefreshedBatchId,
    })) as TaskRetryFallbackOutcome;
  }

  public async taskItemAction(
    taskId: string,
    revision: number | null,
    itemId: string,
    action: UnifiedTaskItemAction
  ): Promise<TaskRef> {
    return (await this.invoke("application_task_item_action", {
      taskId,
      revision,
      itemId,
      action,
    })) as TaskRef;
  }

  public async taskDeleteHistory(taskId: string): Promise<void> {
    await this.invoke("application_task_delete_history", { taskId });
  }

  public async taskSetPinned(taskId: string, pinned: boolean): Promise<string[]> {
    return (await this.invoke("application_task_set_pinned", { taskId, pinned })) as string[];
  }

  public async listPinnedTasks(): Promise<string[]> {
    return (await this.invoke("application_list_pinned_tasks")) as string[];
  }

  public async taskRetryFallbackEligibility(
    taskIds: string[]
  ): Promise<Record<string, boolean>> {
    return (await this.invoke("application_task_retry_fallback_eligibility", {
      taskIds,
    })) as Record<string, boolean>;
  }

  public async checkExistingDownloadedScenes(
    items: ExistingDownloadCheckItem[],
    targetDir: string | null = null
  ): Promise<ExistingDownloadMatch[]> {
    return (await this.invoke("check_existing_downloaded_scenes", {
      items,
      targetDir,
    })) as ExistingDownloadMatch[];
  }

  public async taskOverview(): Promise<TaskOverview> {
    const raw = await this.invoke("application_task_overview");
    return parseTaskOverview(raw);
  }

  public async accountSessionStatuses(): Promise<AccountSessionStatus[]> {
    const raw = await this.invoke("application_account_session_statuses");
    return parseAccountSessionStatuses(raw);
  }

  public async previewAllMatchTask(request: BatchAllMatchPreviewRequest): Promise<BatchAllMatchPreview> {
    return (await this.invoke("application_preview_all_match_task", { request })) as BatchAllMatchPreview;
  }

  public async createAllMatchTask(request: BatchAllMatchCreateTaskRequest): Promise<BatchAllMatchTaskOutcome> {
    return (await this.invoke("application_create_all_match_task", { request })) as BatchAllMatchTaskOutcome;
  }

  public async testNetworkConnection(): Promise<string> {
    return (await this.invoke("application_test_network_connection")) as string;
  }
}

export const defaultUnifiedClient = new UnifiedClient();
