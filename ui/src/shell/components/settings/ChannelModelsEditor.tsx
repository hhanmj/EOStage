import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import {
  ArrowDown,
  ArrowUp,
  Check,
  ChevronDown,
  Download,
  HelpCircle,
  Info,
  Loader2,
  Lock,
  Pencil,
  Plug,
  Plus,
  RefreshCw,
  Search,
  SlidersHorizontal,
  Trash2,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { t, useI18n } from "@/i18n";
import { isTauriDesktopHost } from "@/lib/bridge.ts";
import { HoverHintLabel } from "@/components/desktop/HoverHintLabel.tsx";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import { SpSelect } from "@/components/ui/SpSelect";
import type { AgentChannelModel, AgentReasoningLevel } from "@/lib/generated/v3-contracts";
import {
  isOfficialChannelId,
  mergeChannelModels,
  moveChannelModel,
  newChannelModel,
  reasoningLevelPresets,
  filterDiscoveredModels,
} from "../../api/channelModels";
import { CHANNEL_PROTOCOLS } from "../../api/agentChannelsLogic";
import { matchSpecificPreset, matchModelPreset } from "../../api/modelPresets";
import {
  getPublicModelRegistryState,
  subscribePublicModelRegistry,
  syncPublicModelRegistry,
} from "../../api/publicModelRegistry";

import { useDelayedFlag, useFullTurnSpin } from "@/hooks/useLoadingPrimitives";
import { emitTopToast } from "@/components/ui/TopCenterToast";
import { OptimisticInlineStatus } from "@/components/ui/OptimisticInlineStatus";
import { Skeleton } from "@/components/ui/Skeleton";
import { loadDisabledChannelModels, persistModelEnabled } from "@/lib/channelPreferences";

function FetchModelsDelayedSkeleton() {
  useI18n();
  const showSkeleton = useDelayedFlag(true);
  if (!showSkeleton) {
    return <div className="h-36" aria-busy="true" aria-label={t("settings.channels.preparingModelsAria")} />;
  }
  return (
    <div className="space-y-1.5 py-1" aria-busy="true" aria-label={t("settings.channels.loadingModelsAria")}>
      {Array.from({ length: 4 }, (_, index) => (
        <Skeleton key={index} className="h-9 w-full rounded-sp-sm" />
      ))}
    </div>
  );
}

export interface ChannelModelsEditorProps {
  channelId?: string;
  models: AgentChannelModel[];
  protocol: string;
  disabled?: boolean;
  fetching?: boolean;
  discoveredCount?: number;
  onFetch: () => void;
  onChange: (models: AgentChannelModel[]) => void;
  onTestModel?: (modelId: string) => void;
  /** 正在测速的模型 ID（未在测速时为 null），用于按钮内转圈与禁用反馈。 */
  testingModelId?: string | null;
  fetchButtonLabel?: string;
  fetchButtonRef?: React.RefObject<HTMLButtonElement | null>;
}

const iconButtonClass =
  "flex h-7 w-7 shrink-0 items-center justify-center rounded-sp-sm text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed outline-none focus-visible:ring-1 focus-visible:ring-sp-accent";

function formatContextBadge(tokens: number): string {
  if (tokens >= 1_000_000 && tokens % 1_000_000 === 0) {
    return `${tokens / 1_000_000}M`;
  }
  if (tokens >= 1_048_576 && tokens % 1_048_576 === 0) {
    return `${tokens / 1_048_576}M`;
  }
  return `${Math.round(tokens / 1024)}K`;
}

export function ChannelModelsEditor({
  channelId,
  models,
  protocol,
  disabled = false,
  fetching = false,
  discoveredCount = 0,
  onFetch,
  onChange,
  onTestModel,
  testingModelId = null,
  fetchButtonLabel,
  fetchButtonRef,
}: ChannelModelsEditorProps) {
  useI18n();
  const resolvedFetchButtonLabel = fetchButtonLabel ?? t("settings.channels.identifyModels");
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [isCreatingModal, setIsCreatingModal] = useState(false);
  const [disabledModels, setDisabledModels] = useState<Record<string, boolean>>(() =>
    channelId ? loadDisabledChannelModels(channelId) : {},
  );
  const [modelPrefStatus, setModelPrefStatus] = useState<{
    modelKey: string;
    state: "idle" | "saving" | "saved" | "error";
    errorMessage?: string;
    failedNextEnabled?: boolean;
  }>({ modelKey: "", state: "idle" });

  React.useEffect(() => {
    setDisabledModels(channelId ? loadDisabledChannelModels(channelId) : {});
    setModelPrefStatus({ modelKey: "", state: "idle" });
  }, [channelId]);

  const handleToggleModelEnabled = React.useCallback(
    async (modelKey: string, nextEnabled: boolean) => {
      setDisabledModels((prev) => ({ ...prev, [modelKey]: !nextEnabled }));
      if (!channelId) {
        return;
      }
      setModelPrefStatus({ modelKey, state: "saving" });
      try {
        await persistModelEnabled(channelId, modelKey, nextEnabled);
        setModelPrefStatus({ modelKey, state: "saved" });
      } catch (err) {
        setDisabledModels((prev) => ({ ...prev, [modelKey]: nextEnabled }));
        setModelPrefStatus({
          modelKey,
          state: "error",
          errorMessage: err instanceof Error ? err.message : t("settings.channels.saveToggleFailed"),
          failedNextEnabled: nextEnabled,
        });
      }
    },
    [channelId],
  );

  const showFetchSpinner = useDelayedFlag(fetching);
  const triggerRef = useRef<HTMLElement | null>(null);
  const addButtonRef = useRef<HTMLButtonElement>(null);
  const editingModel = editingIndex === null ? null : models[editingIndex] ?? null;

  const closeEditor = useCallback(() => {
    setEditingIndex(null);
    setIsCreatingModal(false);
  }, []);

  const addModel = (event: React.MouseEvent<HTMLButtonElement>) => {
    triggerRef.current = event.currentTarget;
    onChange([...models, newChannelModel("", protocol)]);
    setIsCreatingModal(true);
    setEditingIndex(models.length);
  };

  const handleCancelEdit = useCallback(() => {
    if (isCreatingModal && editingIndex !== null && !models[editingIndex]?.id?.trim()) {
      onChange(models.filter((_, i) => i !== editingIndex));
    }
    closeEditor();
  }, [closeEditor, editingIndex, isCreatingModal, models, onChange]);

  const deleteModel = () => {
    if (editingIndex === null) return;
    triggerRef.current = addButtonRef.current;
    onChange(models.filter((_, index) => index !== editingIndex));
    closeEditor();
  };
  const deleteModelAt = (index: number) => {
    if (editingIndex === index) closeEditor();
    onChange(models.filter((_, i) => i !== index));
  };
  const moveModel = (offset: -1 | 1) => {
    if (editingIndex === null) return;
    onChange(moveChannelModel(models, editingIndex, offset));
    setEditingIndex(editingIndex + offset);
  };

  const visibleModels = models.filter(
    (m, idx) => !(isCreatingModal && idx === editingIndex && !m.id.trim())
  );

  return (
    <div aria-label={t("settings.channels.models")} className="space-y-2 pt-1">
      {/* 紧凑模型栏头部：左侧「模型列表」，右侧「识别渠道模型 (DSH)」+「+ 添加模型 (EOStage)」 */}
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="text-[13px] text-sp-text-secondary">
            {t("settings.channels.models")}
          </span>
          {visibleModels.length > 0 && (
            <span className="eo-settings-badge font-mono">
              {visibleModels.length}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            ref={fetchButtonRef}
            type="button"
            aria-label={t("settings.channels.fetchAvailable")}
            aria-busy={fetching}
            title={t("settings.channels.identifyModelsTooltip")}
            disabled={disabled || fetching}
            onClick={onFetch}
            className="eo-settings-btn-secondary min-w-[124px] justify-center disabled:opacity-50 outline-none"
          >
            {showFetchSpinner ? (
              <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
            ) : (
              <Download className="h-3.5 w-3.5 shrink-0" />
            )}
            <span>{resolvedFetchButtonLabel}</span>
            {discoveredCount > 0 && (
              <span className="rounded-full bg-sp-surface-muted px-1 font-mono text-xs text-sp-text">
                {discoveredCount}
              </span>
            )}
          </button>
          <button
            ref={addButtonRef}
            type="button"
            aria-label={t("settings.channels.addModel")}
            disabled={disabled}
            onClick={addModel}
            className="eo-settings-btn-secondary bg-sp-surface-muted disabled:opacity-50 outline-none"
          >
            <Plus className="h-3.5 w-3.5" />
            <span>{t("settings.channels.addModel")}</span>
          </button>
        </div>
      </div>

      {/* 空状态虚线框（统一交互布局 图 2） vs 紧凑模型行列表 */}
      {visibleModels.length === 0 ? (
        <div className="flex items-center gap-2 rounded-sp-md border border-dashed border-sp-line bg-sp-surface px-3 py-3 text-xs text-sp-text-muted">
          <Info className="h-4 w-4 shrink-0" />
          <span>{t("settings.channels.emptyModelsHint")}</span>
        </div>
      ) : (
        <div className="rounded-sp-md border border-sp-line bg-sp-card divide-y divide-sp-divider overflow-hidden">
          {models.map((model, index) => {
            if (isCreatingModal && index === editingIndex && !model.id.trim()) {
              return null;
            }
            const label = model.name?.trim() || model.id.trim() || t("settings.channels.unnamed");
            const modelKey = model.id || `idx-${index}`;
            const isModelEnabled = !disabledModels[modelKey];
            const isEditing = editingIndex === index;
            const effectiveContextWindow =
              model.context_window != null && model.context_window > 0
                ? model.context_window
                : 131072;
            return (
              <div
                key={index}
                className={cn(
                  "flex min-h-10 items-center justify-between gap-2 px-3 py-2 transition-colors hover:bg-sp-hover",
                  isEditing && "bg-sp-surface-muted"
                )}
              >
                <div className="flex min-w-0 flex-1 items-center gap-2 flex-wrap">
                  <span
                    title={label}
                    className="min-w-0 truncate text-[13px] font-semibold text-sp-text"
                  >
                    {label}
                  </span>
                  {model.id && model.name && model.name.trim() !== model.id.trim() && (
                    <span className="shrink-0 font-mono text-xs text-sp-text-muted">
                      {model.id}
                    </span>
                  )}
                  <span className="eo-settings-badge shrink-0 font-mono">
                    {formatContextBadge(effectiveContextWindow)}
                  </span>
                  {model.input_types?.includes("image") && (
                    <span className="eo-settings-badge shrink-0">
                      {t("settings.channels.visionBadge")}
                    </span>
                  )}
                  {isOfficialChannelId(channelId) && model.is_free === true && (
                    <span
                      data-testid={`model-free-badge-${index}`}
                      className="eo-settings-badge eo-settings-badge-free shrink-0"
                    >
                      Free
                    </span>
                  )}
                </div>

                {/* 右侧操作按钮组 */}
                <div className="flex shrink-0 items-center gap-1">
                  {onTestModel && (
                    <button
                      type="button"
                      title={
                        testingModelId === model.id
                          ? t("settings.channels.testingModelTitle", { name: label })
                          : t("settings.channels.testModelTitle", { name: label })
                      }
                      aria-label={t("settings.channels.testModelTitle", { name: label })}
                      aria-busy={testingModelId === model.id}
                      data-testid={`model-test-btn-${index}`}
                      disabled={disabled || testingModelId !== null}
                      onClick={() => onTestModel(model.id)}
                      className={cn(iconButtonClass, "active:scale-95 active:bg-sp-hover transition-transform")}
                    >
                      {testingModelId === model.id ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <Plug className="h-3.5 w-3.5" />
                      )}
                    </button>
                  )}
                  <button
                    type="button"
                    aria-label={t("settings.channels.configureModel", { name: label })}
                    title={t("settings.channels.modelSettings")}
                    disabled={disabled}
                    onClick={(event) => {
                      triggerRef.current = event.currentTarget;
                      setIsCreatingModal(false);
                      setEditingIndex(isEditing ? null : index);
                    }}
                    className={iconButtonClass}
                  >
                    <Pencil className="h-3.5 w-3.5" />
                  </button>
                  <button
                    type="button"
                    title={t("settings.channels.removeModelTitle", { name: label })}
                    aria-label={t("settings.channels.removeModelTitle", { name: label })}
                    disabled={disabled}
                    onClick={() => deleteModelAt(index)}
                    className="flex h-7 w-7 shrink-0 items-center justify-center rounded-sp-sm text-sp-danger-text hover:bg-sp-danger-bg transition-colors duration-fast ease-sp cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={isModelEnabled}
                    aria-label={t("settings.channels.enableModelAria", { name: label })}
                    disabled={disabled}
                    onClick={() => void handleToggleModelEnabled(modelKey, !isModelEnabled)}
                    className="eo-settings-switch ml-1"
                  >
                    <span className="eo-settings-switch-thumb" />
                  </button>
                  {modelPrefStatus.modelKey === modelKey && (
                    <OptimisticInlineStatus
                      state={modelPrefStatus.state}
                      errorMessage={modelPrefStatus.errorMessage}
                      onRetry={
                        modelPrefStatus.failedNextEnabled !== undefined
                          ? () =>
                              void handleToggleModelEnabled(
                                modelKey,
                                modelPrefStatus.failedNextEnabled!,
                              )
                          : undefined
                      }
                    />
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* 统一风格「添加模型 / 编辑模型」智能配置弹窗（对齐图 3） */}
      {editingIndex !== null && (
        <ChannelModelInlineEditor
          model={editingModel}
          modelIndex={editingIndex}
          modelCount={models.length}
          protocol={protocol}
          disabled={disabled}
          isCreating={isCreatingModal}
          onCancel={handleCancelEdit}
          onDone={(updated) => {
            onChange(models.map((item, i) => (i === editingIndex ? updated : item)));
            closeEditor();
          }}
          onMove={moveModel}
          onDelete={deleteModel}
          triggerRef={triggerRef}
        />
      )}
    </div>
  );
}

interface ChannelModelInlineEditorProps {
  model: AgentChannelModel | null;
  modelIndex: number;
  modelCount: number;
  protocol: string;
  disabled: boolean;
  isCreating?: boolean;
  onCancel: () => void;
  onDone: (model: AgentChannelModel) => void;
  onMove: (offset: -1 | 1) => void;
  onDelete: () => void;
  triggerRef: React.RefObject<HTMLElement | null>;
}

function defaultReasoningMappingForProtocol(protocol: string): string {
  return protocol === "anthropic_messages"
    ? '{"thinking": {"type": "enabled", "budget_tokens": "{{value}}"}}'
    : '{"reasoning_effort": "{{value}}"}';
}

/**
 * 统一风格「添加模型 / 编辑模型」智能配置弹窗（支持公开模型库同步、全字段自由手改、完整悬停说明与一键重置表单）
 */
function ChannelModelInlineEditor({
  model,
  modelIndex,
  modelCount,
  protocol,
  disabled,
  isCreating = false,
  onCancel,
  onDone,
  onMove,
  onDelete,
  triggerRef,
}: ChannelModelInlineEditorProps) {
  useI18n();
  const fieldUid = useId();
  const cardRef = useRef<HTMLDivElement>(null);
  const idInputRef = useRef<HTMLInputElement>(null);
  const initialModelRef = useRef<AgentChannelModel | null>(model ? { ...model } : null);
  const lastModelIndexRef = useRef<number>(modelIndex);

  const [draft, setDraft] = useState<AgentChannelModel>(() => model ?? newChannelModel("", protocol));
  const [smartConfigEnabled, setSmartConfigEnabled] = useState<boolean>(true);
  const [manualFields, setManualFields] = useState<Record<string, boolean>>({});
  const [advancedOpen, setAdvancedOpen] = useState<boolean>(true);
  const [resetFeedback, setResetFeedback] = useState<boolean>(false);
  const [registryState, setRegistryState] = useState(() => getPublicModelRegistryState());
  const publicSyncSpin = useFullTurnSpin(registryState.status === "syncing");

  // 仅当切换编辑另一个模型索引时才重新载入初始模型，防止父组件重渲染覆盖正在输入的内容
  useEffect(() => {
    if (lastModelIndexRef.current !== modelIndex && model) {
      lastModelIndexRef.current = modelIndex;
      initialModelRef.current = { ...model };
      setDraft({ ...model });
      setManualFields({});
    }
  }, [modelIndex, model]);

  // 订阅公开大模型参数库状态，并在真实桌面环境下自动后台同步最新模型元数据
  useEffect(() => {
    const unsub = subscribePublicModelRegistry(() => {
      setRegistryState(getPublicModelRegistryState());
    });
    if (isTauriDesktopHost()) {
      void syncPublicModelRegistry(false);
    }
    return unsub;
  }, []);

  const [confirmDiscardOpen, setConfirmDiscardOpen] = useState<boolean>(false);

  const hasUnsavedEdits = useMemo(() => {
    const baseline = initialModelRef.current ?? model;
    if (!baseline) return false;
    return JSON.stringify(draft) !== JSON.stringify(baseline);
  }, [draft, model]);

  const handleSafeCancel = useCallback(() => {
    if (!hasUnsavedEdits || isCreating) {
      onCancel();
      return;
    }
    setConfirmDiscardOpen(true);
  }, [hasUnsavedEdits, isCreating, onCancel]);

  const confirmDiscardOpenRef = useRef(confirmDiscardOpen);
  confirmDiscardOpenRef.current = confirmDiscardOpen;
  const handleSafeCancelRef = useRef(handleSafeCancel);
  handleSafeCancelRef.current = handleSafeCancel;

  // 仅在弹窗首次挂载与卸载时管理 Escape 监听，绝不自动聚焦输入框或弹出下拉框抢夺焦点
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        if (confirmDiscardOpenRef.current) {
          setConfirmDiscardOpen(false);
          return;
        }
        handleSafeCancelRef.current();
      }
    };
    document.addEventListener("keydown", keydown);
    return () => {
      document.removeEventListener("keydown", keydown);
      triggerRef.current?.focus();
    };
  }, [triggerRef]);

  if (!model) return null;

  const update = (change: Partial<AgentChannelModel>, fieldKey?: string) => {
    if (fieldKey) {
      setManualFields((prev) => (prev[fieldKey] ? prev : { ...prev, [fieldKey]: true }));
    }
    setDraft((previous) => ({ ...previous, ...change }));
  };

  const updateLevels = (levels: AgentReasoningLevel[]) =>
    update({ reasoning_levels: levels }, "reasoning_levels");

  const specificMatch = useMemo(
    () => matchSpecificPreset(draft.id, protocol),
    [draft.id, protocol, registryState.updatedAt, registryState.count]
  );

  const effectiveProtocol = (draft.protocol?.trim() || protocol);

  const buildInputTypesWithPreset = (
    baseTypes: readonly ("text" | "image" | "video" | "pdf")[] | undefined,
    withVideo?: boolean,
    withPdf?: boolean,
  ): ("text" | "image" | "video" | "pdf")[] => {
    const set = new Set<"text" | "image" | "video" | "pdf">(baseTypes ?? ["text"]);
    set.add("text");
    if (withVideo) set.add("video");
    if (withPdf) set.add("pdf");
    return (["text", "image", "video", "pdf"] as const).filter((k) => set.has(k));
  };

  const toggleMediaInputType = (kind: "image" | "video" | "pdf", enabled: boolean) => {
    const current = new Set<"text" | "image" | "video" | "pdf">(draft.input_types ?? ["text"]);
    current.add("text");
    if (enabled) current.add(kind);
    else current.delete(kind);
    const ordered = (["text", "image", "video", "pdf"] as const).filter((k) => current.has(k));
    update({ input_types: ordered }, "input_types");
  };

  /**
   * 当用户输入或修改模型 ID 时，如果开启了「智能配置」且命中公开库或内置规则，实时自动套用未手动锁定的推荐参数
   */
  const handleIdChange = (newId: string) => {
    const specific = matchSpecificPreset(newId, effectiveProtocol);
    if (smartConfigEnabled && specific.matched) {
      setDraft((previous) => {
        const prevSpecific = matchSpecificPreset(previous.id, effectiveProtocol);
        const isAutoName =
          !manualFields.name &&
          (!previous.name ||
            previous.name.trim() === "" ||
            previous.name === previous.id ||
            (prevSpecific.matched && previous.name === prevSpecific.preset.name));
        return {
          ...previous,
          id: newId,
          name: isAutoName ? specific.preset.name || newId : previous.name,
          context_window: manualFields.context_window
            ? previous.context_window
            : (specific.preset.context_window ?? previous.context_window),
          max_output_tokens: manualFields.max_output_tokens
            ? previous.max_output_tokens
            : (specific.preset.max_output_tokens ?? previous.max_output_tokens),
          supports_tools: manualFields.supports_tools
            ? previous.supports_tools
            : (specific.preset.supports_tools ?? previous.supports_tools),
          input_types: manualFields.input_types
            ? previous.input_types
            : buildInputTypesWithPreset(
                specific.preset.input_types ?? previous.input_types,
                specific.supports_video,
                specific.supports_pdf,
              ),
          reasoning_levels: manualFields.reasoning_levels
            ? previous.reasoning_levels
            : specific.preset.reasoning_levels
              ? [...specific.preset.reasoning_levels]
              : previous.reasoning_levels,
        };
      });
    } else {
      setDraft((previous) => ({ ...previous, id: newId }));
    }
  };

  /**
   * 手动点击「应用预设」（位于思考档位栏，更新思考档位预设）
   */
  const handleApplyPreset = () => {
    const preset = matchModelPreset(draft.id, effectiveProtocol);
    const levels =
      preset.reasoning_levels && preset.reasoning_levels.length > 0
        ? preset.reasoning_levels
        : reasoningLevelPresets(effectiveProtocol);
    setDraft((previous) => ({ ...previous, reasoning_levels: levels }));
  };

  /**
   * 底部「重置表单」：恢复初始模型 ID（编辑已有模型时）或清空新建表单手改项，并按最新公开模型库/预设重新填充全部字段
   */
  const handleResetForm = () => {
    const baselineId =
      !isCreating && initialModelRef.current?.id?.trim()
        ? initialModelRef.current.id.trim()
        : draft.id.trim();
    const specific = matchSpecificPreset(baselineId, protocol);
    const preset = matchModelPreset(baselineId, protocol);
    setManualFields({});
    setSmartConfigEnabled(true);
    const preservedFree = draft.is_free ?? initialModelRef.current?.is_free;
    setDraft({
      id: baselineId,
      name: baselineId ? preset.name || baselineId : "",
      protocol: null,
      context_window: preset.context_window ?? 128000,
      max_output_tokens: preset.max_output_tokens ?? 16384,
      supports_tools: preset.supports_tools ?? true,
      supports_structured_output: true,
      supports_web_search: false,
      supports_mid_turn_system: true,
      ...(typeof preservedFree === "boolean" ? { is_free: preservedFree } : {}),
      reasoning_param_mapping: null,
      input_types: buildInputTypesWithPreset(
        preset.input_types ?? ["text"],
        specific.supports_video,
        specific.supports_pdf,
      ),
      reasoning_levels: [...(preset.reasoning_levels ?? [])],
    });
    setResetFeedback(true);
    window.setTimeout(() => setResetFeedback(false), 2200);
  };

  const handleSyncPublicRegistry = async () => {
    const next = await syncPublicModelRegistry(true);
    emitTopToast({
      text:
        next.status === "ready"
          ? t("settings.channels.publicSyncOk", { count: next.count })
          : t("settings.channels.publicSyncFailed"),
      tone: next.status === "ready" ? "success" : "danger",
    });
    if (next.status === "ready" && smartConfigEnabled && draft.id.trim()) {
      const specific = matchSpecificPreset(draft.id, effectiveProtocol);
      if (specific.matched) {
        setDraft((prev) => ({
          ...prev,
          name: manualFields.name ? prev.name : (specific.preset.name || prev.name),
          context_window: manualFields.context_window
            ? prev.context_window
            : (specific.preset.context_window ?? prev.context_window),
          max_output_tokens: manualFields.max_output_tokens
            ? prev.max_output_tokens
            : (specific.preset.max_output_tokens ?? prev.max_output_tokens),
          supports_tools: manualFields.supports_tools
            ? prev.supports_tools
            : (specific.preset.supports_tools ?? prev.supports_tools),
          input_types: manualFields.input_types
            ? prev.input_types
            : buildInputTypesWithPreset(
                specific.preset.input_types ?? ["text"],
                specific.supports_video,
                specific.supports_pdf,
              ),
          reasoning_levels: manualFields.reasoning_levels
            ? prev.reasoning_levels
            : [...(specific.preset.reasoning_levels ?? [])],
        }));
      }
    }
  };

  const label = model.name?.trim() || model.id.trim() || t("settings.channels.modelSettings");
  const isImageEnabled = draft.input_types?.includes("image") ?? false;
  const isVideoEnabled = draft.input_types?.includes("video") ?? false;
  const isPdfEnabled = draft.input_types?.includes("pdf") ?? false;
  const isToolsEnabled = draft.supports_tools ?? true;
  const isStructuredOutputEnabled = draft.supports_structured_output ?? true;
  const isWebSearchEnabled = draft.supports_web_search ?? false;
  const isMidTurnSystemEnabled = draft.supports_mid_turn_system ?? true;
  const reasoningMappingValue =
    draft.reasoning_param_mapping ?? defaultReasoningMappingForProtocol(effectiveProtocol);

  return (
    <ModalOverlay
      data-testid="model-inline-editor-backdrop"
      closeOnBackdropClick={false}
    >
      <div
        ref={cardRef}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-[88vh] w-full max-w-[520px] flex-col rounded-modal border border-sp-line bg-sp-surface p-5 shadow-modal overflow-hidden"
      >
        {/* 弹窗顶部标题栏 */}
        <div className="flex shrink-0 items-center justify-between gap-3 pb-3 border-b border-sp-divider">
          <span className="truncate text-sm font-semibold text-sp-text">
            {isCreating || !model.id ? t("settings.channels.addModel") : label}
          </span>
          <div className="flex shrink-0 items-center gap-1">
            {!isCreating && modelCount > 1 && (
              <>
                <button
                  type="button"
                  title={t("settings.channels.moveUp")}
                  aria-label={t("settings.channels.moveUpLabel", { index: modelIndex + 1 })}
                  disabled={disabled || modelIndex === 0}
                  onClick={() => onMove(-1)}
                  className={iconButtonClass}
                >
                  <ArrowUp className="h-3.5 w-3.5" />
                </button>
                <button
                  type="button"
                  title={t("settings.channels.moveDown")}
                  aria-label={t("settings.channels.moveDownLabel", { index: modelIndex + 1 })}
                  disabled={disabled || modelIndex === modelCount - 1}
                  onClick={() => onMove(1)}
                  className={iconButtonClass}
                >
                  <ArrowDown className="h-3.5 w-3.5" />
                </button>
              </>
            )}
            <button
              type="button"
              onClick={handleSafeCancel}
              aria-label={t("settings.channels.closeModelEditorAria")}
              className={iconButtonClass}
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </div>

        {/* 智能配置开关行 + 公开模型元数据同步入口 (独立纵向滚动主体) */}
        <div className="flex-1 min-h-0 overflow-y-auto overflow-x-hidden pr-1 py-3 space-y-4">
        <div
          data-testid="smart-model-config-row"
          className="flex flex-wrap items-center justify-between gap-2"
        >
          <div className="flex items-center gap-2">
            <span className="text-[13px] font-semibold text-sp-text">{t("settings.channels.smartConfig")}</span>
            <HoverHintLabel hint={t("settings.channels.smartConfigTooltip")} focusable maxWidthClass="max-w-[260px]">
              <HelpCircle
                className="h-3.5 w-3.5 text-sp-text-muted hover:text-sp-text"
                aria-label={t("settings.channels.smartConfigHelpAria")}
              />
            </HoverHintLabel>
            <button
              type="button"
              role="switch"
              aria-checked={smartConfigEnabled}
              aria-label={t("settings.channels.smartConfig")}
              disabled={disabled}
              onClick={() => {
                const next = !smartConfigEnabled;
                setSmartConfigEnabled(next);
                if (next && draft.id) {
                  const specific = matchSpecificPreset(draft.id, protocol);
                  if (specific.matched) {
                    setDraft((prev) => ({
                      ...prev,
                      context_window: manualFields.context_window
                        ? prev.context_window
                        : specific.preset.context_window,
                      max_output_tokens: manualFields.max_output_tokens
                        ? prev.max_output_tokens
                        : specific.preset.max_output_tokens,
                      supports_tools: manualFields.supports_tools
                        ? prev.supports_tools
                        : specific.preset.supports_tools,
                      input_types: manualFields.input_types
                        ? prev.input_types
                        : [...(specific.preset.input_types ?? ["text"])],
                      reasoning_levels: manualFields.reasoning_levels
                        ? prev.reasoning_levels
                        : [...(specific.preset.reasoning_levels ?? [])],
                    }));
                  }
                }
              }}
              className="eo-settings-switch"
            >
              <span className="eo-settings-switch-thumb" />
            </button>
          </div>

          <div className="flex items-center gap-1.5">
            {smartConfigEnabled && specificMatch.matched && (
              <span className="eo-settings-badge">
                {specificMatch.source === "public_api"
                  ? t("settings.channels.publicRegistryMatched")
                  : t("settings.channels.presetMatched")}
                {specificMatch.preset.name}
              </span>
            )}
            <button
              type="button"
              data-testid="sync-public-models-btn"
              aria-busy={registryState.status === "syncing"}
              disabled={disabled || registryState.status === "syncing"}
              onClick={() => void handleSyncPublicRegistry()}
              title={t("settings.channels.syncPublicModelsTooltip")}
              className="inline-flex h-6 min-w-[108px] items-center justify-center gap-1 rounded-sp-sm border border-sp-line bg-sp-card px-2 text-xs text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text cursor-pointer disabled:opacity-50"
            >
              <RefreshCw
                className={cn(
                  "h-3 w-3 shrink-0",
                  publicSyncSpin.spinClass
                )}
              />
              <span>
                {registryState.count > 0
                  ? t("settings.channels.publicRegistryCount", { count: registryState.count })
                  : t("settings.channels.syncPublicRegistry")}
              </span>
            </button>
          </div>
        </div>

        {/* 核心参数区：模型 ID、显示名称、上下文窗口、最大输出 Token */}
        <div className="space-y-3">
          <div className="relative space-y-1 text-[13px] text-sp-text-secondary">
            <label htmlFor={`${fieldUid}-id`} className="block">
              {t("settings.channels.modelId")}
            </label>
            <input
              id={`${fieldUid}-id`}
              ref={idInputRef}
              aria-label={t("settings.channels.modelIdLabel", { index: modelIndex + 1 })}
              placeholder={t("settings.channels.modelIdPlaceholder")}
              value={draft.id}
              disabled={disabled}
              onChange={(event) => handleIdChange(event.target.value)}
              className="eo-settings-control-input h-9"
            />
          </div>

          <div className="space-y-1 text-[13px] text-sp-text-secondary">
            <label htmlFor={`${fieldUid}-name`} className="block">
              {t("settings.channels.displayName")}
            </label>
            <input
              id={`${fieldUid}-name`}
              aria-label={t("settings.channels.displayNameLabel", { index: modelIndex + 1 })}
              placeholder={t("settings.channels.displayNamePlaceholder")}
              value={draft.name ?? ""}
              disabled={disabled}
              onChange={(event) => update({ name: event.target.value }, "name")}
              className="eo-settings-control-input h-9"
            />
          </div>

          <div className="space-y-1 text-[13px] text-sp-text-secondary">
            <label htmlFor={`${fieldUid}-protocol`} className="block">
              {t("settings.channels.protocolOverride")}
            </label>
            <SpSelect
              data-testid="model-protocol-override-select"
              aria-label={t("settings.channels.protocolOverrideAria", { index: modelIndex + 1 })}
              value={draft.protocol ?? ""}
              disabled={disabled}
              onChange={(val) => update({ protocol: val || null }, "protocol")}
              triggerClassName="h-9 text-[13px]"
              options={[
                {
                  value: "",
                  label: t("settings.channels.followProviderProtocol", {
                    protocol: CHANNEL_PROTOCOLS.find((p) => p.value === protocol)?.label_zh ?? protocol,
                  }),
                },
                ...CHANNEL_PROTOCOLS.map((opt) => ({
                  value: opt.value,
                  label: opt.label_zh,
                })),
              ]}
            />
          </div>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="space-y-1 text-[13px] text-sp-text-secondary">
              <div className="flex items-center gap-1">
                <label htmlFor={`${fieldUid}-ctx`}>{t("settings.channels.contextWindow")}</label>
                <HoverHintLabel hint={t("settings.channels.contextWindowTooltip")} focusable maxWidthClass="max-w-[240px]">
                  <HelpCircle
                    className="h-3.5 w-3.5 text-sp-text-muted hover:text-sp-text"
                    aria-label={t("settings.channels.contextWindowHelpAria")}
                  />
                </HoverHintLabel>
              </div>
              <input
                id={`${fieldUid}-ctx`}
                aria-label={t("settings.channels.contextWindowLabel", { index: modelIndex + 1 })}
                type="number"
                min="1"
                step="1024"
                value={draft.context_window ?? ""}
                disabled={disabled}
                onChange={(event) => {
                  const raw = event.target.value.trim();
                  const parsed = raw === "" ? null : Number(raw);
                  update(
                    {
                      context_window:
                        parsed !== null && Number.isFinite(parsed) && parsed > 0
                          ? Math.round(parsed)
                          : null,
                    },
                    "context_window"
                  );
                }}
                className="eo-settings-control-input h-9"
              />
            </div>

            <div className="space-y-1 text-[13px] text-sp-text-secondary">
              <div className="flex items-center gap-1">
                <label htmlFor={`${fieldUid}-maxout`}>{t("settings.channels.maxOutput")}</label>
                <HoverHintLabel
                  hint={t("settings.channels.maxOutputTooltip")}
                  focusable
                  align="end"
                  maxWidthClass="max-w-[240px]"
                >
                  <HelpCircle
                    className="h-3.5 w-3.5 text-sp-text-muted hover:text-sp-text"
                    aria-label={t("settings.channels.maxOutputHelpAria")}
                  />
                </HoverHintLabel>
              </div>
              <input
                id={`${fieldUid}-maxout`}
                aria-label={t("settings.channels.maxOutputLabel", { index: modelIndex + 1 })}
                type="number"
                min="1"
                step="1024"
                value={draft.max_output_tokens ?? ""}
                disabled={disabled}
                onChange={(event) => {
                  const raw = event.target.value.trim();
                  const parsed = raw === "" ? null : Number(raw);
                  update(
                    {
                      max_output_tokens:
                        parsed !== null && Number.isFinite(parsed) && parsed > 0
                          ? Math.round(parsed)
                          : null,
                    },
                    "max_output_tokens"
                  );
                }}
                className="eo-settings-control-input h-9"
              />
            </div>
          </div>
        </div>

        {/* 高级配置折叠区（统一交互布局 图 3：输入类型胶囊、模型能力胶囊、推理等级、推理参数映射） */}
        <div className="space-y-3 pt-1">
          <button
            type="button"
            onClick={() => setAdvancedOpen((prev) => !prev)}
            className="flex items-center gap-1 text-[13px] font-semibold text-sp-text cursor-pointer"
          >
            <ChevronDown
              className={cn(
                "h-3.5 w-3.5 text-sp-text-secondary transition-transform",
                !advancedOpen && "-rotate-90"
              )}
            />
            <span>{t("settings.channels.advancedConfig")}</span>
          </button>

          {advancedOpen && (
            <div className="space-y-3 pl-1">
              {/* 1. 输入类型 */}
              <div className="space-y-2">
                <div className="flex items-center gap-1 text-[13px] text-sp-text-secondary">
                  <span>{t("settings.channels.inputTypes")}</span>
                  <HoverHintLabel hint={t("settings.channels.inputTypesTooltip")} focusable maxWidthClass="max-w-[260px]">
                    <HelpCircle
                      className="h-3.5 w-3.5 text-sp-text-muted hover:text-sp-text"
                      aria-label={t("settings.channels.inputTypesHelpAria")}
                    />
                  </HoverHintLabel>
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="inline-flex h-8 items-center gap-2 rounded-sp-sm border border-sp-line bg-sp-surface-muted px-3 text-[13px] text-sp-text select-none">
                    <span className="flex h-4 w-4 items-center justify-center rounded-sp-sm bg-sp-accent-fill text-sp-accent-ink">
                      <Check className="h-3 w-3" />
                    </span>
                    <span>{t("settings.channels.textInputType")}</span>
                    <Lock className="h-3 w-3 text-sp-text-muted" />
                  </span>

                  <label
                    className={cn(
                      "inline-flex h-8 cursor-pointer items-center gap-2 rounded-sp-sm border border-sp-line px-3 text-[13px] transition-colors select-none",
                      isImageEnabled
                        ? "bg-sp-surface-muted font-semibold text-sp-text"
                        : "bg-sp-card text-sp-text-secondary hover:bg-sp-hover"
                    )}
                  >
                    <input
                      type="checkbox"
                      aria-label={t("settings.channels.imageInputLabel", { index: modelIndex + 1 })}
                      checked={isImageEnabled}
                      disabled={disabled}
                      onChange={(event) => toggleMediaInputType("image", event.target.checked)}
                      className="h-3.5 w-3.5 accent-sp-accent-fill"
                    />
                    <span>{t("settings.channels.imageInput")}</span>
                  </label>

                  <button
                    type="button"
                    aria-pressed={isVideoEnabled}
                    aria-label={t("settings.channels.videoInputAria", { index: modelIndex + 1 })}
                    disabled={disabled}
                    onClick={() => toggleMediaInputType("video", !isVideoEnabled)}
                    className={cn(
                      "inline-flex h-8 cursor-pointer items-center gap-2 rounded-sp-sm border border-sp-line px-3 text-[13px] transition-colors",
                      isVideoEnabled
                        ? "bg-sp-surface-muted font-semibold text-sp-text"
                        : "bg-sp-card text-sp-text-secondary hover:bg-sp-hover"
                    )}
                  >
                    <span
                      className={cn(
                        "flex h-3.5 w-3.5 items-center justify-center rounded-sp-sm border border-sp-line",
                        isVideoEnabled && "bg-sp-accent-fill text-sp-accent-ink"
                      )}
                    >
                      {isVideoEnabled && <Check className="h-2.5 w-2.5" />}
                    </span>
                    <span>{t("settings.channels.videoInputType")}</span>
                  </button>

                  <button
                    type="button"
                    aria-pressed={isPdfEnabled}
                    aria-label={t("settings.channels.pdfInputAria", { index: modelIndex + 1 })}
                    disabled={disabled}
                    onClick={() => toggleMediaInputType("pdf", !isPdfEnabled)}
                    className={cn(
                      "inline-flex h-8 cursor-pointer items-center gap-2 rounded-sp-sm border border-sp-line px-3 text-[13px] transition-colors",
                      isPdfEnabled
                        ? "bg-sp-surface-muted font-semibold text-sp-text"
                        : "bg-sp-card text-sp-text-secondary hover:bg-sp-hover"
                    )}
                  >
                    <span
                      className={cn(
                        "flex h-3.5 w-3.5 items-center justify-center rounded-sp-sm border border-sp-line",
                        isPdfEnabled && "bg-sp-accent-fill text-sp-accent-ink"
                      )}
                    >
                      {isPdfEnabled && <Check className="h-2.5 w-2.5" />}
                    </span>
                    <span>PDF</span>
                  </button>
                </div>
              </div>

              {/* 2. 模型能力 */}
              <div className="space-y-2">
                <div className="flex items-center gap-1 text-[13px] text-sp-text-secondary">
                  <span>{t("settings.channels.modelCapabilities")}</span>
                  <HoverHintLabel hint={t("settings.channels.modelCapabilitiesTooltip")} focusable maxWidthClass="max-w-[260px]">
                    <HelpCircle
                      className="h-3.5 w-3.5 text-sp-text-muted hover:text-sp-text"
                      aria-label={t("settings.channels.modelCapabilitiesHelpAria")}
                    />
                  </HoverHintLabel>
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  <label
                    className={cn(
                      "inline-flex h-8 cursor-pointer items-center gap-2 rounded-sp-sm border border-sp-line px-3 text-[13px] transition-colors select-none",
                      isToolsEnabled
                        ? "bg-sp-surface-muted font-semibold text-sp-text"
                        : "bg-sp-card text-sp-text-secondary hover:bg-sp-hover"
                    )}
                  >
                    <input
                      type="checkbox"
                      aria-label={t("settings.channels.supportsToolsLabel", { index: modelIndex + 1 })}
                      checked={isToolsEnabled}
                      disabled={disabled}
                      onChange={(event) =>
                        update({ supports_tools: event.target.checked }, "supports_tools")
                      }
                      className="h-3.5 w-3.5 accent-sp-accent-fill"
                    />
                    <span>{t("settings.channels.supportsTools")}</span>
                  </label>

                  <button
                    type="button"
                    aria-pressed={isStructuredOutputEnabled}
                    disabled={disabled}
                    onClick={() =>
                      update(
                        { supports_structured_output: !isStructuredOutputEnabled },
                        "supports_structured_output"
                      )
                    }
                    className={cn(
                      "inline-flex h-8 cursor-pointer items-center gap-2 rounded-sp-sm border border-sp-line px-3 text-[13px] transition-colors",
                      isStructuredOutputEnabled
                        ? "bg-sp-surface-muted font-semibold text-sp-text"
                        : "bg-sp-card text-sp-text-secondary hover:bg-sp-hover"
                    )}
                  >
                    <span
                      className={cn(
                        "flex h-3.5 w-3.5 items-center justify-center rounded-sp-sm border border-sp-line",
                        isStructuredOutputEnabled && "bg-sp-accent-fill text-sp-accent-ink"
                      )}
                    >
                      {isStructuredOutputEnabled && <Check className="h-2.5 w-2.5" />}
                    </span>
                    <span>{t("settings.channels.structuredOutput")}</span>
                  </button>

                  <button
                    type="button"
                    aria-pressed={isWebSearchEnabled}
                    disabled={disabled}
                    onClick={() =>
                      update(
                        { supports_web_search: !isWebSearchEnabled },
                        "supports_web_search"
                      )
                    }
                    className={cn(
                      "inline-flex h-8 cursor-pointer items-center gap-2 rounded-sp-sm border border-sp-line px-3 text-[13px] transition-colors",
                      isWebSearchEnabled
                        ? "bg-sp-surface-muted font-semibold text-sp-text"
                        : "bg-sp-card text-sp-text-secondary hover:bg-sp-hover"
                    )}
                  >
                    <span
                      className={cn(
                        "flex h-3.5 w-3.5 items-center justify-center rounded-sp-sm border border-sp-line",
                        isWebSearchEnabled && "bg-sp-accent-fill text-sp-accent-ink"
                      )}
                    >
                      {isWebSearchEnabled && <Check className="h-2.5 w-2.5" />}
                    </span>
                    <span>{t("settings.channels.nativeWebSearch")}</span>
                  </button>

                  <button
                    type="button"
                    aria-pressed={isMidTurnSystemEnabled}
                    disabled={disabled}
                    onClick={() =>
                      update(
                        { supports_mid_turn_system: !isMidTurnSystemEnabled },
                        "supports_mid_turn_system"
                      )
                    }
                    className={cn(
                      "inline-flex h-8 cursor-pointer items-center gap-2 rounded-sp-sm border border-sp-line px-3 text-[13px] transition-colors",
                      isMidTurnSystemEnabled
                        ? "bg-sp-surface-muted font-semibold text-sp-text"
                        : "bg-sp-card text-sp-text-secondary hover:bg-sp-hover"
                    )}
                  >
                    <span
                      className={cn(
                        "flex h-3.5 w-3.5 items-center justify-center rounded-sp-sm border border-sp-line",
                        isMidTurnSystemEnabled && "bg-sp-accent-fill text-sp-accent-ink"
                      )}
                    >
                      {isMidTurnSystemEnabled && <Check className="h-2.5 w-2.5" />}
                    </span>
                    <span>{t("settings.channels.midTurnSystem")}</span>
                  </button>
                </div>
              </div>

              {/* 3. 推理等级 (从低到高) */}
              <div className="space-y-2">
                <div className="flex items-center justify-between gap-2">
                  <span className="inline-flex items-center gap-1 text-[13px] text-sp-text-secondary">
                    {t("settings.channels.reasoningLevelsHeading")}
                    <HoverHintLabel hint={t("settings.channels.reasoningLevelsTooltip")} focusable maxWidthClass="max-w-[260px]">
                      <HelpCircle
                        className="h-3.5 w-3.5 text-sp-text-muted hover:text-sp-text"
                        aria-label={t("settings.channels.reasoningLevelsHelpAria")}
                      />
                    </HoverHintLabel>
                  </span>
                  <button
                    type="button"
                    disabled={disabled}
                    onClick={handleApplyPreset}
                    className="flex h-7 items-center gap-1 rounded-sp-sm px-2 text-[13px] text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer disabled:opacity-50 outline-none"
                  >
                    <SlidersHorizontal className="h-3.5 w-3.5" />
                    {t("settings.channels.applyPresets")}
                  </button>
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  {(draft.reasoning_levels ?? []).map((level, levelIndex) => (
                    <ReasoningLevelChip
                      key={levelIndex}
                      modelIndex={modelIndex}
                      level={level}
                      disabled={disabled}
                      onChange={(change) =>
                        updateLevels(
                          (draft.reasoning_levels ?? []).map((item, index) =>
                            index === levelIndex ? { ...item, ...change } : item
                          )
                        )
                      }
                      onRemove={() =>
                        updateLevels(
                          (draft.reasoning_levels ?? []).filter((_, index) => index !== levelIndex)
                        )
                      }
                    />
                  ))}
                  <button
                    type="button"
                    aria-label={t("settings.channels.addReasoningLevel", { index: modelIndex + 1 })}
                    disabled={disabled}
                    onClick={() =>
                      updateLevels([...(draft.reasoning_levels ?? []), { level: "", value: "" }])
                    }
                    className="flex h-8 w-8 items-center justify-center rounded-sp-sm border border-sp-line bg-sp-card text-[13px] text-sp-text hover:bg-sp-hover transition-colors cursor-pointer disabled:opacity-50 outline-none"
                  >
                    <Plus className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>

              {/* 4. 推理参数映射 */}
              <div className="space-y-1">
                <span className="inline-flex items-center gap-1 text-[13px] text-sp-text-secondary">
                  {t("settings.channels.reasoningMapping")}
                  <HoverHintLabel
                    hint={t("settings.channels.reasoningMappingTooltip")}
                    focusable
                    placement="top"
                    maxWidthClass="max-w-[260px]"
                  >
                    <HelpCircle
                      className="h-3.5 w-3.5 text-sp-text-muted hover:text-sp-text"
                      aria-label={t("settings.channels.reasoningMappingHelpAria")}
                    />
                  </HoverHintLabel>
                </span>
                <textarea
                  rows={2}
                  aria-label={t("settings.channels.reasoningMappingAria", { index: modelIndex + 1 })}
                  value={reasoningMappingValue}
                  onChange={(e) =>
                    update(
                      { reasoning_param_mapping: e.target.value || null },
                      "reasoning_param_mapping"
                    )
                  }
                  disabled={disabled}
                  className="w-full rounded-sp-sm border border-sp-line bg-sp-card p-2 font-mono text-[13px] text-sp-text-secondary outline-none focus:border-sp-text-secondary"
                />
              </div>
            </div>
          )}
        </div>
        </div>

        {/* 底部操作栏（吸底固定不随滚出弹窗：左侧「重置表单」，右侧「取消」+ 黑色「保存」） */}
        <div className="flex shrink-0 items-center justify-between gap-2 pt-3 border-t border-sp-divider bg-sp-surface">
          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={handleResetForm}
              className="text-[13px] text-sp-text underline hover:opacity-80 cursor-pointer"
            >
              {t("settings.channels.resetForm")}
            </button>
            {resetFeedback && (
              <span role="status" className="text-xs text-sp-success-text">
                {t("settings.channels.resetFormDone")}
              </span>
            )}
            {!isCreating && modelCount > 1 && (
              <button
                type="button"
                aria-label={t("settings.channels.deleteModel", { index: modelIndex + 1 })}
                disabled={disabled || modelCount <= 1}
                onClick={onDelete}
                className="flex h-7 items-center gap-1 rounded-sp-sm px-2 text-[13px] font-semibold text-sp-danger-text hover:bg-sp-danger-bg transition-colors cursor-pointer"
              >
                <Trash2 className="h-3.5 w-3.5" />
                <span>{t("settings.channels.deleteShort")}</span>
              </button>
            )}
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={handleSafeCancel}
              className="eo-settings-btn-secondary px-3"
            >
              {t("settings.channels.cancel")}
            </button>
            <button
              type="button"
              disabled={disabled}
              onClick={() => onDone(draft)}
              className="eo-settings-btn-primary px-4 disabled:opacity-50"
            >
              {t("settings.channels.done")}
            </button>
          </div>
        </div>
      </div>

      {confirmDiscardOpen && (
        <ModalOverlay
          data-testid="model-unsaved-confirm-dialog"
          className="z-[60]"
          closeOnBackdropClick={false}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-label={t("settings.channels.discardModelDialogAria")}
            onClick={(e) => e.stopPropagation()}
            className="w-full max-w-[380px] rounded-modal border border-sp-line bg-sp-surface p-5 shadow-modal space-y-4"
          >
            <div className="space-y-1">
              <div className="text-[14px] font-semibold text-sp-text">{t("settings.channels.discardModelTitle")}</div>
              <p className="text-[13px] text-sp-text-secondary leading-relaxed">
                {t("settings.channels.unsavedChanges")}
              </p>
            </div>
            <div className="flex items-center justify-end gap-2">
              <button
                type="button"
                data-testid="model-unsaved-confirm-stay"
                onClick={() => setConfirmDiscardOpen(false)}
                className="eo-settings-btn-secondary px-3 cursor-pointer"
              >
                {t("settings.channels.continueEditing")}
              </button>
              <button
                type="button"
                data-testid="model-unsaved-confirm-discard"
                onClick={() => {
                  setConfirmDiscardOpen(false);
                  onCancel();
                }}
                className="inline-flex h-[28px] items-center justify-center rounded-sp-sm bg-sp-danger-strong px-3 text-[13px] font-semibold text-white hover:opacity-90 cursor-pointer"
              >
                {t("settings.channels.discardChanges")}
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}
    </ModalOverlay>
  );
}

function ReasoningLevelChip({
  modelIndex,
  level,
  disabled,
  onChange,
  onRemove,
}: {
  modelIndex: number;
  level: AgentReasoningLevel;
  disabled: boolean;
  onChange: (change: Partial<AgentReasoningLevel>) => void;
  onRemove: () => void;
}) {
  useI18n();
  const hasDistinctValue =
    Boolean(level.value?.trim()) && level.value?.trim() !== level.level.trim();
  const [editingCustomValue, setEditingCustomValue] = useState(hasDistinctValue);

  return (
    <div className="flex items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-card p-1">
      <input
        aria-label={t("settings.channels.levelName", { index: modelIndex + 1 })}
        value={level.level}
        disabled={disabled}
        placeholder={t("settings.channels.levelNamePlaceholder")}
        onChange={(event) => {
          const nextLevel = event.target.value;
          const keepSynced =
            !editingCustomValue &&
            (level.value == null || level.value === "" || level.value === level.level);
          onChange(
            keepSynced
              ? { level: nextLevel, value: nextLevel.trim() === "off" ? null : nextLevel }
              : { level: nextLevel }
          );
        }}
        className="h-6 w-20 rounded-sp-sm bg-transparent px-2 text-[13px] text-sp-text outline-none focus:ring-1 focus:ring-sp-accent"
      />
      {(editingCustomValue || hasDistinctValue) && level.level.trim() !== "off" && (
        <input
          aria-label={t("settings.channels.levelValue", { index: modelIndex + 1 })}
          value={level.value ?? ""}
          disabled={disabled}
          onChange={(event) => onChange({ value: event.target.value })}
          placeholder={t("settings.channels.default")}
          className="h-6 w-20 rounded-sp-sm bg-sp-surface px-2 font-mono text-[13px] text-sp-text outline-none placeholder:text-sp-text-muted focus:ring-1 focus:ring-sp-accent disabled:opacity-50"
        />
      )}
      {level.level.trim() !== "off" && !editingCustomValue && !hasDistinctValue && (
        <button
          type="button"
          title={t("settings.channels.customParamValueTitle")}
          aria-label={t("settings.channels.customParamValueAria", {
            level: level.level || t("settings.channels.newLevelLabel"),
          })}
          disabled={disabled}
          onClick={() => setEditingCustomValue(true)}
          className="flex h-6 w-5 items-center justify-center rounded-sp-sm text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer disabled:opacity-50"
        >
          <Pencil className="h-2.5 w-2.5" />
        </button>
      )}
      <button
        type="button"
        aria-label={t("settings.channels.deleteReasoningLevel", {
          index: modelIndex + 1,
          level: level.level || t("settings.channels.unnamedLevel"),
        })}
        title={t("settings.channels.deleteLevel")}
        disabled={disabled}
        onClick={onRemove}
        className="flex h-6 w-6 items-center justify-center rounded-sp-sm text-sp-text-muted hover:bg-sp-hover hover:text-sp-danger-text cursor-pointer transition-colors duration-fast ease-sp disabled:opacity-50 outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

export interface ChannelModelsFetchDialogProps {
  open: boolean;
  protocol?: string;
  models: AgentChannelModel[];
  ids: string[];
  loading: boolean;
  error: string | null;
  onCancel: () => void;
  onMerge: (models: AgentChannelModel[]) => void;
  triggerRef: React.RefObject<HTMLElement | null>;
}

export function ChannelModelsFetchDialog({
  open,
  protocol,
  models,
  ids,
  loading,
  error,
  onCancel,
  onMerge,
  triggerRef,
}: ChannelModelsFetchDialogProps) {
  useI18n();
  const titleId = useId();
  const [search, setSearch] = useState("");
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const searchRef = useRef<HTMLInputElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const [smartFilter, setSmartFilter] = useState(true);
  const visibleIds = useMemo(
    () =>
      filterDiscoveredModels(ids, {
        hideNonChat: smartFilter,
        collapseSnapshots: smartFilter,
      }),
    [ids, smartFilter]
  );

  useEffect(() => {
    if (open) {
      setSearch("");
      setSmartFilter(true);
      setChecked(new Set(filterDiscoveredModels(ids)));
    }
  }, [open, ids]);

  useEffect(() => {
    if (!open) return;
    const timer = window.setTimeout(() => searchRef.current?.focus(), 0);
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onCancel();
      }
      if (event.key === "Tab" && dialogRef.current) {
        const elements = Array.from(
          dialogRef.current.querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled)")
        );
        const first = elements[0],
          last = elements[elements.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first?.focus();
        }
      }
    };
    document.addEventListener("keydown", keydown, true);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("keydown", keydown, true);
      triggerRef.current?.focus();
    };
  }, [open, onCancel, triggerRef]);

  const filtered = useMemo(
    () => visibleIds.filter((id) => id.toLowerCase().includes(search.trim().toLowerCase())),
    [visibleIds, search]
  );

  if (!open) return null;

  const toggleAll = () =>
    setChecked((previous) => {
      const next = new Set(previous);
      const allChecked = filtered.length > 0 && filtered.every((id) => next.has(id));
      for (const id of filtered) {
        if (allChecked) next.delete(id);
        else next.add(id);
      }
      return next;
    });

  return (
    <ModalOverlay closeOnBackdropClick={false}>
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="flex max-h-[560px] w-full max-w-[480px] flex-col rounded-modal border border-sp-line bg-sp-surface dark:bg-sp-card p-5 shadow-modal space-y-3"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 id={titleId} className="text-sm font-semibold text-sp-text">
          {t("settings.channels.fetchAvailable")}
        </h2>
        <input
          ref={searchRef}
          aria-label={t("settings.channels.searchModels")}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t("settings.channels.searchModels")}
          className="eo-settings-input h-9"
        />
        <label className="flex cursor-pointer items-center gap-2 text-[13px] text-sp-text-secondary">
          <input
            type="checkbox"
            aria-label={t("settings.channels.smartFilterLabel")}
            checked={smartFilter}
            onChange={(e) => setSmartFilter(e.target.checked)}
            className="h-4 w-4 accent-sp-accent-fill"
          />
          <span>{t("settings.channels.smartFilterLabel")}</span>
        </label>
        <div className="flex items-center justify-between">
          <span className="text-[13px] text-sp-text-muted">
            {t("settings.channels.modelCount", { count: visibleIds.length })}
          </span>
          <button
            type="button"
            disabled={loading || filtered.length === 0}
            onClick={toggleAll}
            className="h-7 rounded-sp-sm px-2 text-[13px] text-sp-text hover:bg-sp-hover cursor-pointer transition-colors duration-fast ease-sp disabled:opacity-50 outline-none"
          >
            {filtered.every((id) => checked.has(id)) && filtered.length
              ? t("settings.channels.deselectAll")
              : t("settings.channels.selectAll")}
          </button>
        </div>
        <div className="min-h-24 flex-1 overflow-y-auto rounded-card border border-sp-line bg-sp-card p-1 space-y-1">
          {loading ? (
            <FetchModelsDelayedSkeleton />
          ) : error ? (
            <p role="alert" className="p-2 text-[13px] text-sp-danger-text">
              {error}
            </p>
          ) : filtered.length === 0 ? (
            <div className="flex h-24 flex-col items-center justify-center gap-2 text-[13px] text-sp-text-muted">
              <Search className="h-5 w-5 stroke-[1.5] text-sp-text-muted/60" aria-hidden="true" />
              <span>{t("settings.channels.noMatches")}</span>
            </div>
          ) : (
            filtered.map((id) => (
              <label
                key={id}
                className={cn(
                  "flex h-8 cursor-pointer items-center gap-2 rounded-sp-sm px-3 text-[13px] text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp",
                  checked.has(id) && "bg-sp-selected font-normal"
                )}
              >
                <input
                  type="checkbox"
                  aria-label={t("settings.channels.selectModel", { id })}
                  checked={checked.has(id)}
                  onChange={() =>
                    setChecked((previous) => {
                      const next = new Set(previous);
                      if (next.has(id)) next.delete(id);
                      else next.add(id);
                      return next;
                    })
                  }
                  className="h-4 w-4 accent-sp-accent-fill"
                />
                <span className="min-w-0 flex-1 truncate font-mono text-[13px]">{id}</span>
                {checked.has(id) && <Check className="h-3.5 w-3.5 shrink-0 text-sp-text" />}
              </label>
            ))
          )}
        </div>
        <div className="flex justify-end gap-2 pt-1">
          <button
            type="button"
            onClick={onCancel}
            className="eo-settings-btn-secondary px-3"
          >
            {t("settings.channels.cancel")}
          </button>
          <button
            type="button"
            disabled={loading || Boolean(error)}
            onClick={() => {
              onMerge(mergeChannelModels(models, [...checked], protocol));
              onCancel();
            }}
            className="eo-settings-btn-primary px-4 disabled:opacity-50"
          >
            {t("settings.channels.updateList")}
          </button>
        </div>
      </div>
    </ModalOverlay>
  );
}
