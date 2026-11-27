/**
 * AgentComposer.tsx
 * EOStage Concentric InputShell Composer (InputShellBeardBand + InputShellEyebrow + InputShellContainer + ComposerToolbar)
 */

import React, { useState, useRef, useEffect, useCallback, useMemo } from "react";
import {
  Plus,
  ArrowUp,
  Square,
  SquareDashed,
  MapPin,
  Upload,
  Workflow,
  Blocks,
  AlertTriangle,
  Search,
  X,
  Paperclip,
  FileText,
} from "lucide-react";
import { getAgentSkills, getEnabledAgentSkills, type AgentSkill } from "@/lib/agentSkills";
import { localizeAgentSkillForDisplay } from "@/lib/pluginsLocale";
import {
  extractSelectedSkillsFromPrompt,
  reconstructPromptWithSkills,
  tokenizeChatEntities,
} from "./agent/chatEntityTokens";
import {
  createImeEnterGuardState,
  onImeCompositionEnd,
  onImeCompositionStart,
  onImeKeyUp,
  shouldBlockKeyDownForIme,
} from "./agent/imeEnterGuard";
import {
  COMPOSER_ATTACHMENT_ACCEPT,
  MAX_COMPOSER_ATTACHMENTS,
  extractClipboardFiles,
  formatAttachmentSize,
  readFileAsComposerAttachment,
  type ComposerAttachment,
} from "./agent/composerAttachments";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import { isMacPlatform } from "@/lib/platformShortcuts";
import {
  agentAttachItems,
  canSendComposer,
  AGENT_VECTOR_EXTENSIONS,
  isAcceptedVectorFileName,
  type AgentAttachItemId,
} from "@/shell/api/agentComposerLogic";
import { AgentPermissionMenu } from "./AgentPermissionMenu";
import { AgentModelPicker } from "./AgentModelPicker";
import { AgentContextHud } from "./AgentContextHud";
import {
  parseStoredModelSelection,
  resolveActiveModelMeta,
} from "@/shell/api/channelModels";
import {
  getPerformanceAndUsagePrefs,
  subscribePerformanceAndUsagePrefs,
  type ContextHudMode,
  type ContextTelemetryEventLike,
  type ContextTelemetryRunSummary,
} from "@/lib/performanceAndUsagePrefs";
import {
  popoverPlacement,
  DESKTOP_TITLEBAR_HEIGHT,
} from "@/shell/api/agentPermissionLogic";
import type {
  AgentPermissionMode,
  AgentPermissionModeView,
  AgentChannelView,
  AgentModelSelection,
} from "../../lib/generated/v3-contracts";

const COMPOSER_MENTION_DATASETS = [
  { id: "Sentinel-1", descKey: "agent.composer.mentionDescSentinel1" },
  { id: "Sentinel-2", descKey: "agent.composer.mentionDescSentinel2" },
  { id: "Landsat-C2L2", descKey: "agent.composer.mentionDescLandsat" },
  { id: "NISAR", descKey: "agent.composer.mentionDescNisar" },
  { id: "Copernicus-DEM", descKey: "agent.composer.mentionDescCopDem" },
  { id: "AW3D30", descKey: "agent.composer.mentionDescAw3d30" },
  { id: "SRTM-GL1", descKey: "agent.composer.mentionDescSrtmGl1" },
  { id: "WHU-CLCD", descKey: "agent.composer.mentionDescWhuClcd" },
  { id: "EOT20", descKey: "agent.composer.mentionDescEot20" },
  { id: "GEBCO-2026", descKey: "agent.composer.mentionDescGebco" },
] as const;

export interface AgentComposerProps {
  compact?: boolean;
  eyebrow?: React.ReactNode;
  value: string;
  onInputChange: (value: string) => void;
  onSubmit: () => void;
  busy?: boolean;
  isRunning?: boolean;
  onStop?: () => void;
  /** 允许在上一轮运行中继续输入与发送（排队或立即引导） */
  allowSendWhileBusy?: boolean;
  sendTitleOverride?: string;
  channelReadiness: {
    ready: boolean;
    disabled: boolean;
    reason_zh: string;
  };
  channels: AgentChannelView[];
  sessionId: string | null;
  onModelChange?: (selection: AgentModelSelection | null) => void;
  onChannelGoSettings?: () => void;
  onOpenPerfSettings?: () => void;
  contextRuns?: readonly ContextTelemetryRunSummary[];
  contextEvents?: readonly ContextTelemetryEventLike[];
  permissionView: AgentPermissionModeView | null;
  permissionStatus: "loading" | "ready" | "error";
  permissionPending?: boolean;
  onPermissionSelect: (mode: AgentPermissionMode) => Promise<void> | void;
  onPermissionSetGlobal: (mode: AgentPermissionMode) => Promise<void> | void;
  onPermissionClearSession?: () => Promise<void> | void;
  onDrawRect?: () => void;
  onOpenAdminRegion?: () => void;
  onVectorFile?: (file: File) => void;
  onSelectWorkflow?: () => void;
  onSelectSkillsTools?: () => void;
  attachments?: readonly ComposerAttachment[];
  onAddAttachments?: (files: File[]) => void | Promise<void>;
  onRemoveAttachment?: (id: string) => void;
  attachmentError?: string | null;
  vectorBusy?: boolean;
  vectorError?: string | null;
  placeholder?: string;
  capabilities?: {
    rectDraw: boolean;
    adminBoundary: boolean;
    vectorUpload: boolean;
    workflow?: boolean;
    skillsTools?: boolean;
  };
}

/** Slash-command trigger SVG icon */
function IconSlashSquare({ className = "h-[15px] w-[15px]" }: { className?: string }) {
  return (
    <svg viewBox="0 0 20 20" fill="none" className={className} aria-hidden="true">
      <rect
        x="3"
        y="3"
        width="14"
        height="14"
        rx="4"
        stroke="currentColor"
        strokeWidth="1.5"
      />
      <path
        d="M11.75 6.25L8.25 13.75"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function AgentComposer({
  compact = false,
  eyebrow,
  value,
  onInputChange,
  onSubmit,
  busy = false,
  isRunning = false,
  onStop,
  allowSendWhileBusy = false,
  sendTitleOverride,
  channelReadiness,
  channels,
  sessionId,
  onModelChange,
  onChannelGoSettings,
  contextRuns = [],
  contextEvents = [],
  permissionView,
  permissionStatus,
  permissionPending = false,
  onPermissionSelect,
  onPermissionSetGlobal,
  onPermissionClearSession,
  onDrawRect,
  onOpenAdminRegion,
  onVectorFile,
  attachments: controlledAttachments,
  onAddAttachments,
  onRemoveAttachment,
  attachmentError: controlledAttachmentError = null,
  vectorBusy = false,
  vectorError = null,
  placeholder,
  capabilities = {
    rectDraw: true,
    adminBoundary: true,
    vectorUpload: true,
    workflow: true,
    skillsTools: true,
  },
  onSelectWorkflow,
  onSelectSkillsTools,
}: AgentComposerProps) {
  const { t, locale } = useI18n();
  const [attachMenuOpen, setAttachMenuOpen] = useState(false);
  const [skillPickerOpen, setSkillPickerOpen] = useState(false);
  const [skillFilter, setSkillFilter] = useState("");
  const [mentionPickerOpen, setMentionPickerOpen] = useState(false);
  const [mentionFilter, setMentionFilter] = useState("");
  const [pickerCloseNonce, setPickerCloseNonce] = useState(0);
  const [triedSubmitWithoutModel, setTriedSubmitWithoutModel] = useState(false);
  const [isDragOver, setIsDragOver] = useState(false);
  const [localAttachments, setLocalAttachments] = useState<ComposerAttachment[]>([]);
  const [localAttachmentError, setLocalAttachmentError] = useState<string | null>(null);
  const effectiveAttachments = controlledAttachments ?? localAttachments;
  const effectiveAttachmentError = controlledAttachmentError ?? localAttachmentError;
  const [hudMode, setHudMode] = useState<ContextHudMode>(
    () => getPerformanceAndUsagePrefs().hudMode
  );
  const [selectedModel, setSelectedModel] = useState<AgentModelSelection | null>(() => {
    try {
      return typeof window !== "undefined"
        ? parseStoredModelSelection(window.localStorage.getItem("eostage.agent.modelSelection"))
        : null;
    } catch {
      return null;
    }
  });
  const activeModelMeta = useMemo(
    () => resolveActiveModelMeta(channels, selectedModel),
    [channels, selectedModel]
  );
  const skillPickerRef = useRef<HTMLDivElement | null>(null);
  const skillSearchInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (channelReadiness.ready) {
      setTriedSubmitWithoutModel(false);
    }
  }, [channelReadiness.ready]);

  useEffect(() => {
    return subscribePerformanceAndUsagePrefs((prefs) => {
      setHudMode(prefs.hudMode);
    });
  }, []);

  const knownSkills = useMemo(
    () => getAgentSkills().map((s) => localizeAgentSkillForDisplay(s, locale)),
    [skillPickerOpen, locale]
  );

  const allSkills = useMemo(() => {
    return skillPickerOpen
      ? getEnabledAgentSkills().map((s) => localizeAgentSkillForDisplay(s, locale))
      : [];
  }, [skillPickerOpen, locale]);

  const { selectedSkills, cleanText } = useMemo(
    () => extractSelectedSkillsFromPrompt(value, knownSkills),
    [value, knownSkills]
  );

  const inlineEntityTokens = useMemo(
    () => tokenizeChatEntities(cleanText).filter((seg) => seg.kind !== "text"),
    [cleanText]
  );

  const filteredSkills = useMemo(() => {
    const q = skillFilter.trim().toLowerCase();
    if (!q) return allSkills;
    return allSkills.filter(
      (s) =>
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        Boolean(s.slashCommand && s.slashCommand.toLowerCase().includes(q)) ||
        Boolean(s.tags && s.tags.some((tag: string) => tag.toLowerCase().includes(q)))
    );
  }, [allSkills, skillFilter]);

  const filteredMentions = useMemo(() => {
    const q = mentionFilter.trim().toLowerCase();
    if (!q) return COMPOSER_MENTION_DATASETS;
    return COMPOSER_MENTION_DATASETS.filter(
      (d) =>
        d.id.toLowerCase().includes(q) || t(d.descKey).toLowerCase().includes(q)
    );
  }, [mentionFilter, t]);

  const handleSelectDatasetMention = (datasetId: string) => {
    const replaced = /(?:^|\s)@[^\s@]*$/.test(cleanText)
      ? cleanText.replace(/(^|\s)@[^\s@]*$/, `$1@${datasetId} `)
      : `${cleanText}${cleanText && !/\s$/.test(cleanText) ? " " : ""}@${datasetId} `;
    onInputChange(reconstructPromptWithSkills(replaced, selectedSkills));
    setMentionPickerOpen(false);
    setTimeout(() => {
      if (textareaRef.current) {
        textareaRef.current.focus();
        textareaRef.current.selectionStart = textareaRef.current.value.length;
        textareaRef.current.selectionEnd = textareaRef.current.value.length;
      }
    }, 30);
  };

  const handleSelectSkill = (skill: AgentSkill) => {
    const cleanedDraft =
      skillPickerOpen && /(?:^|\s)\/[^\s/]*$/.test(cleanText)
        ? cleanText.replace(/(?:^|\s)\/[^\s/]*$/, "")
        : cleanText;
    const nextSkills = selectedSkills.some((s) => s.id === skill.id)
      ? selectedSkills
      : [...selectedSkills, skill];
    onInputChange(reconstructPromptWithSkills(cleanedDraft, nextSkills));
    setSkillPickerOpen(false);
    setTimeout(() => {
      if (textareaRef.current) {
        textareaRef.current.focus();
        textareaRef.current.selectionStart = textareaRef.current.value.length;
        textareaRef.current.selectionEnd = textareaRef.current.value.length;
      }
    }, 50);
  };

  const handleRemoveSelectedSkill = (skillId: string) => {
    const remaining = selectedSkills.filter((s) => s.id !== skillId);
    onInputChange(reconstructPromptWithSkills(cleanText, remaining));
  };

  const handleTextareaChange = (nextClean: string) => {
    setPickerCloseNonce((prev) => prev + 1);
    onInputChange(reconstructPromptWithSkills(nextClean, selectedSkills));
    const slashMatch = /(?:^|\s)\/([^\s/]*)$/.exec(nextClean);
    if (slashMatch) {
      setMentionPickerOpen(false);
      setSkillFilter(slashMatch[1]);
      setSkillPickerOpen(true);
      return;
    }
    const atMatch = /(?:^|\s)@([^\s@]*)$/.exec(nextClean);
    if (atMatch) {
      setSkillPickerOpen(false);
      setMentionFilter(atMatch[1]);
      setMentionPickerOpen(true);
      return;
    }
    if (mentionPickerOpen) {
      setMentionPickerOpen(false);
    }
  };

  const containerRef = useRef<HTMLDivElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const attachmentInputRef = useRef<HTMLInputElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const imeGuardRef = useRef(createImeEnterGuardState());
  const attachBtnRef = useRef<HTMLButtonElement | null>(null);
  const attachContainerRef = useRef<HTMLDivElement | null>(null);
  const attachMenuRef = useRef<HTMLDivElement | null>(null);
  const skillBtnRef = useRef<HTMLButtonElement | null>(null);

  const [attachPlacement, setAttachPlacement] = useState<{ side: "up" | "down"; maxHeight: number }>({
    side: "up",
    maxHeight: 340,
  });
  const [skillPlacement, setSkillPlacement] = useState<{ side: "up" | "down"; maxHeight: number }>({
    side: "up",
    maxHeight: 336,
  });
  const [attachHighlight, setAttachHighlight] = useState<{
    itemId: string | null;
    y: number;
    h: number;
  }>({
    itemId: null,
    y: 0,
    h: 32,
  });

  const updateAttachHighlight = (el: HTMLElement, itemId: string, index: number) => {
    const rawTop = el.offsetTop;
    const rawHeight = el.offsetHeight;
    setAttachHighlight({
      itemId,
      y: rawHeight > 0 ? rawTop : index * 32,
      h: rawHeight > 0 ? rawHeight : 32,
    });
  };

  const attachItems = agentAttachItems(capabilities);

  const updateAttachPlacement = useCallback(() => {
    const el = attachBtnRef.current ?? containerRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vh = typeof window !== "undefined" ? window.innerHeight : 800;
    const desiredHeight = Math.max(268, (attachItems.length + 1) * 40 + 24);
    setAttachPlacement(
      popoverPlacement({
        triggerTop: rect.top,
        triggerBottom: rect.bottom,
        viewportHeight: vh,
        desiredHeight,
        topInset: DESKTOP_TITLEBAR_HEIGHT,
        margin: 8,
      })
    );
  }, [attachItems.length]);

  useEffect(() => {
    if (!attachMenuOpen) return;
    updateAttachPlacement();
    const handleResize = () => updateAttachPlacement();
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [attachMenuOpen, updateAttachPlacement]);

  const updateSkillPlacement = useCallback(() => {
    const el = skillBtnRef.current ?? containerRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vh = typeof window !== "undefined" ? window.innerHeight : 800;
    setSkillPlacement(
      popoverPlacement({
        triggerTop: rect.top,
        triggerBottom: rect.bottom,
        viewportHeight: vh,
        desiredHeight: 336,
        topInset: DESKTOP_TITLEBAR_HEIGHT + 12,
        margin: 8,
      })
    );
  }, []);

  useEffect(() => {
    if (!skillPickerOpen) return;
    updateSkillPlacement();
    const handleResize = () => updateSkillPlacement();
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [skillPickerOpen, updateSkillPlacement]);

  useEffect(() => {
    if (!attachMenuOpen) return;
    const handleOutsideClick = (e: MouseEvent) => {
      if (
        attachContainerRef.current &&
        !attachContainerRef.current.contains(e.target as Node)
      ) {
        setAttachMenuOpen(false);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setAttachMenuOpen(false);
        attachBtnRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", handleOutsideClick);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleOutsideClick);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [attachMenuOpen]);

  useEffect(() => {
    if (!skillPickerOpen) return;
    const handleOutsideClick = (e: MouseEvent) => {
      if (
        skillPickerRef.current &&
        !skillPickerRef.current.contains(e.target as Node)
      ) {
        setSkillPickerOpen(false);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setSkillPickerOpen(false);
        textareaRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", handleOutsideClick);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleOutsideClick);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [skillPickerOpen]);

  const effectiveBusyLock = busy && !allowSendWhileBusy;

  const canSend =
    canSendComposer({
      text: value,
      channelReady: channelReadiness.ready,
      busy: effectiveBusyLock,
    }) ||
    (effectiveAttachments.length > 0 && channelReadiness.ready && !effectiveBusyLock);

  const handleAddFiles = useCallback(
    async (files: File[]) => {
      if (files.length === 0) return;
      if (onAddAttachments) {
        await onAddAttachments(files);
        return;
      }
      setLocalAttachmentError(null);
      const nextItems: ComposerAttachment[] = [];
      let firstError: string | null = null;
      const remainingSlots = Math.max(0, MAX_COMPOSER_ATTACHMENTS - localAttachments.length);
      if (remainingSlots <= 0) {
        setLocalAttachmentError(t("agent.composer.maxAttachmentsError", { max: MAX_COMPOSER_ATTACHMENTS }));
        return;
      }
      if (files.length > remainingSlots) {
        firstError = t("agent.composer.maxAttachmentsTruncatedError", {
          max: MAX_COMPOSER_ATTACHMENTS,
          count: remainingSlots,
        });
      }
      for (const file of files.slice(0, remainingSlots)) {
        const res = await readFileAsComposerAttachment(file);
        if (res.attachment) {
          nextItems.push(res.attachment);
        } else if (res.error && !firstError) {
          firstError = res.error;
        }
      }
      if (nextItems.length > 0) {
        setLocalAttachments((prev) => [...prev, ...nextItems].slice(0, MAX_COMPOSER_ATTACHMENTS));
      }
      if (firstError) {
        setLocalAttachmentError(firstError);
      }
    },
    [localAttachments.length, onAddAttachments]
  );

  const handleRemoveAttachmentItem = useCallback(
    (id: string) => {
      if (onRemoveAttachment) {
        onRemoveAttachment(id);
        return;
      }
      setLocalAttachments((prev) => prev.filter((item) => item.id !== id));
      setLocalAttachmentError(null);
    },
    [onRemoveAttachment]
  );

  const handleTriggerSubmit = useCallback(() => {
    setPickerCloseNonce((prev) => prev + 1);
    if (!canSend) return;
    onSubmit();
  }, [canSend, onSubmit]);

  const handleCompositionStart = useCallback(() => {
    onImeCompositionStart(imeGuardRef.current);
  }, []);

  const handleCompositionEnd = useCallback(() => {
    onImeCompositionEnd(imeGuardRef.current, Date.now());
  }, []);

  const handleKeyUp = useCallback((e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    onImeKeyUp(imeGuardRef.current, e.key);
  }, []);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (
        shouldBlockKeyDownForIme(imeGuardRef.current, {
          key: e.key,
          nativeIsComposing: Boolean(e.nativeEvent.isComposing),
          keyCode: (e as unknown as { keyCode?: number }).keyCode,
          nowMs: Date.now(),
        })
      ) {
        return;
      }
      // 空输入框下按 `/` 键真实唤起检索技能选择器
      if (e.key === "/" && !value.trim() && !e.ctrlKey && !e.metaKey && !e.altKey) {
        e.preventDefault();
        setSkillFilter("");
        setSkillPickerOpen(true);
        setTimeout(() => skillSearchInputRef.current?.focus(), 30);
        return;
      }
      if (e.key === "Escape" && (skillPickerOpen || mentionPickerOpen)) {
        e.preventDefault();
        setSkillPickerOpen(false);
        setMentionPickerOpen(false);
        return;
      }
      if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
        e.preventDefault();
        if (canSend) {
          handleTriggerSubmit();
        } else if (!channelReadiness.ready) {
          setTriedSubmitWithoutModel(true);
        }
      }
    },
    [canSend, channelReadiness.ready, handleTriggerSubmit, mentionPickerOpen, skillPickerOpen, value]
  );

  const handlePaste = useCallback(
    (e: React.ClipboardEvent<HTMLElement>) => {
      const files = extractClipboardFiles(e.clipboardData);
      if (files.length === 0) return;
      e.preventDefault();
      void handleAddFiles(files);
    },
    [handleAddFiles]
  );

  const handleAttachItemClick = (id: AgentAttachItemId | "media-upload") => {
    setAttachMenuOpen(false);
    if (id === "media-upload") {
      attachmentInputRef.current?.click();
    } else if (id === "rect-draw") {
      onDrawRect?.();
    } else if (id === "admin-region") {
      onOpenAdminRegion?.();
    } else if (id === "vector-upload") {
      fileInputRef.current?.click();
    } else if (id === "workflow") {
      if (onSelectWorkflow) {
        onSelectWorkflow();
      } else {
        setSkillFilter("");
        setSkillPickerOpen(true);
      }
    } else if (id === "skills-tools") {
      if (onSelectSkillsTools) {
        onSelectSkillsTools();
      } else {
        setSkillFilter("");
        setSkillPickerOpen(true);
      }
    }
  };

  const handleFileInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file && isAcceptedVectorFileName(file.name)) {
      onVectorFile?.(file);
    }
    e.target.value = "";
  };

  const handleAttachmentInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files ?? []);
    if (files.length > 0) {
      void handleAddFiles(files);
    }
    e.target.value = "";
  };

  const handleDragOver = (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    if (!isDragOver) setIsDragOver(true);
  };

  const handleDragLeave = (e: React.DragEvent<HTMLDivElement>) => {
    if (containerRef.current && !containerRef.current.contains(e.relatedTarget as Node | null)) {
      setIsDragOver(false);
    }
  };

  const handleDrop = (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(false);
    const files = Array.from(e.dataTransfer?.files ?? []);
    if (files.length === 0) return;
    if (
      onVectorFile &&
      files.length === 1 &&
      isAcceptedVectorFileName(files[0].name) &&
      !files[0].name.toLowerCase().endsWith(".json")
    ) {
      onVectorFile(files[0]);
      return;
    }
    void handleAddFiles(files);
  };

  const completedRunsCount = useMemo(
    () => contextRuns.filter((r) => r.status === "completed").length,
    [contextRuns]
  );

  return (
    <div
      ref={containerRef}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
      className="cds-composer-shell relative @container w-full"
    >
      {/* Hidden vector file input */}
      <input
        ref={fileInputRef}
        type="file"
        accept={AGENT_VECTOR_EXTENSIONS.join(",")}
        aria-label={t("agent.composer.uploadVector")}
        className="hidden"
        onChange={handleFileInputChange}
      />
      {/* Hidden multimodal image/file attachment input */}
      <input
        ref={attachmentInputRef}
        type="file"
        multiple
        accept={COMPOSER_ATTACHMENT_ACCEPT}
        aria-label={t("agent.composer.attachMediaLabel")}
        data-testid="composer-attachment-file-input"
        className="hidden"
        onChange={handleAttachmentInputChange}
      />

      {/* Outer Concentric `InputShellBeardBand` (Neutral concentric desktop shell) */}
      <div
        data-testid="eo-input-shell-band"
        className="eo-concentric-band rounded-[16px] border border-sp-line bg-sp-surface-muted p-[3px] shadow-card transition-colors"
      >
        {/* Top Concentric `InputShellEyebrow` — EOStage 实时空间与渠道上下文眉栏 */}
        {eyebrow ? (
          <div className="px-3 py-1.5 text-[12px] text-sp-text-secondary">
            {eyebrow}
          </div>
        ) : (
          <div className="flex items-center justify-between px-3.5 py-1.5 text-[12px]">
            {vectorError ? (
              <div className="flex items-center gap-1.5 text-rose-600 dark:text-rose-400">
                <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                <span className="truncate">{vectorError}</span>
              </div>
            ) : vectorBusy ? (
              <div className="flex items-center gap-1.5 text-sp-text-secondary">
                <span className="h-2 w-2 rounded-full bg-sp-accent" />
                <span>{t("agent.composer.vectorImporting")}</span>
              </div>
            ) : sessionId && contextRuns.length > 0 ? (
              <div className="flex items-center gap-2 text-sp-text-secondary">
                <span className="h-2 w-2 rounded-full bg-sp-status-success" />
                <span>
                  {t("agent.composer.completedRunsSummary", {
                    completed: completedRunsCount,
                    total: contextRuns.length,
                  })}
                </span>
              </div>
            ) : !channelReadiness.ready ? (
              <span className="truncate text-sp-text-secondary">
                {locale === "zh-CN" && channelReadiness.reason_zh
                  ? channelReadiness.reason_zh
                  : t("agent.composer.unconfiguredModelShort")}
              </span>
            ) : (
              <span className="truncate text-sp-text-secondary">
                {t("agent.composer.defaultStatus")}
              </span>
            )}

            <div className="ml-3 flex items-center gap-2.5 shrink-0 whitespace-nowrap">
              {onOpenAdminRegion && (
                <>
                  <button
                    type="button"
                    data-testid="eyebrow-admin-region-btn"
                    onClick={onOpenAdminRegion}
                    className="font-medium text-sp-accent hover:underline cursor-pointer"
                  >
                    {t("agent.composer.regionLink")}
                  </button>
                  <span className="text-sp-line">·</span>
                </>
              )}
              <button
                type="button"
                onClick={() => onChannelGoSettings?.()}
                className="font-medium text-sp-accent hover:underline cursor-pointer"
              >
                {!channelReadiness.ready ? t("agent.composer.configureModel") : t("agent.composer.modelsAndSources")}
              </button>
            </div>
          </div>
        )}

        {/* Inner `InputShellContainer` */}
        <div
          data-testid="eo-input-shell-inner"
          className={cn(
            "eo-concentric-inner relative rounded-[13px] border border-[#e4e4e7] bg-white px-3.5 pt-3 pb-2.5 transition-colors dark:border-[#323238] dark:bg-[#1e1e22]",
            isDragOver && "ring-1 ring-sp-accent border-sp-accent bg-sp-surface-muted/40"
          )}
        >
          {effectiveAttachments.length > 0 && (
            <div
              data-testid="composer-attachments-strip"
              className="mb-2 flex flex-wrap items-center gap-2 px-1"
            >
              {effectiveAttachments.map((att) =>
                att.kind === "image" && att.previewUrl ? (
                  <div
                    key={att.id}
                    data-testid={`composer-attachment-item-${att.id}`}
                    data-attachment-kind="image"
                    title={`${att.name} (${formatAttachmentSize(att.sizeBytes)})`}
                    className="group/att relative h-12 w-12 shrink-0 overflow-hidden rounded-[8px] border border-sp-line bg-sp-surface-muted"
                  >
                    <img
                      src={att.previewUrl}
                      alt={att.name}
                      className="h-full w-full object-cover"
                    />
                    <button
                      type="button"
                      data-testid={`composer-attachment-remove-${att.id}`}
                      aria-label={t("agent.composer.removeAttachmentAria", { name: att.name })}
                      onClick={() => handleRemoveAttachmentItem(att.id)}
                      className="absolute top-0.5 right-0.5 inline-flex h-4 w-4 items-center justify-center rounded-full bg-black/70 text-white hover:bg-black cursor-pointer transition-opacity"
                    >
                      <X className="h-2.5 w-2.5" />
                    </button>
                  </div>
                ) : (
                  <div
                    key={att.id}
                    data-testid={`composer-attachment-item-${att.id}`}
                    data-attachment-kind={att.kind}
                    title={`${att.name} (${formatAttachmentSize(att.sizeBytes)})`}
                    className="inline-flex max-w-[220px] items-center gap-1.5 rounded-[8px] border border-sp-line bg-sp-surface-muted px-2.5 py-1.5 text-xs text-sp-text"
                  >
                    <FileText className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                    <span className="min-w-0 truncate">{att.name}</span>
                    {att.sizeBytes > 0 && (
                      <span className="shrink-0 text-xs text-sp-text-muted tabular-nums">
                        {formatAttachmentSize(att.sizeBytes)}
                      </span>
                    )}
                    <button
                      type="button"
                      data-testid={`composer-attachment-remove-${att.id}`}
                      aria-label={t("agent.composer.removeAttachmentAria", { name: att.name })}
                      onClick={() => handleRemoveAttachmentItem(att.id)}
                      className="ml-0.5 inline-flex h-4 w-4 shrink-0 items-center justify-center rounded text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </div>
                )
              )}
            </div>
          )}

          {effectiveAttachmentError && (
            <div
              data-testid="composer-attachment-error"
              className="mb-1.5 flex items-center gap-1.5 px-1 text-xs text-sp-text-secondary"
            >
              <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-sp-status-warning" />
              <span className="min-w-0 truncate">{effectiveAttachmentError}</span>
            </div>
          )}
          {(selectedSkills.length > 0 || inlineEntityTokens.length > 0) && (
            <div
              data-testid="composer-skill-chips"
              className="mb-1.5 flex flex-wrap items-center gap-1.5 px-1"
            >
              {selectedSkills.map((skill) => (
                <span
                  key={skill.id}
                  data-testid={`composer-skill-chip-${skill.id}`}
                  className="sp-chat-token-chip sp-chat-token-chip--skill sp-chip-enter px-2 rounded-[6px] gap-1.5"
                >
                  <span>{skill.name}</span>
                  <button
                    type="button"
                    aria-label={t("agent.composer.removeSkillAria", { name: skill.name })}
                    onClick={() => handleRemoveSelectedSkill(skill.id)}
                    className="ml-0.5 inline-flex h-3.5 w-3.5 items-center justify-center rounded hover:bg-black/10 dark:hover:bg-white/15 cursor-pointer"
                  >
                    <X className="h-2.5 w-2.5" />
                  </button>
                </span>
              ))}
              {inlineEntityTokens.map((tok, idx) => (
                <span
                  key={`${tok.kind}-${idx}-${tok.value}`}
                  data-token-kind={tok.kind}
                  className={cn(
                    "sp-chat-token-chip",
                    tok.kind === "skill" && "sp-chat-token-chip--skill",
                    tok.kind === "dataset" && "sp-chat-token-chip--dataset",
                    tok.kind === "aoi" && "sp-chat-token-chip--aoi"
                  )}
                >
                  {tok.kind === "skill" ? tok.value.replace(/^\/+/, "") : tok.value}
                </span>
              ))}
            </div>
          )}

          {/* Rich `<kbd>` Placeholder Overlay when `value === ""` */}
          <div className="relative">
            {!value && (
              <div
                aria-hidden="true"
                onClick={() => textareaRef.current?.focus()}
                className="pointer-events-none absolute top-0.5 left-1 flex flex-wrap items-center gap-1 text-[13.5px] leading-[1.5] text-[#71717a] select-none dark:text-[#a1a1aa]"
              >
                <span>{t("agent.composer.richPlaceholderPrefix")}</span>
                <kbd className="inline-flex h-[18px] items-center justify-center rounded-[4px] border border-[#e4e4e7] bg-[#f4f4f5] px-1.5 font-sans text-xs font-normal text-[#52525b] dark:border-[#3f3f46] dark:bg-[#27272a] dark:text-[#a1a1aa]">
                  /
                </kbd>
                <span>{t("agent.composer.richPlaceholderSuffix")}</span>
              </div>
            )}

            <textarea
              ref={textareaRef}
              value={cleanText}
              disabled={effectiveBusyLock}
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
              onChange={(e) => handleTextareaChange(e.target.value)}
              onKeyDown={handleKeyDown}
              onKeyUp={handleKeyUp}
              onCompositionStart={handleCompositionStart}
              onCompositionEnd={handleCompositionEnd}
              onPaste={handlePaste}
              placeholder={placeholder ?? t("agent.composer.defaultPlaceholder")}
              aria-label={t("agent.composer.inputAria")}
              className={cn(
                "w-full resize-none border-0 bg-transparent p-1 text-[13.5px] leading-relaxed text-[#18181b] placeholder:text-transparent outline-none focus:ring-0 disabled:cursor-not-allowed dark:text-[#f4f4f5]",
                compact ? "min-h-[44px]" : "min-h-[60px]"
              )}
            />

            {mentionPickerOpen && filteredMentions.length > 0 && (
              <div
                data-testid="agent-dataset-mention-picker"
                role="listbox"
                aria-label={t("agent.composer.mentionPickerAria")}
                className="sp-popover-blur-in absolute bottom-[calc(100%+6px)] left-0 z-50 w-[300px] max-w-full rounded-[12px] border border-sp-line bg-sp-surface shadow-overlay flex flex-col overflow-hidden select-none"
              >
                <div className="shrink-0 border-b border-sp-line/60 px-2.5 py-1.5 text-xs font-semibold text-sp-text-muted">
                  {t("agent.composer.mentionPickerTitle")}
                </div>
                <div className="max-h-[212px] overflow-y-auto p-1.5 flex flex-col gap-0.5">
                  {filteredMentions.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      data-testid={`mention-dataset-${item.id}`}
                      onClick={() => handleSelectDatasetMention(item.id)}
                      className="shrink-0 flex flex-col gap-0.5 rounded-[7px] px-2 py-1.5 text-left hover:bg-sp-hover cursor-pointer transition-colors"
                    >
                      <span className="font-mono text-xs font-medium text-sp-text">
                        @{item.id}
                      </span>
                      <span className="truncate text-xs text-sp-text-muted">
                        {t(item.descKey)}
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            )}
          </div>

          {!channelReadiness.ready && (Boolean(cleanText.trim()) || triedSubmitWithoutModel) && (
            <div
              data-testid="composer-model-required-hint"
              className="mt-1.5 flex items-center justify-between gap-2 rounded-[7px] border border-sp-line bg-sp-warning-bg/70 px-2.5 py-1 text-xs text-sp-warning-text"
            >
              <span className="min-w-0 truncate">
                {t("agent.composer.unconfiguredModelBanner")}
              </span>
              {onChannelGoSettings && (
                <button
                  type="button"
                  data-testid="composer-go-model-settings-btn"
                  onClick={onChannelGoSettings}
                  className="shrink-0 font-medium text-sp-accent hover:underline cursor-pointer"
                >
                  {t("agent.composer.goConfigureModel")}
                </button>
              )}
            </div>
          )}

          {/* Bottom `ComposerToolbar` — 单行不换行 */}
          <div
            data-testid="agent-composer-toolbar"
            className="flex flex-nowrap items-center justify-between gap-2 pt-1 min-w-0"
          >
            {/* Left: `+` Attach, `[/]` Slash Skills, `SlidersHorizontal + Agent` Permission */}
            <div className="flex items-center gap-0.5 shrink-0">
              {attachItems.length > 0 && (
                <div ref={attachContainerRef} className="relative inline-flex items-center gap-0.5 shrink-0">
                  {/* 1. `+` Button */}
                  <button
                    ref={attachBtnRef}
                    type="button"
                    onClick={() => setAttachMenuOpen((prev) => !prev)}
                    aria-label={t("agent.composer.attachAria")}
                    title={t("agent.composer.attachAria")}
                    aria-haspopup="menu"
                    aria-expanded={attachMenuOpen}
                    className={cn(
                      "cds-btn-squish flex h-7 w-7 items-center justify-center rounded-[7px] text-[#52525b] transition-colors cursor-pointer outline-none dark:text-[#a1a1aa]",
                      attachMenuOpen
                        ? "bg-[#f4f4f5] text-[#18181b] dark:bg-[#27272a] dark:text-white"
                        : "hover:bg-[#f4f4f5] hover:text-[#18181b] dark:hover:bg-[#27272a] dark:hover:text-white"
                    )}
                  >
                    <Plus
                      className={cn(
                        "h-[16px] w-[16px] stroke-[1.75] sp-attach-plus-icon",
                        attachMenuOpen && "is-open"
                      )}
                    />
                  </button>

                  {/* 2. `[/]` Slash Skills Button */}
                  <button
                    ref={skillBtnRef}
                    type="button"
                    onClick={() => {
                      setSkillFilter("");
                      updateSkillPlacement();
                      setSkillPickerOpen((prev) => !prev);
                    }}
                    aria-label={t("agent.composer.skillPickerTitle")}
                    title={t("agent.composer.skillPickerTitle")}
                    className={cn(
                      "cds-btn-squish flex h-7 w-7 items-center justify-center rounded-[7px] text-[#52525b] transition-colors cursor-pointer outline-none dark:text-[#a1a1aa]",
                      skillPickerOpen
                        ? "bg-[#f4f4f5] text-[#18181b] dark:bg-[#27272a] dark:text-white"
                        : "hover:bg-[#f4f4f5] hover:text-[#18181b] dark:hover:bg-[#27272a] dark:hover:text-white"
                    )}
                  >
                    <IconSlashSquare className="h-[15px] w-[15px]" />
                  </button>

                  {/* Skills & Playbooks Popover */}
                  {skillPickerOpen && (
                    <div
                      ref={skillPickerRef}
                      role="dialog"
                      aria-label={t("agent.composer.skillPickerTitle")}
                      data-testid="agent-skill-picker"
                      data-placement={skillPlacement.side}
                      style={{
                        maxHeight: skillPlacement.maxHeight > 0 ? `${skillPlacement.maxHeight}px` : undefined,
                      }}
                      className={cn(
                        "sp-popover-blur-in absolute z-50 -left-[15px] w-[344px] max-w-[calc(100vw-32px)] rounded-[14px] bg-sp-surface p-2 shadow-overlay border border-sp-line flex flex-col gap-2 select-none overflow-hidden",
                        skillPlacement.side === "up"
                          ? "bottom-[calc(100%+8px)] origin-bottom-left"
                          : "top-[calc(100%+8px)] origin-top-left"
                      )}
                    >
                      <div className="flex items-center justify-between px-2 pt-0.5 pb-1.5 border-b border-sp-divider">
                        <div className="flex items-center gap-1.5 text-xs font-semibold text-sp-text">
                          <Blocks className="h-3.5 w-3.5 text-sp-text-secondary shrink-0 stroke-[1.75]" />
                          <span>{t("agent.composer.skillPickerTitle")}</span>
                        </div>
                        <button
                          type="button"
                          onClick={() => setSkillPickerOpen(false)}
                          className="p-1 text-sp-text-muted hover:text-sp-text rounded-[6px] hover:bg-sp-hover text-xs transition-colors cursor-pointer"
                          aria-label="Close"
                        >
                          <X className="h-3.5 w-3.5" />
                        </button>
                      </div>

                      <div className="relative">
                        <Search className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-sp-text-muted" />
                        <input
                          ref={skillSearchInputRef}
                          type="text"
                          value={skillFilter}
                          onChange={(e) => setSkillFilter(e.target.value)}
                          placeholder={t("agent.composer.skillPickerSearch")}
                          className="w-full sp-control-sm pl-7 pr-2.5 text-xs rounded-[6px] bg-sp-surface-muted border border-sp-line text-sp-text placeholder:text-sp-text-muted focus:outline-none focus:border-sp-accent"
                        />
                      </div>

                      <div className="min-h-0 max-h-[228px] flex-1 overflow-y-auto flex flex-col gap-1">
                        {filteredSkills.length === 0 ? (
                          <div className="py-5 text-center text-xs text-sp-text-muted">
                            {t("agent.composer.skillPickerEmpty")}
                          </div>
                        ) : (
                          filteredSkills.map((skill) => (
                            <div
                              key={skill.id}
                              role="button"
                              tabIndex={0}
                              data-testid={`skill-picker-item-${skill.id}`}
                              onClick={() => handleSelectSkill(skill)}
                              onKeyDown={(e) => {
                                if (e.key === "Enter" || e.key === " ") {
                                  e.preventDefault();
                                  handleSelectSkill(skill);
                                }
                              }}
                              className="shrink-0 flex flex-col gap-0.5 px-2 py-2 rounded-[6px] hover:bg-sp-hover cursor-pointer transition-colors text-left"
                            >
                              <div className="flex items-center justify-between gap-1">
                                <span className="text-xs font-medium leading-[18px] text-sp-text truncate">
                                  {skill.name}
                                </span>
                              </div>
                              <p className="text-xs leading-[18px] text-sp-text-muted line-clamp-1">{skill.description}</p>
                            </div>
                          ))
                        )}
                      </div>

                      <div className="px-2 pt-1.5 pb-0.5 border-t border-sp-divider flex items-center justify-between text-xs">
                        <span className="text-xs text-sp-text-muted">{t("agent.composer.skillPickerManage")}</span>
                        <button
                          type="button"
                          onClick={() => {
                            setSkillPickerOpen(false);
                            window.location.hash = "#/plugins";
                          }}
                          className="text-xs text-sp-accent hover:underline flex items-center gap-1 font-medium cursor-pointer"
                        >
                          <Blocks className="h-3.5 w-3.5 shrink-0 stroke-[1.75]" />
                          <span>{t("shell.nav.plugins")}</span>
                        </button>
                      </div>
                    </div>
                  )}

                  {/* Attach Menu Popover */}
                  {attachMenuOpen && (
                    <div
                      ref={attachMenuRef}
                      role="menu"
                      aria-label={t("agent.composer.attach")}
                      data-testid="agent-attach-menu"
                      data-placement={attachPlacement.side}
                      onMouseLeave={() => setAttachHighlight((prev) => ({ ...prev, itemId: null }))}
                      style={{
                        maxHeight:
                          attachPlacement.maxHeight > 0
                            ? `${Math.max(attachPlacement.maxHeight, 268)}px`
                            : undefined,
                      }}
                      className={cn(
                        "sp-popover-blur-in absolute z-40 w-max min-w-[296px] max-w-[min(372px,calc(100vw-32px))] rounded-[12px] bg-sp-surface p-1.5 shadow-overlay border border-sp-line flex flex-col overflow-y-auto",
                        attachPlacement.side === "up"
                          ? "bottom-[calc(100%+8px)] -left-[15px] origin-bottom-left"
                          : "top-[calc(100%+8px)] -left-[15px] origin-top-left"
                      )}
                    >
                      <div className="relative flex flex-col">
                        <div
                          aria-hidden="true"
                          data-testid="attach-menu-fluid-highlight"
                          data-active-item={attachHighlight.itemId ?? ""}
                          className={cn(
                            "sp-fluid-menu-highlight",
                            attachHighlight.itemId !== null && "is-active"
                          )}
                          style={
                            {
                              "--sp-highlight-y": `${attachHighlight.y}px`,
                              "--sp-highlight-h": `${attachHighlight.h}px`,
                            } as React.CSSProperties
                          }
                        />
                        {attachItems.map((item, idx) => (
                          <div
                            key={item.id}
                            role="menuitem"
                            data-testid={`attach-menu-item-${item.id}`}
                            tabIndex={0}
                            onMouseEnter={(e) => updateAttachHighlight(e.currentTarget, item.id, idx)}
                            onFocus={(e) => updateAttachHighlight(e.currentTarget, item.id, idx)}
                            onClick={() => handleAttachItemClick(item.id)}
                            onKeyDown={(e) => {
                              if (e.key === "Enter" || e.key === " ") {
                                e.preventDefault();
                                handleAttachItemClick(item.id);
                              }
                            }}
                            className="sp-fluid-menu-item sp-row-balanced flex items-center gap-2.5 px-2.5 rounded-[6px] text-xs text-[#18181b] dark:text-[#f4f4f5] cursor-pointer transition-colors outline-none"
                          >
                            {item.id === "rect-draw" && <SquareDashed className="sp-menu-item-icon h-4 w-4 text-[#52525b] shrink-0" />}
                            {item.id === "admin-region" && <MapPin className="sp-menu-item-icon h-4 w-4 text-[#52525b] shrink-0" />}
                            {item.id === "vector-upload" && <Upload className="sp-menu-item-icon h-4 w-4 text-[#52525b] shrink-0" />}
                            {item.id === "workflow" && <Workflow className="sp-menu-item-icon h-4 w-4 text-[#52525b] shrink-0" />}
                            {item.id === "skills-tools" && <Blocks className="sp-menu-item-icon h-4 w-4 text-[#52525b] shrink-0 stroke-[1.75]" />}
                            <span className="whitespace-nowrap shrink-0">{item.label_zh}</span>
                            {item.aside_zh && (
                              <span className="ml-auto pl-4 text-xs text-[#a1a1aa] whitespace-nowrap shrink-0">
                                {item.aside_zh}
                              </span>
                            )}
                          </div>
                        ))}
                        <div
                          role="menuitem"
                          data-testid="attach-menu-item-media-upload"
                          tabIndex={0}
                          onMouseEnter={(e) =>
                            updateAttachHighlight(e.currentTarget, "media-upload", attachItems.length)
                          }
                          onFocus={(e) =>
                            updateAttachHighlight(e.currentTarget, "media-upload", attachItems.length)
                          }
                          onClick={() => handleAttachItemClick("media-upload")}
                          onKeyDown={(e) => {
                            if (e.key === "Enter" || e.key === " ") {
                              e.preventDefault();
                              handleAttachItemClick("media-upload");
                            }
                          }}
                          className="sp-fluid-menu-item sp-row-balanced flex items-center gap-2.5 px-2.5 rounded-[6px] text-xs text-[#18181b] dark:text-[#f4f4f5] cursor-pointer transition-colors outline-none"
                        >
                          <Paperclip className="sp-menu-item-icon h-4 w-4 text-[#52525b] shrink-0" />
                          <span className="whitespace-nowrap shrink-0">{t("agent.composer.attachMediaLabel")}</span>
                          <span className="ml-auto pl-4 text-xs text-[#a1a1aa] whitespace-nowrap shrink-0">
                            {t("agent.composer.attachMediaHint")}
                          </span>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              )}

              {/* 3. `SlidersHorizontal + Agent` Permission Picker */}
              <AgentPermissionMenu
                view={permissionView}
                status={permissionStatus}
                pendingApply={permissionPending}
                onSelect={onPermissionSelect}
                onSetGlobal={onPermissionSetGlobal}
                onClearSession={onPermissionClearSession}
                compact={compact}
              />
            </div>

            {/* Right: Pulse HUD (minimal circle or pro inline pills) + Model Picker + Single Circular Send Button */}
            <div className="flex flex-nowrap items-center justify-end gap-1.5 ml-auto min-w-0 flex-1">
              <AgentContextHud
                runs={contextRuns}
                events={contextEvents}
                draftText={value}
                maxContextTokens={activeModelMeta.contextWindow}
                modelLabel={activeModelMeta.modelLabel}
                compact={compact}
              />

              <AgentModelPicker
                channels={channels}
                sessionId={sessionId}
                ready={channelReadiness.ready}
                compact={compact || hudMode === "pro"}
                closeNonce={pickerCloseNonce}
                onGoSettings={onChannelGoSettings}
                onChange={(sel) => {
                  setSelectedModel(sel);
                  onModelChange?.(sel);
                }}
              />

              {/* Single Morphing Action Button: Send (↑) -> Dark Rounded-Square Stop (■) while running */}
              {(() => {
                const showMorphStop = Boolean(isRunning && onStop);
                return (
                  <button
                    type="button"
                    data-testid="agent-send-btn"
                    data-action-mode={showMorphStop ? "stop" : "send"}
                    onClick={showMorphStop ? onStop : handleTriggerSubmit}
                    disabled={showMorphStop ? false : !canSend}
                    aria-label={showMorphStop ? t("agent.composer.stopCurrentRun") : t("agent.composer.send")}
                    title={
                      showMorphStop
                        ? t("agent.composer.stopCurrentRun")
                        : sendTitleOverride ??
                          (!channelReadiness.ready
                            ? t("agent.composer.configureModelBeforeSend")
                            : `${t("agent.composer.send")} (Enter)`)
                    }
                    className={cn(
                      "cds-btn-squish ml-0.5 inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-[7px] transition-colors",
                      showMorphStop || canSend
                        ? "bg-sp-accent-fill text-sp-accent-ink hover:opacity-90 cursor-pointer"
                        : "bg-[#8e8e93] text-white cursor-not-allowed dark:bg-[#52525b] dark:text-[#d4d4d8]"
                    )}
                  >
                    {showMorphStop ? (
                      <Square className="h-[11px] w-[11px] fill-current" />
                    ) : (
                      <ArrowUp className="h-[14px] w-[14px] stroke-[2.2]" />
                    )}
                  </button>
                );
              })()}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
