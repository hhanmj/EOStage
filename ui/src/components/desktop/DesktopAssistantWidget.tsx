import React, { useState, useEffect, useRef } from "react";
import {
  MessageSquare,
  Maximize2,
  X,
  Send,
  AlertTriangle,
  ExternalLink,
  Command,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import type { Bbox } from "@/lib/bridge";
import type { ModelChannelReadiness } from "@/shell/api/agentTypes";
import { isAgentWired, getFixtureAssistantGreetingSync } from "@/shell/api";
import { StatusStateCard } from "@/shell/components/StatusStateCard";

export interface WorkbenchContextSummary {
  aoiName?: string;
  aoiBbox?: Bbox | null;
  startDate?: string;
  endDate?: string;
  resourceId?: string;
}

export interface DesktopAssistantWidgetProps {
  channelReadiness: ModelChannelReadiness;
  contextSummary?: WorkbenchContextSummary;
  onExpandToWorkbench: (context?: WorkbenchContextSummary) => void;
  onOpenSettings?: (category?: "network" | "models" | "accounts") => void;
  initialMode?: "collapsed" | "expanded" | "cmdk";
}

export function DesktopAssistantWidget({
  channelReadiness,
  contextSummary,
  onExpandToWorkbench,
  onOpenSettings,
  initialMode = "collapsed",
}: DesktopAssistantWidgetProps) {
  const { t } = useI18n();
  const [panelOpen, setPanelOpen] = useState<boolean>(initialMode === "expanded");
  const [cmdkOpen, setCmdkOpen] = useState<boolean>(initialMode === "cmdk");
  const [inputText, setInputText] = useState("");
  const [cmdkInput, setCmdkInput] = useState("");
  const cmdkInputRef = useRef<HTMLInputElement>(null);

  // 监听 initialMode 变化（配合 URL hash 直达）
  useEffect(() => {
    if (initialMode === "expanded") {
      setPanelOpen(true);
      setCmdkOpen(false);
    } else if (initialMode === "cmdk") {
      setCmdkOpen(true);
      setPanelOpen(false);
    } else if (initialMode === "collapsed") {
      setPanelOpen(false);
      setCmdkOpen(false);
    }
  }, [initialMode]);

  // 全局 Ctrl+K / Cmd+K 快捷键监听
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setCmdkOpen((prev) => !prev);
      } else if (e.key === "Escape" && cmdkOpen) {
        setCmdkOpen(false);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [cmdkOpen]);

  // 自动聚焦 Cmd+K 输入框
  useEffect(() => {
    if (cmdkOpen) {
      setTimeout(() => cmdkInputRef.current?.focus(), 50);
    }
  }, [cmdkOpen]);

  const isWired = isAgentWired();
  const greeting = isWired ? getFixtureAssistantGreetingSync() : null;

  const handleSend = () => {
    if (!inputText.trim() || channelReadiness.disabled) return;
    setInputText("");
    // 携带上下文直接放大进入工作台展示完整响应
    onExpandToWorkbench(contextSummary);
  };

  const handleCmdkSend = () => {
    if (!cmdkInput.trim() || channelReadiness.disabled) return;
    setCmdkOpen(false);
    setCmdkInput("");
    onExpandToWorkbench(contextSummary);
  };

  return (
    <>
      {/* 1. Ctrl+K 居中顶部命令条 */}
      {cmdkOpen && (
        <div
          role="dialog"
          aria-label={t("agent.assistant.commandBar")}
          className="fixed inset-0 z-[1200] flex items-start justify-center pt-24 bg-black/45 dark:bg-black/65 backdrop-blur-[2px]"
          onClick={() => {
            if (!cmdkInput.trim()) setCmdkOpen(false);
          }}
        >
          <div
            className="w-[520px] rounded-card border border-sp-line bg-sp-surface p-3 shadow-modal select-none space-y-3"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center gap-2.5 border-b border-sp-line pb-2.5">
              <MessageSquare className="h-4 w-4 text-sp-text-secondary shrink-0" />
              <input
                ref={cmdkInputRef}
                type="text"
                value={cmdkInput}
                disabled={channelReadiness.disabled}
                onChange={(e) => setCmdkInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    handleCmdkSend();
                  }
                }}
                placeholder={
                  channelReadiness.disabled
                    ? t("agent.assistant.commandDisabled")
                    : t("agent.assistant.commandPlaceholder")
                }
                className="flex-1 bg-transparent text-xs text-sp-text outline-none placeholder:text-sp-text-disabled"
              />
              <button
                type="button"
                onClick={() => setCmdkOpen(false)}
                title={t("agent.assistant.closeHint")}
                aria-label={t("agent.assistant.close")}
                className="flex h-6 w-6 items-center justify-center rounded-sm text-sp-text-muted hover:bg-sp-hover cursor-pointer"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>

            {/* 渠道未配置提示 */}
            {channelReadiness.disabled && (
              <div className="flex items-center justify-between text-xs text-sp-text-secondary bg-sp-surface-muted p-2 rounded-md border border-sp-line">
                <div className="flex items-center gap-1.5">
                  <AlertTriangle className="h-3.5 w-3.5 text-sp-warning-text" />
                  <span>{channelReadiness.reason_zh}</span>
                </div>
                <button
                  type="button"
                  onClick={() => {
                    setCmdkOpen(false);
                    onOpenSettings?.("models");
                  }}
                  className="font-semibold text-sp-accent-strong hover:underline flex items-center gap-1 cursor-pointer"
                >
                  <span>{t("agent.settings")}</span>
                  <ExternalLink className="h-3 w-3" />
                </button>
              </div>
            )}

            <div className="flex items-center justify-between text-xs text-sp-text-muted">
              <span>{t("agent.assistant.currentContext", { name: contextSummary?.aoiName || t("agent.assistant.defaultArea") })}</span>
              <span className="flex items-center gap-1">
                <kbd className="rounded-sm bg-sp-surface-muted px-1.5 py-0.5 border border-sp-line font-mono text-xs">
                  ESC
                </kbd>
                <span>{t("agent.assistant.close")}</span>
              </span>
            </div>
          </div>
        </div>
      )}

      {/* 2. 右下角收起/展开的助手面板与胶囊按钮 */}
      <div className="fixed bottom-4 right-4 z-[1050] select-none flex flex-col items-end">
        {panelOpen ? (
          /* 展开状态：小型对话面板 (同 SettingsModal 平面卡片语言) */
          <div className="w-[360px] h-[460px] rounded-card border border-sp-line bg-sp-surface shadow-modal flex flex-col overflow-hidden mb-2 duration-base ease-sp">
            {/* 面板顶栏 */}
            <div className="flex h-11 shrink-0 items-center justify-between border-b border-sp-line px-3 bg-sp-surface-muted">
              <div className="flex items-center gap-2">
                <MessageSquare className="h-4 w-4 text-sp-text-secondary" />
                <span className="text-xs font-bold text-sp-text">{t("agent.assistant.title")}</span>
              </div>
              <div className="flex items-center gap-1">
                {/* 放大进入 Agent 工作台按钮 */}
                <button
                  type="button"
                  onClick={() => onExpandToWorkbench(contextSummary)}
                  title={t("agent.assistant.expand")}
                  aria-label={t("agent.assistant.expand")}
                  className="flex h-7 w-7 items-center justify-center rounded-sm text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp cursor-pointer"
                >
                  <Maximize2 className="h-3.5 w-3.5" />
                </button>
                {/* 收起面板 */}
                <button
                  type="button"
                  onClick={() => setPanelOpen(false)}
                  title={t("agent.assistant.collapse")}
                  aria-label={t("agent.assistant.collapse")}
                  className="flex h-7 w-7 items-center justify-center rounded-sm text-sp-text-secondary hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>

            {/* 面板中间消息流区域 */}
            <div className="flex-1 overflow-y-auto p-3 space-y-3">
              {!isWired ? (
                /* 生产环境诚实未接入态：共享五态组件 */
                <StatusStateCard
                  variant="not_connected"
                  title={t("agent.assistant.notConnected")}
                  description={t("agent.assistant.notConnectedDescription")}
                  action={
                    onOpenSettings
                      ? {
                          label: t("agent.assistant.modelSettings"),
                          onClick: () => onOpenSettings("models"),
                          variant: "link",
                        }
                      : undefined
                  }
                  compact
                  className="h-full"
                />
              ) : (
                /* 开发/fixture 演示环境 */
                <>
                  <div className="rounded-md bg-sp-surface-muted p-2.5 border border-sp-line text-xs space-y-1">
                    <div className="font-semibold text-sp-text">{greeting?.boundTitle}</div>
                    <div className="text-xs text-sp-text-secondary">
                      {t("agent.assistant.currentArea", { name: contextSummary?.aoiName || greeting?.defaultAoi || "" })}
                    </div>
                    <div className="text-xs text-sp-text-muted">
                      {t("agent.assistant.dataSource", { name: contextSummary?.resourceId || greeting?.defaultResource || "" })}
                    </div>
                  </div>

                  <div className="rounded-card bg-sp-canvas p-3 border border-sp-line text-xs text-sp-text leading-relaxed">
                    {greeting?.greetingText}
                  </div>
                </>
              )}
            </div>

            {/* 面板底部输入框 */}
            <div className="p-2.5 border-t border-sp-line bg-sp-surface">
              {channelReadiness.disabled ? (
                <div className="rounded-md bg-sp-surface-muted p-2 border border-sp-line text-xs space-y-1">
                  <div className="flex items-center justify-between text-sp-text-secondary">
                    <span className="text-xs">{channelReadiness.reason_zh}</span>
                    <button
                      type="button"
                      onClick={() => onOpenSettings?.("models")}
                      className="text-xs font-semibold text-sp-accent-strong hover:underline flex items-center gap-0.5 cursor-pointer"
                    >
                      <span>{t("agent.settings")}</span>
                      <ExternalLink className="h-2.5 w-2.5" />
                    </button>
                  </div>
                  <input
                    type="text"
                    disabled
                    placeholder={t("agent.assistant.inputDisabled")}
                    className="h-8 w-full rounded-sm border border-sp-line bg-sp-surface px-2 text-xs text-sp-text-disabled cursor-not-allowed outline-none"
                  />
                </div>
              ) : (
                <div className="relative flex items-center">
                  <input
                    type="text"
                    value={inputText}
                    onChange={(e) => setInputText(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && !e.shiftKey) {
                        e.preventDefault();
                        handleSend();
                      }
                    }}
                    placeholder={t("agent.assistant.inputPlaceholder")}
                    className="h-8 w-full rounded-sm border border-sp-line bg-sp-surface px-2.5 pr-8 text-xs text-sp-text outline-none focus:border-sp-accent"
                  />
                  <button
                    type="button"
                    onClick={handleSend}
                    disabled={!inputText.trim()}
                    title={t("agent.assistant.sendMessage")}
                    aria-label={t("agent.assistant.sendMessage")}
                    className={cn(
                      "absolute right-1 flex h-6 w-6 items-center justify-center rounded-sm transition-colors duration-fast ease-sp cursor-pointer",
                      inputText.trim()
                        ? "bg-sp-accent-fill text-white hover:opacity-90"
                        : "bg-sp-surface-muted text-sp-text-disabled cursor-not-allowed"
                    )}
                  >
                    <Send className="h-3 w-3" />
                  </button>
                </div>
              )}
            </div>
          </div>
        ) : (
          /* 收起状态：小圆钮 (圆形，小阴影，居中放置单色对话图标) */
          <button
            type="button"
            onClick={() => setPanelOpen(true)}
            title={t("agent.assistant.shortcutTitle")}
            aria-label={t("agent.assistant.title")}
            className="flex h-11 w-11 items-center justify-center rounded-full bg-sp-surface text-sp-text-secondary border border-sp-line shadow-overlay hover:bg-sp-hover hover:text-sp-text transition-all duration-fast ease-sp cursor-pointer"
          >
            <MessageSquare className="h-5 w-5" />
          </button>
        )}
      </div>
    </>
  );
}
