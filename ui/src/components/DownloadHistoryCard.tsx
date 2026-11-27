import { FileText, FolderOpen, Loader2, RotateCcw, Trash2 } from "lucide-react";
import { Badge, type BadgeProps } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { formatTaskTimestamp } from "@/lib/sarTaskDisplayName";

export type DownloadHistoryCardRow = {
  id: string;
  task_id?: string;
  revision?: number;
  name: string;
  status: string;
  detail: string;
  task_type?: string | null;
  concurrency?: number | null;
  created_at?: string | null;
  ended_at?: string | null;
  ts?: number;
};

export type DownloadHistoryStatusPresentation = Pick<BadgeProps, "variant" | "className">;

export type DownloadHistoryCardProps<Row extends DownloadHistoryCardRow> = {
  row: Row;
  title: string;
  statusLabel: string;
  statusPresentation?: DownloadHistoryStatusPresentation;
  busy?: boolean;
  error?: string;
  onOpen?(taskId: string, revision?: number): void;
  onOpenOutput?(taskId: string, revision: number): void;
  onOpenLocalOutput?(row: Row): void;
  onRestore?(row: Row): void;
  restoreBusy?: boolean;
  restoreTitle?: string;
  onDelete?(row: Row): void;
};

export function DownloadHistoryCard<Row extends DownloadHistoryCardRow>({
  row,
  title,
  statusLabel,
  statusPresentation,
  busy = false,
  error,
  onOpen,
  onOpenOutput,
  onOpenLocalOutput,
  onRestore,
  restoreBusy = false,
  restoreTitle,
  onDelete,
}: DownloadHistoryCardProps<Row>) {
  const taskId = row.task_id;
  const revision = row.revision;
  const canOpen = Boolean(taskId && onOpen);
  const canOpenOutput = Boolean(taskId && revision !== undefined && onOpenOutput);
  const hasActions = canOpen || canOpenOutput || Boolean(onOpenLocalOutput) || Boolean(onRestore);

  return (
    <div
      className="rounded-lg border bg-card px-3 py-2"
      data-testid="download-history-card"
      data-task-id={taskId}
    >
      <div
        className="grid grid-cols-[minmax(0,1fr)_auto_auto] items-start gap-2"
        data-history-header
      >
        <div className="min-w-0 whitespace-normal break-words text-sm font-semibold leading-5">
          {title}
        </div>
        <Badge
          {...statusPresentation}
          className={cn("shrink-0 whitespace-nowrap", statusPresentation?.className)}
          data-history-status
        >
          {statusLabel}
        </Badge>
        {onDelete ? (
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-7 w-7 shrink-0 text-muted-foreground hover:text-destructive"
            onClick={() => onDelete(row)}
            aria-label="删除历史记录"
            title="删除历史记录"
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        ) : (
          <span aria-hidden="true" />
        )}
      </div>

      <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-muted-foreground" data-history-facts>
        <span>任务类型：<span className="font-mono text-foreground">{row.task_type || "—"}</span></span>
        <span>并发：<span className="font-mono text-foreground">{row.concurrency ?? "—"}</span></span>
      </div>
      <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-muted-foreground" data-history-metadata>
        <span>创建：<span className="font-mono text-foreground">{formatTaskTimestamp(row.created_at ?? row.ts)}</span></span>
        {row.ended_at && <span>结束：<span className="font-mono text-foreground">{formatTaskTimestamp(row.ended_at)}</span></span>}
      </div>

      {hasActions && (
        <div
          className="mt-1.5 flex flex-wrap gap-1.5"
          data-history-actions
          data-task-history-actions={taskId || row.id}
        >
          {canOpen && taskId && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 shrink-0 px-2 text-xs"
              disabled={busy}
              onClick={() => onOpen?.(taskId, row.revision)}
            >
              {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <FileText className="h-3.5 w-3.5" />}
              详情
            </Button>
          )}
          {canOpenOutput && taskId && revision !== undefined && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 shrink-0 px-2 text-xs"
              onClick={() => onOpenOutput?.(taskId, revision)}
            >
              <FolderOpen className="h-3.5 w-3.5" />
              打开目录
            </Button>
          )}
          {onOpenLocalOutput && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 shrink-0 px-2 text-xs"
              onClick={() => onOpenLocalOutput(row)}
            >
              <FolderOpen className="h-3.5 w-3.5" />
              打开目录
            </Button>
          )}
          {onRestore && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 shrink-0 px-2 text-xs"
              disabled={restoreBusy}
              onClick={() => onRestore(row)}
              title={restoreTitle}
            >
              {restoreBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RotateCcw className="h-3.5 w-3.5" />}
              恢复
            </Button>
          )}
        </div>
      )}

      {busy && (
        <div className="mt-1.5 text-xs text-muted-foreground" data-history-pending>
          正在读取任务详情…
        </div>
      )}
      {error && (
        <div role="alert" className="mt-1.5 break-words text-xs text-destructive">
          {error}
        </div>
      )}
    </div>
  );
}
