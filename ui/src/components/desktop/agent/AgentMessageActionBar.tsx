/**
 * AgentMessageActionBar.tsx
 * Icon-only per-turn message action bar for user and assistant messages:
 * - User turn (`[data-testid="agent-user-turn-actions"]`): Icon-only Copy, Edit & Resend (with inline editor), Revert Turn.
 * - Assistant turn (`[data-testid="agent-assistant-turn-actions"]`): Icon-only Copy, Retry/Stop, and EOStage-style hover Usage Icon on the same row.
 */

import React, { useCallback, useEffect, useRef, useState } from "react";
import { BarChart2, Check, Copy, Pencil, RotateCcw, Send, Square, Undo2, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import { cleanCopyableMarkdown } from "@/shell/api/agentThinkSplitLogic";
import { turnUsageText } from "@/shell/api/agentUsage";
import type { RunUsageDetailState } from "../agentRunUsageHooks";

export async function copyTextToClipboard(text: string): Promise<boolean> {
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Fallback for non-secure or headless test environments
    }
  }
  if (typeof document !== "undefined") {
    try {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.setAttribute("readonly", "true");
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      const ok = typeof document.execCommand === "function" ? document.execCommand("copy") : true;
      document.body.removeChild(ta);
      return ok;
    } catch {
      return false;
    }
  }
  return false;
}

export interface AgentMessageActionBarProps {
  role: "user" | "assistant";
  content: string;
  turnIndex?: number;
  runId?: string;
  isRunning?: boolean;
  onEditResend?: (newText: string, turnIndex?: number) => void | Promise<void>;
  onRevertTurn?: (turnIndex?: number) => void | Promise<void>;
  isEditing?: boolean;
  onEditingChange?: (editing: boolean) => void;
  onRetryTurn?: (runId?: string, turnIndex?: number) => void | Promise<void>;
  onStopRun?: (runId?: string) => void | Promise<void>;
  onCopy?: (copiedCleanText: string) => void;
  usageTotal?: unknown;
  usageDetail?: RunUsageDetailState;
  onRequestUsageDetail?: () => void;
  className?: string;
}

const ICON_BTN_CLASS =
  "inline-flex h-7 w-7 items-center justify-center rounded-md text-sp-text-muted hover:bg-sp-hover hover:text-sp-text disabled:opacity-45 disabled:cursor-not-allowed transition-colors duration-fast ease-sp cursor-pointer";

export function AgentMessageActionBar({
  role,
  content,
  turnIndex,
  runId,
  isRunning = false,
  onEditResend,
  onRevertTurn,
  isEditing: controlledEditing,
  onEditingChange,
  onRetryTurn,
  onStopRun,
  onCopy,
  usageTotal,
  usageDetail,
  onRequestUsageDetail,
  className,
}: AgentMessageActionBarProps) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  const [internalEditing, setInternalEditing] = useState(false);
  const [draftText, setDraftText] = useState(content);
  const [usageHovered, setUsageHovered] = useState(false);
  const [usagePinned, setUsagePinned] = useState(false);
  const usageWrapRef = useRef<HTMLDivElement | null>(null);
  const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const isEditing = controlledEditing !== undefined ? controlledEditing : internalEditing;
  const usageSummary = role === "assistant" ? turnUsageText(usageTotal) : null;
  const cachedHitLabel = (() => {
    if (!usageTotal || typeof usageTotal !== "object" || Array.isArray(usageTotal)) return null;
    const u = usageTotal as Record<string, unknown>;
    const cached =
      typeof u.cached === "number" && Number.isSafeInteger(u.cached) && u.cached > 0
        ? u.cached
        : typeof u.cached_tokens === "number" && Number.isSafeInteger(u.cached_tokens) && u.cached_tokens > 0
          ? u.cached_tokens
          : 0;
    if (cached <= 0) return null;
    const formatted = String(cached).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
    return `命中缓存 ${formatted}`;
  })();
  const showUsagePopover = usageSummary !== null && (usageHovered || usagePinned);
  void usageDetail;

  const setEditing = useCallback(
    (next: boolean) => {
      if (onEditingChange) {
        onEditingChange(next);
      } else {
        setInternalEditing(next);
      }
    },
    [onEditingChange]
  );

  useEffect(() => {
    setDraftText(content);
  }, [content]);

  useEffect(() => {
    return () => {
      if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
    };
  }, []);

  useEffect(() => {
    if (!usagePinned) return;
    const handleOutside = (e: MouseEvent) => {
      if (usageWrapRef.current && !usageWrapRef.current.contains(e.target as Node)) {
        setUsagePinned(false);
      }
    };
    const handleEsc = (e: KeyboardEvent) => {
      if (e.key === "Escape") setUsagePinned(false);
    };
    document.addEventListener("mousedown", handleOutside);
    document.addEventListener("keydown", handleEsc);
    return () => {
      document.removeEventListener("mousedown", handleOutside);
      document.removeEventListener("keydown", handleEsc);
    };
  }, [usagePinned]);

  const handleCopy = useCallback(async () => {
    const cleanText = role === "assistant" ? cleanCopyableMarkdown(content) : content.trim();
    await copyTextToClipboard(cleanText);
    onCopy?.(cleanText);
    setCopied(true);
    if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
    copyTimerRef.current = setTimeout(() => setCopied(false), 1800);
  }, [content, onCopy, role]);

  const handleOpenEdit = useCallback(() => {
    setDraftText(content);
    setEditing(true);
  }, [content, setEditing]);

  const handleCancelEdit = useCallback(() => {
    setDraftText(content);
    setEditing(false);
  }, [content, setEditing]);

  const handleConfirmEdit = useCallback(async () => {
    const trimmed = draftText.trim();
    if (!trimmed || !onEditResend) return;
    setEditing(false);
    await onEditResend(trimmed, turnIndex);
  }, [draftText, onEditResend, setEditing, turnIndex]);

  if (role === "user") {
    return (
      <div className={cn("flex flex-col items-end gap-2 w-full", className)}>
        {isEditing && (
          <div
            data-testid="agent-inline-edit-box"
            className="w-full rounded-xl border border-sp-accent bg-sp-card p-3 shadow-card flex flex-col gap-2"
          >
            <textarea
              data-testid="agent-edit-textarea"
              value={draftText}
              onChange={(e) => setDraftText(e.target.value)}
              placeholder={t("agent.actions.editPlaceholder")}
              rows={3}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  handleCancelEdit();
                } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  void handleConfirmEdit();
                }
              }}
              className="w-full resize-y rounded-lg border border-sp-line bg-sp-canvas px-3 py-2 text-sm text-sp-text placeholder:text-sp-text-muted focus:border-sp-accent focus:outline-none select-text"
            />
            <div className="flex items-center justify-end gap-2">
              <button
                type="button"
                data-testid="btn-cancel-edit-msg"
                onClick={handleCancelEdit}
                className="inline-flex items-center gap-1 rounded-lg border border-sp-line bg-sp-surface-muted px-2.5 py-1 text-xs font-medium text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp cursor-pointer"
              >
                <X className="h-3.5 w-3.5" />
                <span>{t("agent.actions.cancelEdit")}</span>
              </button>
              <button
                type="button"
                data-testid="btn-confirm-edit-resend"
                disabled={draftText.trim().length === 0 || isRunning}
                onClick={() => void handleConfirmEdit()}
                className="inline-flex items-center gap-1 rounded-lg bg-sp-accent px-3 py-1 text-xs font-medium text-sp-canvas hover:bg-sp-accent-strong disabled:opacity-50 disabled:cursor-not-allowed transition-colors duration-fast ease-sp cursor-pointer"
              >
                <Send className="h-3.5 w-3.5" />
                <span>{t("agent.actions.confirmEditResend")}</span>
              </button>
            </div>
          </div>
        )}

        <div
          data-testid="agent-user-turn-actions"
          className="flex items-center justify-end gap-0.5 opacity-80 group-hover:opacity-100 focus-within:opacity-100 transition-opacity duration-fast select-none"
        >
          <button
            type="button"
            data-testid="btn-copy-user-msg"
            data-copied={copied ? "true" : "false"}
            onClick={() => void handleCopy()}
            title={copied ? t("agent.actions.copied") : t("agent.actions.copyMessage")}
            aria-label={copied ? t("agent.actions.copied") : t("agent.actions.copyMessage")}
            className={ICON_BTN_CLASS}
          >
            {copied ? (
              <Check className="h-3.5 w-3.5 text-sp-status-success" />
            ) : (
              <Copy className="h-3.5 w-3.5" />
            )}
          </button>

          {onEditResend && (
            <button
              type="button"
              data-testid="btn-edit-resend-msg"
              disabled={isRunning}
              onClick={handleOpenEdit}
              title={t("agent.actions.editResend")}
              aria-label={t("agent.actions.editResend")}
              className={ICON_BTN_CLASS}
            >
              <Pencil className="h-3.5 w-3.5" />
            </button>
          )}

          {onRevertTurn && (
            <button
              type="button"
              data-testid="btn-revert-turn"
              disabled={isRunning}
              onClick={() => void onRevertTurn(turnIndex)}
              title={t("agent.actions.revertTurn")}
              aria-label={t("agent.actions.revertTurn")}
              className={ICON_BTN_CLASS}
            >
              <Undo2 className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div
      data-testid="agent-assistant-turn-actions"
      className={cn(
        "flex items-center justify-start gap-0.5 opacity-80 group-hover:opacity-100 focus-within:opacity-100 transition-opacity duration-fast select-none",
        className
      )}
    >
      <button
        type="button"
        data-testid="btn-copy-assistant-msg"
        data-copied={copied ? "true" : "false"}
        onClick={() => void handleCopy()}
        title={copied ? t("agent.actions.copied") : t("agent.actions.copyMessage")}
        aria-label={copied ? t("agent.actions.copied") : t("agent.actions.copyMessage")}
        className={ICON_BTN_CLASS}
      >
        {copied ? (
          <Check className="h-3.5 w-3.5 text-sp-status-success" />
        ) : (
          <Copy className="h-3.5 w-3.5" />
        )}
      </button>

      {isRunning
        ? onStopRun && (
            <button
              type="button"
              data-testid="btn-stop-run"
              onClick={() => void onStopRun(runId)}
              title={t("agent.actions.stopRun")}
              aria-label={t("agent.actions.stopRun")}
              className="inline-flex h-7 w-7 items-center justify-center rounded-md text-sp-danger-text bg-sp-danger-bg hover:opacity-90 transition-opacity duration-fast ease-sp cursor-pointer"
            >
              <Square className="h-3 w-3 fill-current" />
            </button>
          )
        : onRetryTurn && (
            <button
              type="button"
              data-testid="btn-retry-turn"
              onClick={() => void onRetryTurn(runId, turnIndex)}
              title={t("agent.actions.retryTurn")}
              aria-label={t("agent.actions.retryTurn")}
              className={ICON_BTN_CLASS}
            >
              <RotateCcw className="h-3.5 w-3.5" />
            </button>
          )}

      {usageSummary !== null && (
        <div
          ref={usageWrapRef}
          className="relative inline-flex"
          onMouseEnter={() => {
            onRequestUsageDetail?.();
            setUsageHovered(true);
          }}
          onMouseLeave={() => setUsageHovered(false)}
        >
          <button
            type="button"
            data-testid="btn-turn-usage-icon"
            aria-label={t("agent.actions.turnUsage")}
            aria-expanded={showUsagePopover}
            onFocus={() => {
              onRequestUsageDetail?.();
              setUsageHovered(true);
            }}
            onBlur={() => setUsageHovered(false)}
            onClick={() => {
              onRequestUsageDetail?.();
              setUsagePinned((prev) => !prev);
            }}
            className={cn(
              ICON_BTN_CLASS,
              showUsagePopover && "bg-sp-hover text-sp-text"
            )}
          >
            <BarChart2 className="h-3.5 w-3.5" />
          </button>

          {showUsagePopover && (
            <div
              role="tooltip"
              data-testid="turn-usage-popover"
              className="absolute -bottom-1.5 left-[calc(100%+8px)] z-50 w-max min-w-[220px] max-w-[380px] rounded-lg border border-sp-line bg-sp-surface px-3 py-1.5 text-xs text-sp-text shadow-overlay dark:bg-sp-card select-text"
            >
              <div className="flex items-center justify-between gap-3 leading-5 whitespace-nowrap">
                <span className="font-semibold text-sp-text">
                  {t("agent.actions.turnUsageTitle")}
                </span>
                <span className="text-sp-text-secondary tabular-nums">
                  {usageSummary.replace(/^本轮用量\s*/, "")}
                  {cachedHitLabel ? ` · ${cachedHitLabel}` : ""}
                </span>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
