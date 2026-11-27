import { DownloadHistoryCard, type DownloadHistoryCardRow, type DownloadHistoryStatusPresentation } from "@/components/DownloadHistoryCard";
import { Button } from "@/components/ui/button";

export type DownloadTaskHistoryRow = DownloadHistoryCardRow;

export type DownloadTaskHistoryProps<Row extends DownloadTaskHistoryRow> = {
  rows: readonly Row[];
  busyByTask: Readonly<Record<string, string | undefined>>;
  errorByTask: Readonly<Record<string, string | undefined>>;
  titleFor?(row: Row): string;
  statusLabelFor?(row: Row): string;
  statusPresentationFor?(row: Row): DownloadHistoryStatusPresentation;
  onOpen?(taskId: string, revision?: number): void;
  onOpenOutput?(taskId: string, revision: number): void;
  canOpenLocalOutput?(row: Row): boolean;
  onOpenLocalOutput?(row: Row): void;
  canRestore?(row: Row): boolean;
  onRestore?(row: Row): void;
  restoreBusyFor?(row: Row): boolean;
  restoreTitleFor?(row: Row): string | undefined;
  canDelete?(row: Row): boolean;
  onDelete?(row: Row): void;
  hasMore?: boolean;
  onNext?(): void;
  onPrevious?(): void;
};

/** Terminal-history product boundary. It owns the compact card layout and
 * delegates every action with the exact task/revision authority from its row. */
export function DownloadTaskHistory<Row extends DownloadTaskHistoryRow>({
  rows,
  busyByTask,
  errorByTask,
  titleFor = (row) => row.name,
  statusLabelFor = (row) => row.status,
  statusPresentationFor = () => ({ variant: "neutral" }),
  onOpen,
  onOpenOutput,
  canOpenLocalOutput,
  onOpenLocalOutput,
  canRestore,
  onRestore,
  restoreBusyFor,
  restoreTitleFor,
  canDelete,
  onDelete,
  hasMore = false,
  onNext,
  onPrevious,
}: DownloadTaskHistoryProps<Row>) {
  return (
    <div className="space-y-2" data-ui-download-history>
      {rows.map((row) => {
        const taskId = row.task_id;
        return (
          <DownloadHistoryCard
            key={row.id}
            row={row}
            title={titleFor(row)}
            statusLabel={statusLabelFor(row)}
            statusPresentation={statusPresentationFor(row)}
            busy={Boolean(taskId && busyByTask[taskId] === "open")}
            error={taskId ? errorByTask[taskId] : undefined}
            onOpen={onOpen}
            onOpenOutput={onOpenOutput}
            onOpenLocalOutput={
              onOpenLocalOutput && (canOpenLocalOutput?.(row) ?? true)
                ? onOpenLocalOutput
                : undefined
            }
            onRestore={onRestore && (canRestore?.(row) ?? true) ? onRestore : undefined}
            restoreBusy={restoreBusyFor?.(row) ?? false}
            restoreTitle={restoreTitleFor?.(row)}
            onDelete={onDelete && (canDelete?.(row) ?? true) ? onDelete : undefined}
          />
        );
      })}
      {(onPrevious || hasMore) && (
        <div className="flex flex-wrap gap-2">
          <Button size="sm" variant="outline" disabled={!onPrevious} onClick={onPrevious}>上一页</Button>
          <Button size="sm" variant="outline" disabled={!hasMore} onClick={onNext}>下一页</Button>
        </div>
      )}
    </div>
  );
}
