import { formatByteSize } from "@/lib/formatByteSize";
import { presentLocalReuse } from "@/lib/localReusePresentation";
import {
  taskItemHasCurrentFailureObservation,
  type TaskPagePresentation,
} from "@/lib/taskPagePresentation";
import type { ReactNode } from "react";

function bytes(value: number) {
  return formatByteSize(value);
}

export function TaskPageDetail({
  page,
  onPageIndexChange,
  busy = false,
  nisarSubsetProgress = false,
  renderItemDetails,
}: {
  page: TaskPagePresentation;
  onPageIndexChange: (pageIndex: number) => void;
  busy?: boolean;
  nisarSubsetProgress?: boolean;
  renderItemDetails?: (item: TaskPagePresentation["items"][number]) => ReactNode;
}) {
  const pageCount = Math.max(1, Math.ceil(page.totalCount / Math.max(1, page.pageSize)));
  const pageNumber = Math.min(pageCount, page.pageIndex + 1);
  return (
    <section className="rounded-lg border bg-muted/20 p-2 text-xs" data-task-page-detail>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="font-medium">任务项目明细 · {page.totalCount} 项</div>
        <div className="flex items-center gap-1">
          <button
            type="button"
            disabled={busy || pageNumber <= 1}
            onClick={() => onPageIndexChange(pageNumber - 2)}
          >
            上一页
          </button>
          <span>第 {pageNumber}/{pageCount} 页</span>
          <button
            type="button"
            disabled={busy || pageNumber >= pageCount}
            onClick={() => onPageIndexChange(pageNumber)}
          >
            下一页
          </button>
        </div>
      </div>
      <div className="mt-2 grid gap-1.5">
        {page.items.map((item) => (
          <article key={item.itemId} className="rounded border bg-background/70 px-2 py-1.5">
            <div className="flex min-w-0 items-center justify-between gap-2">
              <span className="truncate font-mono font-medium" title={item.displayName}>{item.displayName}</span>
              <span className="shrink-0">{item.status}</span>
            </div>
            <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-muted-foreground">
              {nisarSubsetProgress ? (
                <>
                  <span>输出字节 {item.status === "completed" ? bytes(item.bytesCompleted) : "待完成"}</span>
                  <span>源文件总量（参考） {item.sourceTotalBytes == null ? "不可用" : bytes(item.sourceTotalBytes)}</span>
                </>
              ) : (
                <span>{bytes(item.bytesCompleted)}{item.totalBytes === null ? " · 总量未知" : ` / ${bytes(item.totalBytes)} · ${item.percentage ?? "未知"}%`}</span>
              )}
              <span>速率 {bytes(item.bytesPerSecond)}/s</span>
              {item.destination && <span title={item.destination}>目标 <span>{item.destination}</span></span>}
              {taskItemHasCurrentFailureObservation(item) && item.failure && (
                <>
                  <span>原因 {item.failure.reasonZh}</span>
                  <span>代码 <span>{item.failure.code}</span></span>
                  <span>{item.failure.retryable ? "失败可重试" : "失败不可重试"}</span>
                </>
              )}
              <span>{item.retryable ? "项目可重试" : "项目不可重试"}</span>
            </div>
            {(() => {
              const reuse = presentLocalReuse(item.localReuse, item.status);
              return reuse && (
                <div className="mt-1 text-muted-foreground" data-local-reuse>
                  <span>{reuse.label}</span>
                  {reuse.detail && <span> · {reuse.detail}</span>}
                </div>
              );
            })()}
            {renderItemDetails?.(item)}
          </article>
        ))}
      </div>
    </section>
  );
}
