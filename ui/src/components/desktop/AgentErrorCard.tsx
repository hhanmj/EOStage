/**
 * AgentErrorCard.tsx
 * Agent 对话流中的错误卡（EOStage 冷静单色极简风格，移除刺眼粉红底框与按钮折行）。
 * 可见文字只用中文解释与脱敏后的真实上游原因；错误码原文收在「展开详情」里（W5-2f R7，agnes P1）。
 * 支持 Markdown 解释文本选取复制与单行恢复操作栏。
 */

import React, { useState } from "react";
import { ChevronRight, Loader2, RotateCcw, Settings } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import type { StreamErrorItem } from "@/shell/api/agentStreamLogic";
import { errorCardHintZh, projectErrorCard } from "@/shell/api/approvalLogic";
import { AgentMarkdownRenderer } from "./agent/AgentMarkdownRenderer";

export interface AlternateModelOption {
  channelId: string;
  modelId: string;
  label: string;
}

export interface AgentErrorCardProps {
  item: StreamErrorItem;
  /** 该运行的渠道拒绝了「关闭思考」（AgentRunSummary.reasoning_off_refused） */
  reasoningOffRefused?: boolean;
  /** 可选：点击重试当前轮次 */
  onRetry?: (runId: string) => void | Promise<void>;
  /** 可选：跳转模型设置排查通道或连接 */
  onOpenModelSettings?: () => void;
  /** 当前会话仍在后台独立运行的下载任务数 */
  activeDownloadCount?: number;
  /** 可选：跳转查看任务与成果面板 */
  onOpenTaskPanel?: () => void;
  /** 可选：基于已保存上下文与工具成果接着继续 */
  onResumeTurn?: (runId: string) => void | Promise<void>;
  /** 可选：其他已配置的备用模型列表 */
  alternateModels?: readonly AlternateModelOption[];
  /** 可选：切换到备用模型并接着继续 */
  onSwitchModelAndResume?: (
    channelId: string,
    modelId: string,
    runId: string
  ) => void | Promise<void>;
}

const GENERIC_UNKNOWN_WORDING = "模型服务返回未知错误";

function sanitizeVisibleCause(raw: string | null | undefined): string | null {
  if (typeof raw !== "string") return null;
  const trimmed = raw.trim();
  if (!trimmed || trimmed === GENERIC_UNKNOWN_WORDING) return null;
  return trimmed;
}

export function AgentErrorCard({
  item,
  reasoningOffRefused = false,
  onRetry,
  onOpenModelSettings,
  activeDownloadCount = 0,
  onOpenTaskPanel,
  onResumeTurn: _onResumeTurn,
  alternateModels: _alternateModels = [],
  onSwitchModelAndResume: _onSwitchModelAndResume,
}: AgentErrorCardProps) {
  const { t } = useI18n();
  const [detailOpen, setDetailOpen] = useState(false);
  const proj = projectErrorCard(item);
  const hint = errorCardHintZh(item.code, reasoningOffRefused);
  const isCancelled =
    item.code === "cancelled" || item.status === "cancelled" || item.status === "interrupted";
  const isForbidden = item.code === "plan_refused_forbidden";

  // 优先展示后端透传的脱敏真实上游诊断（例如 HTTP 状态码 / 提供商报错摘要），避免退化为笼统的「未知错误」
  const explicitMessageZh = sanitizeVisibleCause(item.message_zh);
  const primaryWording =
    !isCancelled &&
    !isForbidden &&
    explicitMessageZh &&
    (proj.wording === GENERIC_UNKNOWN_WORDING || !proj.wording)
      ? explicitMessageZh
      : proj.wording;

  const secondaryUpstreamCause =
    !isCancelled &&
    !isForbidden &&
    explicitMessageZh &&
    explicitMessageZh !== primaryWording
      ? explicitMessageZh
      : null;

  const isAuthOrConfigError = (() => {
    const code = (item.code || "").toLowerCase();
    const raw = `${primaryWording} ${explicitMessageZh || ""} ${code}`.toLowerCase();
    return (
      code === "model_authentication" ||
      code === "model_config" ||
      code === "channel_not_ready" ||
      raw.includes("401") ||
      raw.includes("403") ||
      raw.includes("unauthorized") ||
      raw.includes("api key") ||
      raw.includes("api_key") ||
      raw.includes("认证失败") ||
      raw.includes("令牌") ||
      raw.includes("未配置") ||
      raw.includes("模型渠道")
    );
  })();

  return (
    <div
      data-testid="agent-error-card"
      className="rounded-[12px] border border-sp-line bg-sp-surface dark:bg-sp-card px-4 py-3.5 shadow-card flex items-start gap-3 w-full select-text"
    >
      {/* Subtle monochrome-friendly muted error dot indicator */}
      <span
        aria-hidden="true"
        data-testid="agent-error-indicator"
        className="mt-1.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border border-sp-line bg-sp-surface-muted"
      >
        <span
          className={cn(
            "h-1.5 w-1.5 rounded-full",
            isCancelled ? "bg-sp-text-muted" : "bg-rose-500/80 dark:bg-rose-400/80"
          )}
        />
      </span>

      <div className="flex-1 min-w-0 flex flex-col gap-2">
        <div className="flex flex-col gap-1 min-w-0">
          <div className="text-[13px] font-medium leading-[1.55] text-sp-text select-text">
            <AgentMarkdownRenderer
              compact
              content={primaryWording}
              className="text-[13px] font-medium leading-[1.55] text-sp-text"
            />
          </div>
          {secondaryUpstreamCause && (
            <div
              data-testid="agent-error-upstream-cause"
              className="text-xs leading-relaxed text-sp-text-secondary break-words select-text"
            >
              {secondaryUpstreamCause}
            </div>
          )}
          {proj.modeLine && (
            <div className="text-xs text-sp-text-muted select-text">{proj.modeLine}</div>
          )}
          {hint && (
            <AgentMarkdownRenderer
              compact
              content={hint}
              className="text-xs text-sp-text-secondary"
            />
          )}
        </div>

        {activeDownloadCount > 0 && !isCancelled && (
          <div
            data-testid="agent-error-active-downloads-banner"
            className="flex flex-wrap items-center justify-between gap-2 rounded-[8px] border border-sp-line/80 bg-sp-surface-muted px-3 py-1.5 text-xs text-sp-text-secondary"
          >
            <span className="inline-flex items-center gap-1.5 min-w-0">
              <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-sp-text-muted" aria-hidden="true" />
              <span className="min-w-0">
                当前有 {activeDownloadCount} 个后台下载任务正在独立运行，不受本次模型中断影响
              </span>
            </span>
            {onOpenTaskPanel && (
              <button
                type="button"
                data-testid="agent-error-open-task-panel-btn"
                onClick={onOpenTaskPanel}
                className="shrink-0 whitespace-nowrap font-medium text-sp-text hover:underline cursor-pointer"
              >
                查看任务进度
              </button>
            )}
          </div>
        )}

        <div
          data-testid="agent-error-actions"
          className="flex flex-wrap items-center gap-2 pt-0.5"
        >
          {onRetry && !isCancelled && (
            <button
              type="button"
              data-testid="btn-error-retry"
              onClick={() => void onRetry(item.run_id)}
              className="cds-btn-squish inline-flex h-7 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-[7px] bg-sp-accent-fill px-2.5 text-xs font-medium text-sp-accent-ink hover:opacity-90 cursor-pointer transition-opacity duration-fast ease-sp select-none"
            >
              <RotateCcw className="h-3 w-3 shrink-0" />
              <span>{t("agent.actions.retryTurn")}</span>
            </button>
          )}
          {onOpenModelSettings && isAuthOrConfigError && (
            <button
              type="button"
              data-testid="btn-error-open-model-settings"
              onClick={onOpenModelSettings}
              className="cds-btn-squish inline-flex h-7 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-[7px] border border-sp-line bg-sp-surface dark:bg-sp-surface-muted px-2.5 text-xs font-medium text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text cursor-pointer transition-colors duration-fast ease-sp select-none"
            >
              <Settings className="h-3 w-3 shrink-0" />
              <span>前往模型设置</span>
            </button>
          )}
          {proj.detailCode && (
            <button
              type="button"
              onClick={() => setDetailOpen((v) => !v)}
              aria-expanded={detailOpen}
              aria-label={
                detailOpen
                  ? t("agent.conclusion.collapseDetails")
                  : t("agent.conclusion.expandDetails")
              }
              className="inline-flex h-7 shrink-0 items-center gap-1 whitespace-nowrap rounded-[7px] px-2 text-xs text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer transition-colors duration-fast ease-sp select-none"
            >
              <ChevronRight
                className={cn(
                  "h-3 w-3 shrink-0 transition-transform duration-fast ease-sp",
                  detailOpen && "rotate-90"
                )}
              />
              <span>
                {detailOpen
                  ? t("agent.conclusion.collapseDetails")
                  : t("agent.conclusion.expandDetails")}
              </span>
            </button>
          )}
        </div>

        {proj.detailCode && detailOpen && (
          <div
            data-testid="agent-error-detail-box"
            className="rounded-[8px] border border-sp-line/70 bg-sp-surface-muted px-2.5 py-1.5 text-xs font-mono text-sp-text-muted break-all select-text"
          >
            错误代码: {proj.detailCode}
          </div>
        )}
      </div>
    </div>
  );
}
