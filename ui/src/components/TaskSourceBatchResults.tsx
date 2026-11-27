import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useI18n } from "@/i18n";
import type { StoreTaskSourceBatchPage } from "@/lib/generated/v3-contracts";
import {
  taskHistoricalAction,
  type TaskHistoricalSort,
} from "@/lib/taskHistoricalContext";

function itemStatusKey(status: string | null, belongsToTask: boolean) {
  if (!belongsToTask) return "downloads.sourceBatch.itemStatusAvailable" as const;
  switch (status) {
    case "completed": return "downloads.sourceBatch.itemStatusCompleted" as const;
    case "failed": return "downloads.sourceBatch.itemStatusFailed" as const;
    case "cancelled": return "downloads.sourceBatch.itemStatusCancelled" as const;
    case "interrupted": return "downloads.sourceBatch.itemStatusInterrupted" as const;
    case "paused": return "downloads.sourceBatch.itemStatusPaused" as const;
    case "running": return "downloads.sourceBatch.itemStatusRunning" as const;
    case "queued": return "downloads.sourceBatch.itemStatusQueued" as const;
    default: return "downloads.sourceBatch.itemStatusInTask" as const;
  }
}

export function TaskSourceBatchResults({
  page,
  selectedIds,
  filter,
  sort,
  busy,
  mutationBusy,
  error = null,
  disabledReason = null,
  onRetryLoad,
  onToggleItem,
  onFilterChange,
  onSortChange,
  onPageIndexChange,
  onSubmit,
  onPreview,
}: {
  page: StoreTaskSourceBatchPage | null;
  selectedIds: readonly string[];
  filter: string;
  sort: TaskHistoricalSort;
  busy: boolean;
  mutationBusy: boolean;
  error?: string | null;
  disabledReason?: string | null;
  onRetryLoad?(): void;
  onToggleItem(itemId: string): void;
  onFilterChange(value: string): void;
  onSortChange(value: TaskHistoricalSort): void;
  onPageIndexChange(pageIndex: number): void;
  onSubmit(itemIds: string[]): void;
  onPreview?(item: StoreTaskSourceBatchPage["items"][number]["item"]): void;
}) {
  const { t } = useI18n();
  if (!page) {
    return (
      <div className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground space-y-3">
        <div>{error ? error : busy ? t("downloads.sourceBatch.loading") : t("downloads.sourceBatch.empty")}</div>
        {error && onRetryLoad && (
          <div>
            <Button type="button" size="sm" variant="outline" onClick={onRetryLoad}>
              {t("downloads.sourceBatch.retryLoad")}
            </Button>
          </div>
        )}
      </div>
    );
  }

  const selected = new Set(selectedIds);
  const normalizedFilter = filter.trim().toLocaleLowerCase();
  const rows = page.items
    .filter((entry) => !normalizedFilter || entry.item.display_name.toLocaleLowerCase().includes(normalizedFilter))
    .slice()
    .sort((left, right) => {
      if (sort === "name") return left.item.display_name.localeCompare(right.item.display_name);
      if (sort === "acquisition_time") {
        return (left.item.acquisition_datetime ?? "").localeCompare(right.item.acquisition_datetime ?? "");
      }
      return left.ordinal - right.ordinal;
    });
  const action = taskHistoricalAction(page.task_status);
  const actionLabel = action === "append"
    ? t("downloads.sourceBatch.appendAction")
    : action === "create_successor"
      ? t("downloads.sourceBatch.successorAction")
      : t("downloads.sourceBatch.unavailableAction");
  const totalPages = Math.max(1, Math.ceil(page.total_count / Math.max(1, page.page_size)));

  return (
    <section className="space-y-3" data-task-source-batch-results>
      {error && (
        <div role="alert" className="flex flex-wrap items-center gap-2 text-sm text-destructive">
          <span>{error}</span>
          {onRetryLoad && (
            <Button type="button" size="sm" variant="outline" disabled={busy} onClick={onRetryLoad}>
              {t("downloads.sourceBatch.retryLoad")}
            </Button>
          )}
        </div>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Input
          value={filter}
          className="min-w-[220px] flex-1"
          placeholder={t("downloads.sourceBatch.filterPlaceholder")}
          aria-label={t("downloads.sourceBatch.filterAria")}
          onChange={(event) => onFilterChange(event.target.value)}
        />
        <select
          className="h-9 rounded-md border bg-background px-3 text-sm"
          aria-label={t("downloads.sourceBatch.sortAria")}
          value={sort}
          onChange={(event) => onSortChange(event.target.value as TaskHistoricalSort)}
        >
          <option value="source_order">{t("downloads.sourceBatch.sortSourceOrder")}</option>
          <option value="name">{t("downloads.sourceBatch.sortName")}</option>
          <option value="acquisition_time">{t("downloads.sourceBatch.sortAcquisitionTime")}</option>
        </select>
        <Badge variant="outline">{t("downloads.sourceBatch.batchBadge", { batchId: page.source_batch_id })}</Badge>
        <Badge variant="outline">{t("downloads.sourceBatch.revisionBadge", { rev: page.task_revision })}</Badge>
        <Badge variant="outline">{t("downloads.sourceBatch.totalItemsBadge", { count: page.total_count })}</Badge>
      </div>

      <div className="overflow-hidden rounded-xl border bg-card">
        {rows.map((entry) => {
          const itemId = entry.item.item_id;
          const disabled = entry.belongs_to_task;
          return (
            <div
              key={itemId}
              className={`grid grid-cols-[auto_minmax(0,1fr)_auto_auto] items-center gap-3 border-b px-3 py-3 text-sm last:border-b-0 ${disabled ? "bg-muted/30" : "cursor-pointer"}`}
            >
              <label className="contents">
              <input
                type="checkbox"
                aria-label={t("downloads.sourceBatch.selectAria", { name: entry.item.display_name })}
                checked={disabled || selected.has(itemId)}
                disabled={disabled || mutationBusy}
                onChange={() => onToggleItem(itemId)}
              />
              <span className="min-w-0">
                <span className="block truncate font-medium" title={entry.item.display_name}>{entry.item.display_name}</span>
                <span className="mt-0.5 block truncate text-xs text-muted-foreground">
                  {entry.item.provider_id} · {entry.item.acquisition_datetime ?? t("downloads.sourceBatch.acquisitionUnknown")}
                </span>
              </span>
              </label>
              <Badge variant="outline">{t(itemStatusKey(entry.task_status, entry.belongs_to_task))}</Badge>
              {onPreview && <Button type="button" size="sm" variant="ghost"
                aria-label={t("downloads.sourceBatch.previewAria", { name: entry.item.display_name })} onClick={() => onPreview(entry.item)}>{t("downloads.sourceBatch.preview")}</Button>}
            </div>
          );
        })}
        {rows.length === 0 && (
          <div className="p-8 text-center text-sm text-muted-foreground">{t("downloads.sourceBatch.emptyCurrentPage")}</div>
        )}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={busy || page.page_index <= 0}
            onClick={() => onPageIndexChange(page.page_index - 1)}
          >
            {t("downloads.sourceBatch.prevPage")}
          </Button>
          <span>{t("downloads.sourceBatch.pageIndicator", { current: page.page_index + 1, total: totalPages })}</span>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={busy || page.page_index + 1 >= totalPages}
            onClick={() => onPageIndexChange(page.page_index + 1)}
          >
            {t("downloads.sourceBatch.nextPage")}
          </Button>
        </div>
        <div className="flex items-center gap-2">
          <span className="text-xs text-muted-foreground">{t("downloads.sourceBatch.selectedCount", { count: selectedIds.length })}</span>
          <Button
            type="button"
            size="sm"
            disabled={selectedIds.length === 0 || busy || Boolean(error) || mutationBusy || action === "unavailable" || Boolean(disabledReason)}
            onClick={() => onSubmit(Array.from(selectedIds))}
          >
            {mutationBusy ? t("downloads.sourceBatch.submitting") : actionLabel}
          </Button>
        </div>
      </div>
      {disabledReason && (
        <p className="text-xs text-destructive">
          {disabledReason}
        </p>
      )}
      {selectedIds.length > 250 && (
        <p className="text-xs text-muted-foreground">
          {t("downloads.sourceBatch.largeSelectionNote")}
        </p>
      )}
      {action === "create_successor" && !disabledReason && (
        <p className="text-xs text-muted-foreground">
          {t("downloads.sourceBatch.successorNote")}
        </p>
      )}
      {action === "unavailable" && !disabledReason && (
        <p className="text-xs text-muted-foreground">
          {t("downloads.sourceBatch.unavailableNote")}
        </p>
      )}
    </section>
  );
}
