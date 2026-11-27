import { Loader2, MapPinned } from "lucide-react";

import { Button } from "@/components/ui/button";
import { stableBoundaryIdentity } from "@/lib/aoiSelection";
import type { AdminBoundary } from "@/lib/bridge";
import { formatSignedLat, formatSignedLon } from "@/lib/geoCoords";
import { cn } from "@/lib/utils";

export type AoiCandidatePanelProps = {
  candidates: readonly AdminBoundary[];
  selected: AdminBoundary | null;
  detailBusy: boolean;
  bindBusy: boolean;
  onPreview: (value: AdminBoundary) => void;
  onBind: (value: AdminBoundary) => void;
};

export function AoiCandidatePanel({
  candidates,
  selected,
  detailBusy,
  bindBusy,
  onPreview,
  onBind,
}: AoiCandidatePanelProps) {
  if (candidates.length === 0) return null;

  return (
    <div
      data-testid="aoi-candidate-panel"
      className="flex min-h-0 flex-col rounded-2xl border border-white/45 bg-white/35 text-xs shadow-sm backdrop-blur-xl dark:border-white/10 dark:bg-white/10"
    >
      <div className="shrink-0 px-2 pt-2 font-medium">候选边界（{candidates.length}）</div>
      <div
        data-testid="aoi-candidate-scroll"
        role="listbox"
        aria-label="行政边界候选"
        className="min-h-0 max-h-36 space-y-2 overflow-y-auto p-2"
      >
        {candidates.map((item, index) => {
          const selectedItem = Boolean(
            selected && stableBoundaryIdentity(selected) === stableBoundaryIdentity(item),
          );
          return (
            <button
              key={stableBoundaryIdentity(item) || `unresolved-boundary:${index}`}
              type="button"
              role="option"
              aria-selected={selectedItem}
              onClick={() => onPreview(item)}
              disabled={detailBusy}
              className={cn(
                "block w-full rounded-xl px-2 py-2 text-left transition-colors hover:bg-white/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring dark:hover:bg-white/10",
                selectedItem && "bg-primary/10 ring-1 ring-primary/25",
              )}
            >
              <span className="block truncate font-medium">{item.label}</span>
              <span className="mt-1 block text-[11px] text-muted-foreground">
                {item.source || "行政边界服务"} · {item.geojson ? "真实边界" : detailBusy ? "正在解析" : "候选范围"}
              </span>
              {item.attribution && (
                <span className="mt-1 block text-[10px] text-muted-foreground">{item.attribution}</span>
              )}
              <span className="mt-1 block font-mono text-[11px] text-muted-foreground">
                {formatSignedLon(item.bbox.west)} {formatSignedLat(item.bbox.south)} {formatSignedLon(item.bbox.east)} {formatSignedLat(item.bbox.north)}
              </span>
            </button>
          );
        })}
      </div>
      {selected && (
        <div
          data-testid="aoi-candidate-footer"
          className="sticky bottom-0 shrink-0 border-t border-white/45 bg-popover/95 p-2 backdrop-blur-xl dark:border-white/10"
        >
          <Button
            type="button"
            className="w-full"
            onClick={() => onBind(selected)}
            disabled={bindBusy}
            aria-busy={bindBusy}
            data-state={bindBusy ? "binding" : "idle"}
          >
            {bindBusy ? <Loader2 className="h-4 w-4 animate-spin" /> : <MapPinned className="h-4 w-4" />}
            设置为AOI
          </Button>
        </div>
      )}
    </div>
  );
}
