/**
 * AgentTaskCard.tsx
 * Agent 对话中的任务卡（结论引用的任务），从 DesktopAgentWorkbench.tsx 抽出。
 * 结构与样式沿用原内联写法；新增：磁盘空间不足暂停原因行（W2d pause_reason）与动作失败说明行。
 */

import React, { useEffect, useState } from "react";
import { Download, HardDrive, Pause, Play, RotateCcw, X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { TaskCardModel } from "@/shell/api/agentConversationLogic";
import { pauseReasonLine } from "@/shell/api/taskCardLogic";
import { resolveTaskActionErrorText } from "@/shell/api/taskActionFlow";

type AgentTaskCardAction = "pause" | "resume" | "cancel" | "retry";

export interface AgentTaskCardProps {
  model: TaskCardModel;
  onTaskAction?: (
    taskId: string,
    revision: number,
    action: AgentTaskCardAction
  ) => Promise<unknown> | unknown;
}

export function AgentTaskCard({ model: m, onTaskAction }: AgentTaskCardProps) {
  const [actionError, setActionError] = useState<string | null>(null);
  const isStatusUnknown = m.statusKnown === false || !m.task.status;

  // 任务状态或修订号变化即视为后端已有新结果，清掉上一次的失败说明
  useEffect(() => {
    setActionError(null);
  }, [m.task.status, m.task.revision, m.task.updated_at]);

  const handleAction = async (act: AgentTaskCardAction) => {
    if (!onTaskAction || isStatusUnknown || typeof m.task.revision !== "number") return;
    setActionError(null);
    try {
      await onTaskAction(m.taskId, m.task.revision, act);
    } catch (err: unknown) {
      setActionError(resolveTaskActionErrorText(err, "操作未能完成"));
    }
  };

  const pauseLine = isStatusUnknown ? null : pauseReasonLine(m.task.pause_reason);

  const cardActions = isStatusUnknown
    ? []
    : m.actions.filter(
        (act): act is AgentTaskCardAction =>
          act === "pause" || act === "resume" || act === "cancel" || act === "retry"
      );

  return (
    <div
      className="rounded-card bg-sp-surface border border-sp-line p-4 shadow-card flex flex-col gap-3"
      aria-label="任务卡"
    >
      <div className="flex items-center gap-2">
        <Download className="h-4 w-4 text-sp-text-secondary shrink-0" />
        <span className="text-sm font-semibold text-sp-text flex-1 truncate">
          {m.task.name || "未命名任务"}
        </span>
        <div className="flex items-center gap-1.5 text-xs text-sp-text-secondary font-medium">
          <span
            className={cn(
              "h-2 w-2 rounded-full",
              isStatusUnknown
                ? "bg-sp-text-muted"
                : [
                    m.task.status === "running" && "bg-sp-accent",
                    m.task.status === "completed" && "bg-sp-status-success",
                    m.task.status === "paused" && "bg-sp-status-warning",
                    m.task.status === "failed" && "bg-sp-status-danger",
                    m.task.status === "queued" && "bg-sp-text-muted",
                  ]
            )}
          />
          <span>
            {isStatusUnknown ? "状态未知" : m.statusLabel}
            {!isStatusUnknown && m.percent !== null && m.task.status === "running"
              ? ` ${m.percent}%`
              : ""}
          </span>
        </div>
      </div>

      {m.productLine && (
        <div className="font-mono text-xs text-sp-text break-all leading-5">{m.productLine}</div>
      )}

      {!isStatusUnknown && m.percent !== null && (
        <div className="h-1.5 w-full bg-sp-canvas rounded-full overflow-hidden">
          <div
            className="h-full bg-sp-accent transition-[width] duration-200 ease-sp"
            style={{ width: `${m.percent}%` }}
          />
        </div>
      )}

      {pauseLine && (
        <div className="flex items-center gap-2 text-xs text-sp-warning-text">
          <HardDrive className="h-3.5 w-3.5 shrink-0" />
          <span>{pauseLine}</span>
        </div>
      )}

      {actionError && (
        <div role="alert" className="rounded-sp-sm bg-sp-danger-bg p-2 text-xs text-sp-danger-text">
          {actionError}
        </div>
      )}

      <div className="flex items-center justify-between gap-3 text-xs text-sp-text-secondary flex-wrap pt-0.5">
        <span className="font-medium">
          {m.sizeLine}
          {m.remainingMinutesZh ? `　${m.remainingMinutesZh}` : ""}
        </span>

        {cardActions.length > 0 && onTaskAction && (
          <div className="flex items-center gap-2 ml-auto">
            {cardActions.map((act) => (
              <button
                key={act}
                type="button"
                onClick={() => void handleAction(act)}
                className="flex items-center gap-1 px-2.5 py-1 rounded-md border border-sp-line bg-sp-surface text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer transition-colors duration-fast ease-sp"
              >
                {act === "pause" && <Pause className="h-3 w-3" />}
                {act === "resume" && <Play className="h-3 w-3" />}
                {act === "cancel" && <X className="h-3 w-3" />}
                {act === "retry" && <RotateCcw className="h-3 w-3" />}
                <span>
                  {act === "pause" && "暂停"}
                  {act === "resume" && "继续"}
                  {act === "cancel" && "取消"}
                  {act === "retry" && "重试"}
                </span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
