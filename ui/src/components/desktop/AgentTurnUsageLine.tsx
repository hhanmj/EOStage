import React, { useState } from "react";
import { ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import { turnUsageText } from "@/shell/api/agentUsage";
import type { RunUsageDetailState } from "./agentRunUsageHooks";

/**
 * 对话里一轮运行结束后的用量行（usage_total；没有上报时不显示）。
 * 悬停或展开时才请求逐次明细；展开后列出每次回复的用量。
 */
export function AgentTurnUsageLine({
  usageTotal,
  detail,
  onRequestDetail,
}: {
  usageTotal: unknown;
  detail?: RunUsageDetailState;
  onRequestDetail?: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const text = turnUsageText(usageTotal);
  if (text === null) return null;

  const detailText =
    detail?.status === "loaded" && detail.lines.length > 0 ? detail.lines.join("\n") : undefined;

  return (
    <div className="flex flex-col gap-1">
      <button
        type="button"
        aria-expanded={expanded}
        title={detailText}
        onMouseEnter={onRequestDetail}
        onFocus={onRequestDetail}
        onClick={() => {
          onRequestDetail?.();
          setExpanded((v) => !v);
        }}
        className="flex items-center gap-1 w-fit text-xs text-sp-text-muted tabular-nums hover:text-sp-text cursor-pointer select-none transition-colors duration-fast ease-sp"
      >
        <ChevronRight
          className={cn("h-3 w-3 shrink-0 transition-transform duration-fast ease-sp", expanded && "rotate-90")}
        />
        <span>{text}</span>
      </button>
      {expanded && (
        <div className="pl-4 flex flex-col gap-1 text-xs text-sp-text-muted tabular-nums">
          {(!detail || detail.status === "loading") && <span>正在读取逐次明细…</span>}
          {detail?.status === "failed" && <span>逐次明细暂不可用</span>}
          {detail?.status === "loaded" &&
            (detail.lines.length > 0 ? (
              detail.lines.map((line) => <span key={line}>{line}</span>)
            ) : (
              <span>运行记录里没有逐次用量</span>
            ))}
        </div>
      )}
    </div>
  );
}
