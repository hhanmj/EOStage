/**
 * AgentComposerEyebrow.tsx
 * 输入框上方同心圆眉栏（替换旧版重复消息时间线的「运行记录」条）：
 * 1. 当存在排队待发消息 (`queuedMessages.length > 0`) 或正在转向 (`isSteering`) 时，自动切换为「待发消息队列条」：
 *    支持一键「立即引导」插队转向、取回输入框编辑（纯图标）、移除（纯图标）；
 * 2. 平时默认收拢展示「当前研究区 · 已检出影像数 · 已勾选景数 · 本会话下载任务进度」速览，点击直达右侧分屏地图或任务面板。
 */

import React from "react";
import {
  CheckSquare,
  Download,
  Layers,
  Loader2,
  MapPin,
  Pencil,
  X,
  Zap,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";

export interface QueuedComposerMessage {
  id: string;
  text: string;
}

export interface AgentComposerEyebrowProps {
  hasBoundAoi: boolean;
  aoiLabel?: string | null;
  onOpenMap: () => void;
  onSelectAdminRegion?: () => void;
  onClearAoi?: () => void;
  sourceLabel?: string | null;
  scenesCount: number;
  checkedCount: number;
  onOpenResults: () => void;
  activeTaskCount: number;
  completedTaskCount: number;
  onOpenTasks: () => void;
  queuedMessages: readonly QueuedComposerMessage[];
  isSteering?: boolean;
  onSteerQueued: (id: string) => void;
  onEditQueued: (id: string) => void;
  onRemoveQueued: (id: string) => void;
}

export function AgentComposerEyebrow({
  hasBoundAoi,
  aoiLabel,
  onOpenMap,
  onSelectAdminRegion,
  onClearAoi,
  sourceLabel,
  scenesCount,
  checkedCount,
  onOpenResults,
  activeTaskCount,
  completedTaskCount,
  onOpenTasks,
  queuedMessages,
  isSteering = false,
  onSteerQueued,
  onEditQueued,
  onRemoveQueued,
}: AgentComposerEyebrowProps) {
  const { t } = useI18n();

  if (isSteering || queuedMessages.length > 0) {
    return (
      <div
        data-testid="agent-queued-messages-eyebrow"
        className="flex flex-col gap-1 py-0.5 select-none"
      >
        {isSteering && (
          <div className="flex items-center gap-1.5 text-xs text-sp-accent font-medium">
            <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
            <span className="truncate">{t("agent.queue.steeringActive")}</span>
          </div>
        )}

        {queuedMessages.length > 0 && (
          <div className="flex flex-col gap-1">
            <div className="flex items-center justify-between gap-2 text-xs text-sp-text-muted">
              <span className="font-medium text-sp-text-secondary">
                {t("agent.queue.queuedCount", { count: queuedMessages.length })}
              </span>
            </div>

            {queuedMessages.map((msg) => (
              <div
                key={msg.id}
                data-testid="agent-queued-message-row"
                className="flex items-center justify-between gap-2 rounded-lg border border-sp-line bg-sp-card px-2.5 py-1 text-xs"
              >
                <span className="min-w-0 flex-1 truncate text-sp-text select-text">
                  {msg.text}
                </span>

                <div className="flex items-center gap-1 shrink-0">
                  <button
                    type="button"
                    data-testid="btn-steer-queued-msg"
                    disabled={isSteering}
                    onClick={() => onSteerQueued(msg.id)}
                    title={t("agent.queue.steerNow")}
                    className="inline-flex items-center gap-1 rounded-md bg-sp-hover px-2 py-0.5 text-xs font-medium text-sp-accent hover:bg-sp-selected disabled:opacity-50 cursor-pointer transition-colors"
                  >
                    <Zap className="h-3 w-3 shrink-0" />
                    <span>{t("agent.queue.steerNow")}</span>
                  </button>

                  <button
                    type="button"
                    data-testid="btn-edit-queued-msg"
                    onClick={() => onEditQueued(msg.id)}
                    title={t("agent.queue.editQueued")}
                    aria-label={t("agent.queue.editQueued")}
                    className="inline-flex h-6 w-6 items-center justify-center rounded-md text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer transition-colors"
                  >
                    <Pencil className="h-3 w-3" />
                  </button>

                  <button
                    type="button"
                    data-testid="btn-remove-queued-msg"
                    onClick={() => onRemoveQueued(msg.id)}
                    title={t("agent.queue.removeQueued")}
                    aria-label={t("agent.queue.removeQueued")}
                    className="inline-flex h-6 w-6 items-center justify-center rounded-md text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer transition-colors"
                  >
                    <X className="h-3 w-3" />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  }

  const totalTasks = activeTaskCount + completedTaskCount;

  return (
    <div
      data-testid="agent-live-context-bar"
      className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-xs select-none"
    >
      {hasBoundAoi ? (
        <div
          data-testid="agent-context-aoi-pill"
          className="inline-flex items-center gap-1 min-w-0 text-sp-text"
        >
          <button
            type="button"
            onClick={onOpenMap}
            className="inline-flex items-center gap-1 min-w-0 text-left hover:text-sp-accent cursor-pointer transition-colors"
            title="在右侧地图面板查看当前研究区"
          >
            <MapPin className="h-3.5 w-3.5 text-sp-accent shrink-0" />
            <span className="max-w-[200px] truncate font-medium">
              研究区：{aoiLabel || "已框选范围"}
            </span>
          </button>
          {onClearAoi && (
            <button
              type="button"
              data-testid="btn-clear-context-aoi"
              onClick={onClearAoi}
              aria-label="清除研究区"
              title="清除研究区"
              className="rounded-full p-0.5 text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer transition-colors"
            >
              <X className="h-3 w-3" />
            </button>
          )}
        </div>
      ) : (
        <button
          type="button"
          data-testid="agent-context-aoi-pill"
          onClick={() => (onSelectAdminRegion ? onSelectAdminRegion() : onOpenMap())}
          className="inline-flex items-center gap-1 text-sp-text-secondary hover:text-sp-text cursor-pointer transition-colors"
        >
          <MapPin className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
          <span>全球视图 · 选择研究区</span>
        </button>
      )}

      {scenesCount > 0 && (
        <>
          <span className="text-sp-line">·</span>
          <button
            type="button"
            data-testid="agent-context-batch-pill"
            onClick={onOpenMap}
            className="inline-flex items-center gap-1 text-sp-text hover:text-sp-accent cursor-pointer transition-colors"
          >
            <Layers className="h-3.5 w-3.5 text-sp-accent shrink-0" />
            <span className="max-w-[210px] truncate font-medium">
              {sourceLabel || "当前结果"} · {scenesCount} 景
            </span>
          </button>
        </>
      )}

      {checkedCount > 0 && (
        <>
          <span className="text-sp-line">·</span>
          <button
            type="button"
            data-testid="agent-context-checked-pill"
            onClick={onOpenResults}
            className="inline-flex items-center gap-1 font-medium text-sp-accent hover:underline cursor-pointer transition-colors"
          >
            <CheckSquare className="h-3.5 w-3.5 shrink-0" />
            <span>已勾选 {checkedCount} 景</span>
          </button>
        </>
      )}

      {totalTasks > 0 && (
        <>
          <span className="text-sp-line">·</span>
          <button
            type="button"
            data-testid="agent-context-tasks-pill"
            onClick={onOpenTasks}
            className={cn(
              "inline-flex items-center gap-1 cursor-pointer transition-colors hover:text-sp-accent",
              activeTaskCount > 0 ? "font-medium text-sp-accent" : "text-sp-text-secondary"
            )}
          >
            <Download className="h-3.5 w-3.5 shrink-0" />
            <span className="tabular-nums">
              {activeTaskCount > 0
                ? `下载中 ${activeTaskCount}${completedTaskCount > 0 ? ` · 已完成 ${completedTaskCount}` : ""}`
                : `已完成任务 ${completedTaskCount}`}
            </span>
          </button>
        </>
      )}
    </div>
  );
}
