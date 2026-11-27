/**
 * AgentModelPicker.tsx
 * EOStage-style split floating Channel + Model picker with search input
 * and bottom horizontal stepped Reasoning Intensity Slider (`思考强度 <label> [Ctrl+T]`).
 * Styled natively for both EOStage Light and Dark themes.
 */

import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Box, Check, ChevronRight, RefreshCw, Search, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import { EmptyState } from "@/components/ui/EmptyState";
import { isTauriDesktopHost } from "@/lib/bridge";
import {
  fetchChannelAuthMetaMap,
  resolveChannelAuthKind,
  type ChannelAuthMeta,
} from "@/shell/api/agentChannelsLogic";
import type {
  AgentChannelModel,
  AgentChannelView,
  AgentModelSelection,
} from "@/lib/generated/v3-contracts";
import {
  buildModelSelection,
  channelDisplayName,
  filterEnabledChannelGroups,
  isOfficialChannelId,
  modelSelectionLabel,
  parseStoredModelSelection,
  resolveActiveModelMeta,
  resolveModelSelection,
} from "@/shell/api/channelModels";
import {
  eostageAccountSyncModels,
  notifyEOStageAccountChanged,
} from "@/shell/api/eostageAccount";
import { notifyChannelsChanged } from "@/shell/api/agentChannelsEvents";
import {
  CHANNEL_PREFERENCES_CHANGED_EVENT,
  hydrateDisabledPreferencesFromBackend,
  loadDisabledChannelModels,
  loadDisabledProviders,
} from "@/lib/channelPreferences";
import { ReasoningIntensitySlider } from "./agent/ReasoningIntensitySlider";

const STORAGE_KEY = "eostage.agent.modelSelection";
const GLOBAL_SESSION = "__global__";
const MODEL_SELECTION_SYNC_EVENT = "eostage:agent-model-selection-sync";
const sessionSelections: Record<string, AgentModelSelection | null> = {};

/** Synchronize both per-session and global default model selection from external switches (e.g. ErrorCard fallback). */
export function syncAgentModelSelection(
  sessionId: string | null,
  next: AgentModelSelection | null
): void {
  const key = sessionId || GLOBAL_SESSION;
  sessionSelections[key] = next;
  sessionSelections[GLOBAL_SESSION] = next;
  try {
    if (typeof window !== "undefined") {
      if (next) window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
      else window.localStorage.removeItem(STORAGE_KEY);
      window.dispatchEvent(
        new CustomEvent(MODEL_SELECTION_SYNC_EVENT, {
          detail: { sessionKey: key, selection: next },
        })
      );
    }
  } catch {
    /* Storage and event dispatch are optional. */
  }
}

export interface AgentModelPickerProps {
  channels: AgentChannelView[];
  sessionId: string | null;
  ready: boolean;
  compact?: boolean;
  /** Incrementing this nonce closes the popover cleanly (e.g. when user starts typing or submits). */
  closeNonce?: number;
  onGoSettings?: () => void;
  onChange?: (selection: AgentModelSelection | null) => void;
}

export function AgentModelPicker({
  channels,
  sessionId,
  ready,
  compact = false,
  closeNonce = 0,
  onGoSettings,
  onChange,
}: AgentModelPickerProps) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [channelsPaneOpen, setChannelsPaneOpen] = useState(false);
  const [authMetaMap, setAuthMetaMap] = useState<Record<string, ChannelAuthMeta>>({});

  useEffect(() => {
    if (!isTauriDesktopHost()) return;
    let cancelled = false;
    void fetchChannelAuthMetaMap().then((map) => {
      if (!cancelled) {
        setAuthMetaMap(map);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [channels]);

  const channelAuthBadgeText = useCallback(
    (channel: AgentChannelView): string => {
      const kind = resolveChannelAuthKind(channel, authMetaMap);
      if (kind === "official") return t("agent.modelPicker.authKindOfficial");
      if (kind === "subscription") return t("settings.channels.authKindSubscription");
      return t("settings.channels.authKindApi");
    },
    [authMetaMap, t]
  );
  const [prefRevision, setPrefRevision] = useState(0);
  const [memory, setMemory] = useState<Record<string, AgentModelSelection | null>>({});
  const [activeChannelId, setActiveChannelId] = useState<string | null>(null);
  const [modelSearch, setModelSearch] = useState("");
  const [syncingOfficial, setSyncingOfficial] = useState(false);
  const [officialSyncStatus, setOfficialSyncStatus] = useState<{
    kind: "ok" | "err";
    text: string;
  } | null>(null);
  const [popoverLayout, setPopoverLayout] = useState<{
    fixed: boolean;
    side: "up" | "down";
    left: number;
    top: number | null;
    bottom: number | null;
    maxHeight: number;
    clampedShift: number;
  }>({
    fixed: false,
    side: "up",
    left: 0,
    top: null,
    bottom: null,
    maxHeight: 420,
    clampedShift: 0,
  });
  const memoryRef = useRef(memory);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const prevCloseNonceRef = useRef(closeNonce);
  const sessionKey = sessionId || GLOBAL_SESSION;
  const prevSessionKeyRef = useRef(sessionKey);
  const mountedRef = useRef(true);
  const syncSeqRef = useRef(0);
  const syncStatusTimerRef = useRef<number | null>(null);

  const clearSyncStatusTimer = useCallback(() => {
    if (syncStatusTimerRef.current !== null && typeof window !== "undefined") {
      window.clearTimeout(syncStatusTimerRef.current);
      syncStatusTimerRef.current = null;
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      syncSeqRef.current += 1;
      clearSyncStatusTimer();
    };
  }, [clearSyncStatusTimer]);

  const handleSyncOfficialModels = useCallback(async () => {
    if (syncingOfficial) return;
    clearSyncStatusTimer();
    const seq = ++syncSeqRef.current;
    setSyncingOfficial(true);
    setOfficialSyncStatus(null);
    try {
      await eostageAccountSyncModels();
      notifyChannelsChanged();
      if (!mountedRef.current || seq !== syncSeqRef.current) return;
      setOfficialSyncStatus({ kind: "ok", text: t("agent.modelPicker.syncOfficialOk") });
      if (typeof window !== "undefined") {
        syncStatusTimerRef.current = window.setTimeout(() => {
          syncStatusTimerRef.current = null;
          if (!mountedRef.current || seq !== syncSeqRef.current) return;
          setOfficialSyncStatus(null);
        }, 3000);
      }
    } catch (err) {
      const msg = err instanceof Error && err.message ? err.message : t("agent.modelPicker.syncOfficialFailed");
      if (/失效|登录|expired|sign in|login|失効|ログイン/i.test(msg)) {
        notifyEOStageAccountChanged();
        notifyChannelsChanged();
      }
      if (!mountedRef.current || seq !== syncSeqRef.current) return;
      setOfficialSyncStatus({ kind: "err", text: msg });
    } finally {
      if (mountedRef.current && seq === syncSeqRef.current) {
        setSyncingOfficial(false);
      }
    }
  }, [clearSyncStatusTimer, syncingOfficial, t]);

  const estimatedPopoverWidth = channelsPaneOpen
    ? compact
      ? 426
      : 496
    : compact
      ? 260
      : 300;

  const updatePopoverPosition = useCallback(() => {
    const triggerEl = triggerRef.current;
    if (!triggerEl || typeof window === "undefined") {
      setPopoverLayout({
        fixed: false,
        side: "up",
        left: 0,
        top: null,
        bottom: null,
        maxHeight: 420,
        clampedShift: 0,
      });
      return;
    }
    const rect = triggerEl.getBoundingClientRect();
    if (rect.width === 0 && rect.right === 0 && rect.top === 0 && rect.bottom === 0) {
      setPopoverLayout({
        fixed: false,
        side: "up",
        left: 0,
        top: null,
        bottom: null,
        maxHeight: 420,
        clampedShift: 0,
      });
      return;
    }
    const vw = window.innerWidth || 1280;
    const vh = window.innerHeight || 800;
    const safeMargin = 12;
    const topInset = 52;
    const chatPane = triggerEl.closest("section.chat");
    const chatPaneLeft = chatPane ? Math.round(chatPane.getBoundingClientRect().left) : 0;
    const safeLeft = Math.max(safeMargin, chatPaneLeft > 0 ? chatPaneLeft + 8 : safeMargin);
    const popoverWidth = Math.min(estimatedPopoverWidth, Math.max(240, vw - safeMargin * 2));
    const naturalLeft = rect.right - popoverWidth;
    const maxLeft = Math.max(safeLeft, vw - safeMargin - popoverWidth);
    const clampedLeft = Math.round(Math.min(maxLeft, Math.max(safeLeft, naturalLeft)));
    const clampedShift = Math.max(0, Math.round(clampedLeft - naturalLeft));

    const spaceAbove = rect.top - topInset - 8;
    const spaceBelow = vh - rect.bottom - safeMargin - 8;
    const estimatedHeight = 408;
    const side: "up" | "down" =
      spaceAbove >= estimatedHeight || spaceAbove >= spaceBelow ? "up" : "down";
    const maxHeight = Math.max(
      220,
      Math.floor(side === "up" ? spaceAbove : spaceBelow)
    );

    setPopoverLayout({
      fixed: true,
      side,
      left: clampedLeft,
      top: side === "down" ? Math.round(rect.bottom + 8) : null,
      bottom: side === "up" ? Math.max(safeMargin, Math.round(vh - rect.top + 8)) : null,
      maxHeight,
      clampedShift,
    });
  }, [estimatedPopoverWidth]);

  useEffect(() => {
    if (closeNonce !== prevCloseNonceRef.current) {
      prevCloseNonceRef.current = closeNonce;
      setOpen(false);
    }
  }, [closeNonce]);

  useEffect(() => {
    const bump = () => setPrefRevision((prev) => prev + 1);
    const onExternalSync = (ev: Event) => {
      const detail = (ev as CustomEvent<{ sessionKey?: string; selection?: AgentModelSelection | null }>).detail;
      if (!detail) return;
      const syncKey = detail.sessionKey || GLOBAL_SESSION;
      const sel = detail.selection ?? null;
      sessionSelections[syncKey] = sel;
      sessionSelections[GLOBAL_SESSION] = sel;
      const next = { ...memoryRef.current, [syncKey]: sel, [GLOBAL_SESSION]: sel };
      memoryRef.current = next;
      setMemory(next);
    };
    window.addEventListener(CHANNEL_PREFERENCES_CHANGED_EVENT, bump);
    window.addEventListener(MODEL_SELECTION_SYNC_EVENT, onExternalSync);
    void hydrateDisabledPreferencesFromBackend();
    return () => {
      window.removeEventListener(CHANNEL_PREFERENCES_CHANGED_EVENT, bump);
      window.removeEventListener(MODEL_SELECTION_SYNC_EVENT, onExternalSync);
    };
  }, []);

  useEffect(() => {
    let stored: AgentModelSelection | null = null;
    if (Object.prototype.hasOwnProperty.call(sessionSelections, sessionKey)) {
      stored = sessionSelections[sessionKey];
    } else if (
      sessionKey === GLOBAL_SESSION &&
      Object.prototype.hasOwnProperty.call(memoryRef.current, GLOBAL_SESSION)
    ) {
      stored = memoryRef.current[GLOBAL_SESSION];
    } else {
      try {
        stored = parseStoredModelSelection(window.localStorage.getItem(STORAGE_KEY));
      } catch {
        /* Storage is optional. */
      }
    }
    const existingHasKey = Object.prototype.hasOwnProperty.call(memoryRef.current, sessionKey);
    if (existingHasKey && sessionKey !== GLOBAL_SESSION) return;
    if (
      existingHasKey &&
      JSON.stringify(memoryRef.current[sessionKey] ?? null) === JSON.stringify(stored ?? null)
    ) {
      return;
    }
    const next = { ...memoryRef.current, [sessionKey]: stored };
    memoryRef.current = next;
    setMemory(next);
  }, [sessionKey]);

  const requested = Object.prototype.hasOwnProperty.call(memory, sessionKey)
    ? memory[sessionKey]
    : null;

  const { groups, enabledChannels } = useMemo(() => {
    const disabledProviders = loadDisabledProviders();
    const disabledModelsByChannel: Record<string, Record<string, boolean>> = {};
    for (const ch of channels) {
      disabledModelsByChannel[ch.id] = loadDisabledChannelModels(ch.id);
    }
    const filteredGroups = filterEnabledChannelGroups(
      channels,
      disabledProviders,
      disabledModelsByChannel
    );
    const activeChannels = filteredGroups.map((group) => ({
      ...group.channel,
      model: group.models.some((m) => m.id === group.channel.model)
        ? group.channel.model
        : group.models[0].id,
      models: group.models,
    }));
    return { groups: filteredGroups, enabledChannels: activeChannels };
  }, [channels, prefRevision, open]);

  const selection = resolveModelSelection(enabledChannels, requested);
  const effectiveDefaultGroup = useMemo(
    () => groups.find((g) => g.channel.is_default) ?? groups[0] ?? null,
    [groups]
  );
  const effectiveDefaultModelId = useMemo(() => {
    if (!effectiveDefaultGroup) return null;
    const chModel = effectiveDefaultGroup.channel.model?.trim();
    if (chModel && effectiveDefaultGroup.models.some((m) => m.id === chModel)) {
      return chModel;
    }
    return effectiveDefaultGroup.models[0]?.id ?? null;
  }, [effectiveDefaultGroup]);
  const fullLabel = ready ? modelSelectionLabel(enabledChannels, selection) : t("shared.channelModels.selectModel");
  const displayLabel = fullLabel || t("shared.channelModels.selectModel");
  const activeModelMeta = useMemo(
    () => (ready ? resolveActiveModelMeta(enabledChannels, selection) : {}),
    [enabledChannels, ready, selection],
  );

  const activeGroup = useMemo(() => {
    if (groups.length === 0) return null;
    if (activeChannelId) {
      const found = groups.find((g) => g.channel.id === activeChannelId);
      if (found) return found;
    }
    if (selection) {
      const fromSelection = groups.find((g) => g.channel.id === selection.channel_id);
      if (fromSelection) return fromSelection;
    }
    return groups.find((g) => g.channel.is_default) ?? groups[0];
  }, [activeChannelId, groups, selection]);

  const filteredModels = useMemo(() => {
    if (!activeGroup) return [];
    const q = modelSearch.trim().toLowerCase();
    if (!q) return activeGroup.models;
    return activeGroup.models.filter((m) => {
      const name = (m.name || "").toLowerCase();
      return m.id.toLowerCase().includes(q) || name.includes(q);
    });
  }, [activeGroup, modelSearch]);

  // Resolve the model controlled by the bottom Reasoning Intensity Slider
  const sliderTargetModel: AgentChannelModel | null = useMemo(() => {
    if (!activeGroup || activeGroup.models.length === 0) return null;
    if (selection && selection.channel_id === activeGroup.channel.id) {
      const matched = activeGroup.models.find((m) => m.id === selection.model_id);
      if (matched) return matched;
    }
    return (
      activeGroup.models.find((m) => m.id === activeGroup.channel.model) ??
      activeGroup.models[0]
    );
  }, [activeGroup, selection]);

  const choose = useCallback(
    (next: AgentModelSelection | null, shouldClose: boolean = true) => {
      sessionSelections[sessionKey] = next;
      sessionSelections[GLOBAL_SESSION] = next;
      const updated = { ...memoryRef.current, [sessionKey]: next, [GLOBAL_SESSION]: next };
      memoryRef.current = updated;
      setMemory(updated);
      try {
        if (next) window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
        else window.localStorage.removeItem(STORAGE_KEY);
      } catch {
        /* Storage is optional. */
      }
      if (shouldClose) {
        setOpen(false);
        triggerRef.current?.focus();
      }
      onChange?.(next);
    },
    [onChange, sessionKey]
  );

  // Keep parent dispatch routing synchronized with the displayed pill whenever switching sessions or clicking "New Chat".
  useEffect(() => {
    if (prevSessionKeyRef.current !== sessionKey) {
      prevSessionKeyRef.current = sessionKey;
      onChange?.(selection);
    }
  }, [onChange, selection, sessionKey]);

  const handleSelectReasoningLevel = useCallback(
    (channelId: string, model: AgentChannelModel, level: string) => {
      const next = buildModelSelection(enabledChannels, channelId, model.id, level);
      if (next) {
        choose(next, false);
      }
    },
    [choose, enabledChannels]
  );

  const handleCycleReasoningLevel = useCallback(() => {
    const targetGroup = activeGroup;
    const targetModel = sliderTargetModel;
    if (!targetGroup || !targetModel) return;
    const levels = targetModel.reasoning_levels ?? [];
    if (levels.length === 0) return;

    const currentLevel =
      selection &&
      selection.channel_id === targetGroup.channel.id &&
      selection.model_id === targetModel.id
        ? selection.reasoning_level
        : null;
    const currentIdx = levels.findIndex((item) => item.level === currentLevel);
    const nextIdx = currentIdx >= 0 ? (currentIdx + 1) % levels.length : 0;
    const nextLevel = levels[nextIdx]?.level;
    if (nextLevel) {
      handleSelectReasoningLevel(targetGroup.channel.id, targetModel, nextLevel);
    }
  }, [activeGroup, handleSelectReasoningLevel, selection, sliderTargetModel]);

  // Global Ctrl+T shortcut to cycle reasoning intensity on the active model
  useEffect(() => {
    if (!ready || enabledChannels.length === 0) return;
    const onGlobalKeyDown = (event: KeyboardEvent) => {
      if (
        event.ctrlKey &&
        !event.metaKey &&
        !event.altKey &&
        !event.shiftKey &&
        event.key.toLowerCase() === "t"
      ) {
        const targetGroup = activeGroup ?? groups[0];
        const targetModel = sliderTargetModel;
        const levels = targetModel?.reasoning_levels ?? [];
        if (!targetGroup || !targetModel || levels.length === 0) return;
        event.preventDefault();
        handleCycleReasoningLevel();
      }
    };
    document.addEventListener("keydown", onGlobalKeyDown);
    return () => document.removeEventListener("keydown", onGlobalKeyDown);
  }, [activeGroup, enabledChannels.length, groups, handleCycleReasoningLevel, ready, sliderTargetModel]);

  const close = useCallback(() => {
    setOpen(false);
    setChannelsPaneOpen(false);
  }, []);

  useEffect(() => {
    if (!open) {
      setChannelsPaneOpen(false);
    }
  }, [open]);

  useLayoutEffect(() => {
    if (!open) return;
    updatePopoverPosition();
  }, [open, channelsPaneOpen, updatePopoverPosition]);

  useEffect(() => {
    if (!open) return;
    updatePopoverPosition();
    const outside = (event: MouseEvent) => {
      const target = event.target as Node;
      if (
        containerRef.current &&
        !containerRef.current.contains(target) &&
        !(menuRef.current && menuRef.current.contains(target))
      ) {
        setOpen(false);
      }
    };
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setOpen(false);
        triggerRef.current?.focus();
      }
      if (
        ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key) &&
        menuRef.current &&
        document.activeElement !== searchInputRef.current
      ) {
        event.preventDefault();
        const items = Array.from(menuRef.current.querySelectorAll<HTMLElement>("[role^='menuitem']"));
        if (items.length === 0) return;
        const index = items.indexOf(document.activeElement as HTMLElement);
        const next =
          event.key === "Home"
            ? 0
            : event.key === "End"
              ? items.length - 1
              : (index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
        items[next]?.focus();
      }
    };
    const onResize = () => {
      updatePopoverPosition();
    };
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", keydown);
    window.addEventListener("resize", onResize);
    return () => {
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("keydown", keydown);
      window.removeEventListener("resize", onResize);
    };
  }, [open, updatePopoverPosition]);

  useEffect(() => {
    if (!open) {
      syncSeqRef.current += 1;
      clearSyncStatusTimer();
      setOfficialSyncStatus(null);
      setSyncingOfficial(false);
    }
  }, [clearSyncStatusTimer, open]);

  const handleToggleOpen = () => {
    if (!ready) {
      onGoSettings?.();
      return;
    }
    setOpen((previous) => {
      const nextOpen = !previous;
      if (nextOpen) {
        setModelSearch("");
        setChannelsPaneOpen(false);
        const initialCh =
          selection?.channel_id ??
          groups.find((g) => g.channel.is_default)?.channel.id ??
          groups[0]?.channel.id ??
          null;
        setActiveChannelId(initialCh);
      } else {
        setChannelsPaneOpen(false);
      }
      return nextOpen;
    });
  };

  const activeSliderLevel =
    selection &&
    activeGroup &&
    sliderTargetModel &&
    selection.channel_id === activeGroup.channel.id &&
    selection.model_id === sliderTargetModel.id
      ? selection.reasoning_level
      : null;

  const portalTarget =
    popoverLayout.fixed && typeof document !== "undefined"
      ? (document.querySelector(".sp-modern") ?? document.body)
      : null;

  const popoverNode = open ? (
    <div
      ref={menuRef}
      role="menu"
      aria-label={t("agent.modelPicker.modelMenuAria")}
      data-testid="agent-model-picker-popover"
      data-clamped-shift={popoverLayout.clampedShift}
      data-placement={popoverLayout.side}
      style={
        popoverLayout.fixed
          ? {
              position: "fixed",
              left: `${popoverLayout.left}px`,
              ...(popoverLayout.top !== null ? { top: `${popoverLayout.top}px` } : {}),
              ...(popoverLayout.bottom !== null ? { bottom: `${popoverLayout.bottom}px` } : {}),
              maxHeight: `${popoverLayout.maxHeight}px`,
              zIndex: 90,
            }
          : {
              right: "0px",
            }
      }
      className={cn(
        "sp-popover-blur-in flex max-w-[calc(100vw-24px)] select-none",
        popoverLayout.fixed
          ? popoverLayout.side === "up"
            ? "items-end"
            : "items-start"
          : "absolute bottom-[calc(100%+8px)] z-50 items-end",
        compact ? "gap-1.5" : "gap-2"
      )}
    >
      {/* Left Floating Pane: Enabled Channels + 默认模型 + 管理模型 (opened when Box 1 channel header is clicked) */}
      {channelsPaneOpen && (
      <div
        data-testid="model-picker-channels-pane"
        style={{ maxHeight: `${popoverLayout.maxHeight}px` }}
        className={cn(
          "flex shrink-0 flex-col overflow-hidden rounded-[12px] border border-sp-line bg-sp-surface dark:bg-sp-card shadow-overlay",
          compact ? "w-[168px]" : "w-[196px]"
        )}
      >
        <div className="min-h-0 flex-1 max-h-[304px] overflow-y-auto p-1.5 space-y-0.5">
              {groups.map((group) => {
                const isViewing = activeGroup?.channel.id === group.channel.id;
                const activeChId = selection?.channel_id ?? effectiveDefaultGroup?.channel.id ?? null;
                const isSelectedChannel = activeChId === group.channel.id;
                const pill = channelAuthBadgeText(group.channel);
                const label = channelDisplayName(group.channel);
                return (
                  <button
                    key={group.channel.id}
                    type="button"
                    data-testid={`model-picker-channel-${group.channel.id}`}
                    aria-current={isViewing ? "true" : undefined}
                    onClick={() => {
                      syncSeqRef.current += 1;
                      setSyncingOfficial(false);
                      clearSyncStatusTimer();
                      setOfficialSyncStatus(null);
                      setActiveChannelId(group.channel.id);
                      setModelSearch("");
                    }}
                    className={cn(
                      "flex h-8 w-full cursor-pointer items-center gap-1.5 rounded-[7px] px-2 text-left text-xs text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp outline-none focus-visible:ring-1 focus-visible:ring-sp-accent",
                      isViewing && "bg-sp-selected font-medium"
                    )}
                  >
                    <Box className="h-3.5 w-3.5 shrink-0 text-sp-text-muted stroke-[1.6]" />
                    <span className="min-w-0 truncate" title={label}>
                      {label}
                    </span>
                    <span className="shrink-0 rounded-[4px] border border-sp-line/70 bg-sp-surface-muted px-1.5 py-0.5 text-xs font-medium leading-none text-sp-text-muted">
                      {pill}
                    </span>
                    {isSelectedChannel && (
                      <Check className="ml-auto h-3.5 w-3.5 shrink-0 text-sp-text-secondary" />
                    )}
                  </button>
                );
              })}
            </div>

            <div className="border-t border-sp-divider p-1.5">
              <button
                type="button"
                data-testid="model-picker-manage-btn"
                onClick={() => {
                  close();
                  onGoSettings?.();
                }}
                className="flex h-8 w-full cursor-pointer items-center rounded-[7px] px-2.5 text-left text-xs font-medium text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp"
              >
                {t("agent.modelPicker.manageModels")}
              </button>
            </div>
          </div>
      )}

          {/* Right Floating Pane: Active Channel Header + Search + Model List + Bottom Reasoning Slider */}
          {activeGroup && (
            <div
              data-testid="model-picker-models-pane"
              style={{ maxHeight: `${popoverLayout.maxHeight}px` }}
              className={cn(
                "flex min-w-0 flex-col overflow-hidden rounded-[12px] border border-sp-line bg-sp-surface dark:bg-sp-card shadow-overlay",
                compact ? "w-[252px]" : "w-[292px]"
              )}
            >
              {/* Active Channel Header (`>` to open settings) + Optional Official Model Sync */}
              <div className="p-1.5 pb-1">
                <div className="flex items-center gap-1">
                  <button
                    type="button"
                    data-testid="model-picker-active-channel-header"
                    aria-expanded={channelsPaneOpen}
                    onClick={() => setChannelsPaneOpen((prev) => !prev)}
                    title={channelsPaneOpen ? t("agent.modelPicker.collapseChannels") : t("agent.modelPicker.switchChannel")}
                    className="flex h-8 min-w-0 flex-1 cursor-pointer items-center gap-1.5 rounded-[8px] border border-sp-line/60 bg-sp-surface-muted/80 px-2.5 text-left text-xs font-medium text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp"
                  >
                    <Box className="h-3.5 w-3.5 shrink-0 text-sp-text-secondary stroke-[1.6]" />
                    <span className="min-w-0 truncate">{channelDisplayName(activeGroup.channel)}</span>
                    <span className="shrink-0 rounded-[4px] border border-sp-line/70 bg-sp-surface px-1.5 py-0.5 text-xs font-medium leading-none text-sp-text-muted">
                      {channelAuthBadgeText(activeGroup.channel)}
                    </span>
                    <ChevronRight
                      className={cn(
                        "ml-auto h-3.5 w-3.5 shrink-0 text-sp-text-muted transition-transform duration-150",
                        channelsPaneOpen && "rotate-180"
                      )}
                    />
                  </button>
                  {isOfficialChannelId(activeGroup.channel.id) && (
                    <button
                      type="button"
                      data-testid="model-picker-sync-official-btn"
                      aria-label={t("agent.modelPicker.syncOfficialTitle")}
                      title={t("agent.modelPicker.syncOfficialTitle")}
                      disabled={syncingOfficial}
                      onClick={() => void handleSyncOfficialModels()}
                      className="inline-flex h-8 shrink-0 cursor-pointer items-center gap-1 whitespace-nowrap rounded-[8px] border border-sp-line/60 bg-sp-surface-muted/80 px-2 text-xs font-medium text-sp-text-secondary transition-colors duration-fast ease-sp hover:bg-sp-hover hover:text-sp-text disabled:cursor-not-allowed disabled:opacity-60 outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
                    >
                      <RefreshCw
                        className={cn(
                          "h-3.5 w-3.5 shrink-0 stroke-[1.6]",
                          syncingOfficial && "animate-spin"
                        )}
                      />
                      <span>{syncingOfficial ? t("agent.modelPicker.syncing") : t("agent.modelPicker.sync")}</span>
                    </button>
                  )}
                </div>
                {officialSyncStatus && (
                  <p
                    data-testid="model-picker-sync-status"
                    role={officialSyncStatus.kind === "err" ? "alert" : "status"}
                    className={cn(
                      "px-1 pt-1 text-xs leading-tight",
                      officialSyncStatus.kind === "err"
                        ? "text-sp-text"
                        : "text-sp-text-secondary"
                    )}
                  >
                    {officialSyncStatus.text}
                  </p>
                )}
              </div>

              {/* Search Bar */}
              <div className="flex items-center gap-2 border-b border-sp-divider px-3 py-2">
                <span className="shrink-0 text-xs text-sp-text-muted">{t("agent.modelPicker.modelLabel")}</span>
                <Search className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                <input
                  ref={searchInputRef}
                  type="text"
                  value={modelSearch}
                  onChange={(e) => setModelSearch(e.target.value)}
                  placeholder={t("agent.modelPicker.searchPlaceholder")}
                  aria-label={t("agent.modelPicker.searchAria")}
                  data-testid="model-picker-search-input"
                  className="min-w-0 flex-1 bg-transparent text-xs text-sp-text placeholder:text-sp-text-muted outline-none"
                />
                {modelSearch && (
                  <button
                    type="button"
                    aria-label={t("agent.modelPicker.clearSearchAria")}
                    onClick={() => setModelSearch("")}
                    className="inline-flex h-4 w-4 shrink-0 items-center justify-center rounded text-sp-text-muted hover:text-sp-text cursor-pointer"
                  >
                    <X className="h-3 w-3" />
                  </button>
                )}
              </div>

              {/* Scrollable Model List */}
              <div
                data-testid="model-picker-models-list"
                className="min-h-0 flex-1 max-h-[224px] overflow-y-auto p-1.5 space-y-0.5"
              >
                {filteredModels.length === 0 ? (
                  <EmptyState
                    kind="no-results"
                    size="menu"
                    title={
                      modelSearch.trim()
                        ? t("agent.modelPicker.noMatchQuery", { query: modelSearch.trim() })
                        : t("agent.modelPicker.noMatch")
                    }
                    className="py-6"
                  />
                ) : (
                  filteredModels.map((model) => {
                    const modelActive = selection
                      ? selection.channel_id === activeGroup.channel.id &&
                        selection.model_id === model.id
                      : effectiveDefaultGroup?.channel.id === activeGroup.channel.id &&
                        effectiveDefaultModelId === model.id;
                    const preservedLevel =
                      modelActive &&
                      selection?.reasoning_level &&
                      model.reasoning_levels?.some((l) => l.level === selection.reasoning_level)
                        ? selection.reasoning_level
                        : null;
                    const modelSelection = buildModelSelection(
                      enabledChannels,
                      activeGroup.channel.id,
                      model.id,
                      preservedLevel
                    );
                    const modelDisplayName = model.name?.trim() || model.id;
                    return (
                      <div
                        key={model.id}
                        role="menuitemradio"
                        aria-checked={modelActive}
                        data-testid={`model-picker-model-${model.id}`}
                        tabIndex={0}
                        onClick={() => choose(modelSelection, true)}
                        onKeyDown={(event) => {
                          if (event.key === "Enter" || event.key === " ") {
                            event.preventDefault();
                            choose(modelSelection, true);
                          }
                        }}
                        className={cn(
                          "flex h-8 cursor-pointer items-center gap-2 rounded-[7px] px-2.5 text-xs text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp outline-none focus-visible:ring-1 focus-visible:ring-sp-accent",
                          modelActive && "bg-sp-selected font-medium"
                        )}
                      >
                        <span className="min-w-0 flex-1 truncate" title={model.id}>
                          {modelDisplayName}
                        </span>
                        {isOfficialChannelId(activeGroup.channel.id) && model.is_free === true && (
                          <span
                            data-testid={`model-picker-free-badge-${model.id}`}
                            className="eo-settings-badge eo-settings-badge-free shrink-0"
                          >
                            Free
                          </span>
                        )}
                        {modelActive && (
                          <Check className="h-3.5 w-3.5 shrink-0 text-sp-text-secondary" />
                        )}
                      </div>
                    );
                  })
                )}
              </div>

              {/* Bottom Reasoning Intensity Slider */}
              {sliderTargetModel && (
                <ReasoningIntensitySlider
                  modelDisplayName={sliderTargetModel.name?.trim() || sliderTargetModel.id}
                  levels={sliderTargetModel.reasoning_levels ?? []}
                  activeLevel={activeSliderLevel}
                  onSelectLevel={(level) =>
                    handleSelectReasoningLevel(activeGroup.channel.id, sliderTargetModel, level)
                  }
                  onCycle={handleCycleReasoningLevel}
                />
              )}
            </div>
          )}
        </div>
  ) : null;

  return (
    <div
      ref={containerRef}
      className={cn(
        "relative min-w-0 shrink",
        compact
          ? "max-w-[104px] @min-[620px]:max-w-[132px] @min-[760px]:max-w-[168px]"
          : "max-w-[130px] @min-[480px]:max-w-[180px] @min-[680px]:max-w-[220px]"
      )}
    >
      <button
        ref={triggerRef}
        type="button"
        data-testid="agent-model-picker-trigger"
        aria-label={t("agent.modelPicker.modelMenuAria")}
        title={
          ready
            ? t("agent.modelPicker.triggerTitle", {
                model: fullLabel,
                channel: channelDisplayName(
                  groups.find((g) => g.channel.id === (selection?.channel_id ?? effectiveDefaultGroup?.channel.id))?.channel ??
                    { id: selection?.channel_id ?? effectiveDefaultGroup?.channel.id ?? "" }
                ),
              })
            : t("agent.modelPicker.configureChannelHint")
        }
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={handleToggleOpen}
        className={cn(
          "flex h-[26px] max-w-full min-w-0 items-center gap-1 rounded-[6px] px-2 text-[12.5px] text-[#71717a] transition-colors hover:bg-[#f4f4f5] hover:text-[#18181b] cursor-pointer outline-none dark:text-[#a1a1aa] dark:hover:bg-[#27272a] dark:hover:text-white",
          open && "bg-[#f4f4f5] text-[#18181b] dark:bg-[#27272a] dark:text-white"
        )}
      >
        <span data-testid="agent-model-picker-label" className="block min-w-0 flex-1 truncate">
          {displayLabel}
        </span>
        {activeModelMeta.isFree === true && (
          <span
            data-testid="agent-model-picker-free-badge"
            className="shrink-0 whitespace-nowrap rounded-[4px] bg-sp-success-bg px-1 py-0 text-xs font-semibold leading-[16px] text-sp-success-text"
          >
            Free
          </span>
        )}
      </button>
      {popoverNode ? (portalTarget ? createPortal(popoverNode, portalTarget) : popoverNode) : null}
    </div>
  );
}
