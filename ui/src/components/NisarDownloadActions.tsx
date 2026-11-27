import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";

export type NisarSubsetActionState = "ready" | "blocked" | "integrating";

export type NisarDownloadActionsProps = {
  completeEnabled: boolean;
  completeBusy?: boolean;
  completeDisabledReason?: string | null;
  subsetState: NisarSubsetActionState;
  subsetReason?: string | null;
  subsetDisabled?: boolean;
  subsetBusy?: boolean;
  bandLabel?: string;
  sourceLabel?: string;
  onCompleteDownload: () => void;
  onOpenSubsetWorkspace: () => void;
};

export function NisarDownloadActions({
  completeEnabled,
  completeBusy = false,
  completeDisabledReason = null,
  subsetState,
  subsetReason = null,
  subsetDisabled = false,
  subsetBusy = false,
  bandLabel = "L-band",
  sourceLabel = "ASF / Earthdata",
  onCompleteDownload,
  onOpenSubsetWorkspace,
}: NisarDownloadActionsProps) {
  const status = subsetBusy
    ? subsetReason ?? "正在加载完整冻结批次…"
    : subsetState === "ready"
      ? "可进入裁剪工作区"
      : subsetReason ?? (subsetState === "integrating" ? "该波段正在接入" : "当前不能提交裁剪任务");
  const completeDisabled = completeBusy || !completeEnabled;
  const completeStatus = completeDisabled
    ? completeDisabledReason ?? (completeBusy ? "正在下载完整产品" : "完整产品下载当前不可用")
    : null;

  return (
    <div
      data-nisar-download-actions
      className="flex min-w-0 flex-wrap items-center gap-1.5 rounded-lg border border-primary/25 bg-primary/5 p-2 text-[11px]"
    >
      <span className="px-1 font-medium">NISAR 下载</span>
      <Badge variant="neutral" className="h-5 px-1.5 text-[10px]" data-nisar-band-source>
        {bandLabel} · {sourceLabel}
      </Badge>
      <button
        type="button"
        disabled={completeDisabled}
        aria-label={completeBusy ? "正在下载完整产品" : "完整产品"}
        onClick={onCompleteDownload}
        className="rounded-md border border-border/70 bg-card px-2.5 py-1.5 font-medium transition hover:border-primary/45 hover:bg-muted disabled:cursor-not-allowed disabled:opacity-50"
      >
        {completeBusy ? "正在下载完整产品" : "完整产品"}
      </button>
      <button
        type="button"
        disabled={subsetBusy || subsetDisabled}
        onClick={onOpenSubsetWorkspace}
        className={cn(
          "rounded-md border px-2.5 py-1.5 font-medium transition hover:bg-muted disabled:cursor-wait disabled:opacity-60",
          subsetState === "ready" ? "border-primary bg-primary text-primary-foreground" : "border-warning/50 bg-warning/10",
        )}
      >
        {subsetBusy ? "正在加载裁剪批次" : "AOI 裁剪"}
      </button>
      <span className={cn(
        "min-w-0 flex-1 px-1 text-[10px] leading-4",
        subsetState === "ready" ? "text-muted-foreground" : "text-warning",
      )}>
        {status}
      </span>
      {completeStatus && (
        <span className="basis-full px-1 text-[10px] leading-4 text-warning">
          {completeStatus}
        </span>
      )}
    </div>
  );
}
