import React from "react";
import { cn } from "@/lib/utils";

export function Skeleton({ className }: { className?: string }) {
  return (
    <div
      aria-hidden="true"
      className={cn("sp-skeleton rounded-sp-sm", className)}
    />
  );
}

const TITLE_WIDTHS = ["w-[72%]", "w-[58%]", "w-[66%]", "w-[52%]", "w-[74%]", "w-[61%]"];
const META_WIDTHS = ["w-[44%]", "w-[52%]", "w-[39%]", "w-[48%]", "w-[42%]", "w-[50%]"];

export function CatalogSkeletonRows({ rows = 6 }: { rows?: number }) {
  return (
    <div
      data-testid="catalog-skeleton-rows"
      aria-hidden="true"
      className="divide-y divide-sp-line"
    >
      {Array.from({ length: rows }).map((_, idx) => (
        <div
          key={idx}
          className="flex items-center justify-between gap-3 px-3 py-2.5 min-h-[60px]"
        >
          <div className="flex items-start gap-2.5 min-w-0 flex-1">
            <Skeleton className="mt-0.5 h-3.5 w-3.5 shrink-0 rounded-[4px]" />
            <div className="min-w-0 flex-1 space-y-2">
              <Skeleton className={cn("h-3.5", TITLE_WIDTHS[idx % TITLE_WIDTHS.length])} />
              <div className="flex items-center gap-2">
                <Skeleton className={cn("h-2.5", META_WIDTHS[idx % META_WIDTHS.length])} />
                <Skeleton className="h-2.5 w-14" />
              </div>
            </div>
          </div>
          <Skeleton className="h-7 w-[60px] shrink-0 rounded-sp-sm" />
        </div>
      ))}
    </div>
  );
}

const SESSION_TITLE_WIDTHS = ["w-[68%]", "w-[54%]", "w-[76%]", "w-[60%]", "w-[50%]"];

export function SessionRowSkeletons({ rows = 5 }: { rows?: number }) {
  return (
    <div data-testid="session-row-skeletons" aria-hidden="true" className="space-y-1 px-1 py-1">
      {Array.from({ length: rows }).map((_, idx) => (
        <div
          key={idx}
          className="flex items-center gap-2 rounded-sp-sm px-2.5 h-8"
        >
          <Skeleton className="h-3.5 w-3.5 shrink-0 rounded-[4px]" />
          <Skeleton className={cn("h-3", SESSION_TITLE_WIDTHS[idx % SESSION_TITLE_WIDTHS.length])} />
        </div>
      ))}
    </div>
  );
}

export function AgentMessagesSkeleton() {
  return (
    <div data-testid="agent-messages-skeleton" aria-hidden="true" className="space-y-5 py-4 px-2">
      <div className="flex justify-end">
        <Skeleton className="h-9 w-48 rounded-[18px]" />
      </div>
      <div className="space-y-2 max-w-[82%]">
        <Skeleton className="h-3.5 w-[78%]" />
        <Skeleton className="h-3.5 w-[64%]" />
        <Skeleton className="h-3.5 w-[44%]" />
      </div>
      <div className="flex justify-end">
        <Skeleton className="h-9 w-56 rounded-[18px]" />
      </div>
      <div className="space-y-2 max-w-[82%]">
        <Skeleton className="h-3.5 w-[72%]" />
        <Skeleton className="h-3.5 w-[55%]" />
      </div>
    </div>
  );
}

export function UsageStatsSkeleton() {
  return (
    <div data-testid="usage-stats-skeleton" aria-hidden="true" className="space-y-4">
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        {Array.from({ length: 4 }).map((_, idx) => (
          <div key={idx} className="eo-settings-card p-3 space-y-2">
            <Skeleton className="h-3 w-16" />
            <Skeleton className="h-5 w-20" />
            <Skeleton className="h-2.5 w-24" />
          </div>
        ))}
      </div>
      <div className="eo-settings-card p-3 space-y-2.5">
        <Skeleton className="h-3 w-28" />
        <div className="space-y-2 pt-1">
          {["w-[64%]", "w-[52%]", "w-[70%]"].map((w, idx) => (
            <div key={idx} className="flex items-center justify-between gap-4 py-1">
              <Skeleton className={cn("h-3", w)} />
              <Skeleton className="h-3 w-16 shrink-0" />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
