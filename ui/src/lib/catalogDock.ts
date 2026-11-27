import type { DemTaskStatus } from "./bridge";

export type DockTab = "results" | "preview" | "assets" | "metadata";
export type DockHeightMode = "collapsed" | "normal";
export type DockContext = "catalog" | "dem";

export type DockPreferences = {
  tab: DockTab;
  heightMode: DockHeightMode;
  context: DockContext;
  heightPx: number;
};

export const DEFAULT_DOCK_HEIGHT_PX = 300;

export const DEFAULT_DOCK_PREFERENCES: DockPreferences = Object.freeze({
  tab: "results",
  heightMode: "collapsed",
  context: "catalog",
  heightPx: DEFAULT_DOCK_HEIGHT_PX,
});

export function scopeSelectionKeys(
  availableKeys: Iterable<string>,
  selectedKeys: Iterable<string>,
): Set<string> {
  const selected = new Set(selectedKeys);
  return new Set(Array.from(availableKeys).filter((key) => selected.has(key)));
}

const DOCK_TABS = new Set<DockTab>(["results", "preview", "assets", "metadata"]);
const DOCK_HEIGHTS = new Set<DockHeightMode>(["collapsed", "normal"]);
const DOCK_CONTEXTS = new Set<DockContext>(["catalog", "dem"]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

export function parseDockPreferences(value: unknown): DockPreferences {
  if (!isRecord(value)) return { ...DEFAULT_DOCK_PREFERENCES };
  const tab = value.tab;
  const heightMode = value.heightMode;
  const context = value.context;
  const normalizedHeightMode = heightMode === "expanded"
    ? "normal"
    : DOCK_HEIGHTS.has(heightMode as DockHeightMode)
      ? heightMode as DockHeightMode
      : null;
  if (
    typeof tab !== "string"
    || typeof heightMode !== "string"
    || typeof context !== "string"
    || !DOCK_TABS.has(tab as DockTab)
    || normalizedHeightMode === null
    || !DOCK_CONTEXTS.has(context as DockContext)
  ) {
    return { ...DEFAULT_DOCK_PREFERENCES };
  }
  return {
    tab: "results",
    heightMode: normalizedHeightMode,
    context: context as DockContext,
    heightPx:
      typeof value.heightPx === "number" && Number.isFinite(value.heightPx)
        ? Math.max(180, Math.round(value.heightPx))
        : DEFAULT_DOCK_HEIGHT_PX,
  };
}

export function clampDockHeight(requested: number, availableHeight: number): number {
  const safeAvailable = Number.isFinite(availableHeight) && availableHeight > 0
    ? Math.floor(availableHeight)
    : 620;
  const safeRequested = Number.isFinite(requested) ? Math.round(requested) : DEFAULT_DOCK_HEIGHT_PX;
  const maximum = Math.max(180, safeAvailable - 160);
  return Math.min(maximum, Math.max(180, safeRequested));
}

export function scrollTopForProduct(
  rows: readonly { provider_product_key: string }[],
  key: string,
  rowHeight: number,
): number | null {
  const index = rows.findIndex((row) => row.provider_product_key === key);
  if (index < 0) return null;
  const safeRowHeight = Math.max(1, finiteNonNegative(rowHeight, 1));
  return index * safeRowHeight;
}

export type VirtualRowWindow = {
  start: number;
  end: number;
  offset: number;
  totalHeight: number;
};

function finiteNonNegative(value: number, fallback = 0): number {
  return Number.isFinite(value) && value >= 0 ? value : fallback;
}

export function virtualRowWindow(
  itemCount: number,
  scrollTop: number,
  viewportHeight: number,
  rowHeight = 48,
  overscan = 6,
): VirtualRowWindow {
  const count = Math.max(0, Math.floor(finiteNonNegative(itemCount)));
  if (count === 0) return { start: 0, end: 0, offset: 0, totalHeight: 0 };
  const safeRowHeight = Math.max(1, finiteNonNegative(rowHeight, 1));
  const safeScrollTop = finiteNonNegative(scrollTop);
  const safeViewportHeight = Math.max(1, finiteNonNegative(viewportHeight, 1));
  const safeOverscan = Math.max(0, Math.floor(finiteNonNegative(overscan)));
  const firstVisible = Math.min(count - 1, Math.floor(safeScrollTop / safeRowHeight));
  const visibleCount = Math.max(1, Math.ceil(safeViewportHeight / safeRowHeight));
  const start = Math.max(0, firstVisible - safeOverscan);
  const end = Math.min(count, firstVisible + visibleCount + safeOverscan);
  return {
    start,
    end,
    offset: start * safeRowHeight,
    totalHeight: count * safeRowHeight,
  };
}

export type DemDockArtifactKind =
  | "source_dem"
  | "ellipsoid_dem"
  | "sarscape_ready_dem"
  | "results_table"
  | "conversion_results_table";

export type DemDockArtifact = {
  kind: DemDockArtifactKind;
  label: string;
  path: string;
};

export type DemDockProgress = {
  determinate: boolean;
  completed: number;
  total: number | null;
  percent: number | null;
  unit: "bytes" | "tasks";
};

/** Disposable display projection over the owning DEM manager status. */
export type DemDockProjection = {
  task_id: string;
  name: string;
  provider: string;
  dataset: string;
  state: string;
  bbox_summary: string;
  current_stage: string;
  current_item: string;
  output_dir: string;
  error: string | null;
  remote_source_summary: string;
  progress: DemDockProgress;
  artifacts: DemDockArtifact[];
};

function bboxSummary(task: DemTaskStatus): string {
  const bbox = task.task_snapshot.bbox;
  return [
    `W${bbox.west.toFixed(3)}`,
    `S${bbox.south.toFixed(3)}`,
    `E${bbox.east.toFixed(3)}`,
    `N${bbox.north.toFixed(3)}`,
  ].join(" ");
}

function progressFor(task: DemTaskStatus): DemDockProgress {
  const byteTotal = Number(task.total_bytes ?? 0);
  if (Number.isFinite(byteTotal) && byteTotal > 0) {
    const completed = Math.max(0, Number(task.done_bytes ?? 0));
    return {
      determinate: true,
      completed,
      total: byteTotal,
      percent: Math.max(0, Math.min(100, Math.round((completed / byteTotal) * 100))),
      unit: "bytes",
    };
  }
  const taskTotal = Number(task.total ?? 0);
  if (Number.isFinite(taskTotal) && taskTotal > 0) {
    const completed = Math.max(0, Number(task.done ?? 0));
    return {
      determinate: true,
      completed,
      total: taskTotal,
      percent: Math.max(0, Math.min(100, Math.round((completed / taskTotal) * 100))),
      unit: "tasks",
    };
  }
  return {
    determinate: false,
    completed: Math.max(0, Number(task.done ?? 0)),
    total: null,
    percent: null,
    unit: "tasks",
  };
}

function artifactsFor(task: DemTaskStatus): DemDockArtifact[] {
  const values: Array<[DemDockArtifactKind, string, string | undefined]> = [
    ["source_dem", "Source / cropped DEM", task.raw_dem_path],
    ["ellipsoid_dem", "Ellipsoid-height DEM", task.ellipsoid_dem_path],
    ["sarscape_ready_dem", "SARscape-ready DEM", task.sarscape_ready_dem_path],
    ["results_table", "Download result table", task.results_path],
    ["conversion_results_table", "Conversion result table", task.conversion_results_path],
  ];
  return values.flatMap(([kind, label, path]) => {
    const normalized = path?.trim();
    return normalized ? [{ kind, label, path: normalized }] : [];
  });
}

export function projectDemTasksToDock(
  tasks: readonly DemTaskStatus[],
): DemDockProjection[] {
  return tasks.map((task) => ({
    task_id: task.task_id,
    name: task.name || `${task.task_snapshot.provider} ${task.dataset}`,
    provider: task.task_snapshot.provider,
    dataset: task.task_snapshot.dataset || task.dataset,
    state: task.state,
    bbox_summary: bboxSummary(task),
    current_stage: task.summary_line || "等待任务状态",
    current_item: task.current_scene || "—",
    output_dir: task.output_dir || task.task_snapshot.output_root,
    error: task.error,
    remote_source_summary: [
      task.task_snapshot.provider,
      task.task_snapshot.dataset || task.dataset,
      task.task_snapshot.source_policy,
    ].join(" · "),
    progress: progressFor(task),
    artifacts: artifactsFor(task),
  }));
}
