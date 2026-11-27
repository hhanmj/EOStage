export type ManualColumn = "search" | "map" | "results";
export type ManualColumnSplitter = "search-map" | "map-results";

export type ManualColumnState = {
  collapsed: Record<ManualColumn, boolean>;
  widths: Record<"search" | "results", number>;
};

export type ManualColumnLayout = {
  collapsed: Record<ManualColumn, boolean>;
  widths: Record<ManualColumn, number>;
};

export const MANUAL_COLUMNS_STORAGE_KEY = "eostage:manual-workbench:columns:v1";
export const COLLAPSED_MANUAL_COLUMN_WIDTH = 0;
export const NARROW_WORKBENCH_THRESHOLD = 960;
const MANUAL_SPLITTER_TOTAL_WIDTH = 16;
export const MANUAL_COLUMN_LIMITS = {
  search: { min: 280, max: 480, default: 320 },
  map: { min: 360, default: null },
  results: { min: 340, default: 440 },
} as const;

export const DEFAULT_MANUAL_COLUMNS: ManualColumnState = {
  collapsed: { search: false, map: false, results: false },
  widths: { search: 320, results: 440 },
};

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, Math.round(value)));
}

export function resolveManualColumnLayout(state: ManualColumnState, availableWidth: number): ManualColumnLayout {
  const available = Math.max(0, Math.round(availableWidth) - MANUAL_SPLITTER_TOTAL_WIDTH);
  const collapsed: Record<ManualColumn, boolean> = {
    search: state.collapsed.search,
    map: false, // 地图列本身不提供折叠为竖条
    results: state.collapsed.results,
  };

  let searchWidth = collapsed.search
    ? 0
    : clamp(state.widths.search, MANUAL_COLUMN_LIMITS.search.min, MANUAL_COLUMN_LIMITS.search.max);

  let resultsWidth = 0;
  if (!collapsed.results) {
    const maxResults = available - searchWidth - MANUAL_COLUMN_LIMITS.map.min;
    resultsWidth = clamp(
      state.widths.results,
      MANUAL_COLUMN_LIMITS.results.min,
      Math.max(MANUAL_COLUMN_LIMITS.results.min, maxResults)
    );
  }

  let mapWidth = Math.max(
    MANUAL_COLUMN_LIMITS.map.min,
    available - searchWidth - resultsWidth
  );

  // Small-screen responsive fit (e.g. 920x620 min window): prevent horizontal overflow
  const totalUncompressed = searchWidth + mapWidth + resultsWidth;
  if (available > 0 && totalUncompressed > available) {
    if (collapsed.search !== collapsed.results) {
      // One side column collapsed (narrow auto-focus mode): reserve usable map canvas + readable side panel
      const singleSideMin = !collapsed.search ? 280 : 320;
      const preferredMap = Math.min(
        MANUAL_COLUMN_LIMITS.map.min,
        Math.max(260, Math.floor(available * 0.45)),
      );
      const maxSideBudget = Math.max(200, available - preferredMap);
      if (!collapsed.search) {
        searchWidth = clamp(searchWidth, Math.min(singleSideMin, maxSideBudget), maxSideBudget);
        mapWidth = Math.max(160, available - searchWidth);
      } else {
        resultsWidth = clamp(resultsWidth, Math.min(singleSideMin, maxSideBudget), maxSideBudget);
        mapWidth = Math.max(160, available - resultsWidth);
      }
      return {
        collapsed,
        widths: {
          search: searchWidth,
          map: mapWidth,
          results: resultsWidth,
        },
      };
    }

    const ratio = available / totalUncompressed;
    if (!collapsed.search) {
      searchWidth = Math.max(180, Math.floor(searchWidth * ratio));
    }
    if (!collapsed.results) {
      resultsWidth = Math.max(200, Math.floor(resultsWidth * ratio));
    }
    const remainingForMap = available - searchWidth - resultsWidth;
    if (remainingForMap >= 160) {
      mapWidth = remainingForMap;
    } else {
      mapWidth = Math.max(120, Math.min(available, 160));
      const sideBudget = Math.max(0, available - mapWidth);
      if (!collapsed.search && !collapsed.results && searchWidth + resultsWidth > 0) {
        const nextSearch = Math.floor((searchWidth / (searchWidth + resultsWidth)) * sideBudget);
        searchWidth = nextSearch;
        resultsWidth = Math.max(0, sideBudget - nextSearch);
      } else if (!collapsed.search) {
        searchWidth = sideBudget;
      } else if (!collapsed.results) {
        resultsWidth = sideBudget;
      }
    }
  }

  return {
    collapsed,
    widths: {
      search: searchWidth,
      map: mapWidth,
      results: resultsWidth,
    },
  };
}

export function resizeManualColumns(
  state: ManualColumnState,
  splitter: ManualColumnSplitter,
  delta: number,
  availableWidth: number,
): ManualColumnState {
  const usableWidth = Math.max(0, availableWidth - MANUAL_SPLITTER_TOTAL_WIDTH);
  const layout = resolveManualColumnLayout(state, availableWidth);
  const next: ManualColumnState = {
    collapsed: { ...state.collapsed, map: false },
    widths: { ...state.widths },
  };

  if (splitter === "search-map") {
    if (layout.collapsed.search) {
      if (delta >= MANUAL_COLUMN_LIMITS.search.min) {
        next.collapsed.search = false;
        next.widths.search = clamp(delta, MANUAL_COLUMN_LIMITS.search.min, MANUAL_COLUMN_LIMITS.search.max);
      }
      return next;
    }
    const rawSearch = layout.widths.search + delta;
    const maxAllowedSearch = Math.min(
      MANUAL_COLUMN_LIMITS.search.max,
      usableWidth - layout.widths.results - MANUAL_COLUMN_LIMITS.map.min
    );
    next.widths.search = clamp(
      rawSearch,
      MANUAL_COLUMN_LIMITS.search.min,
      Math.max(MANUAL_COLUMN_LIMITS.search.min, maxAllowedSearch)
    );
    return next;
  }

  if (layout.collapsed.results) {
    if (-delta >= MANUAL_COLUMN_LIMITS.results.min) {
      next.collapsed.results = false;
      next.widths.results = clamp(-delta, MANUAL_COLUMN_LIMITS.results.min, MANUAL_COLUMN_LIMITS.results.default);
    }
    return next;
  }

  // 向右拖动 delta > 0 时结果栏变窄 (- delta)
  const rawResults = layout.widths.results - delta;
  const maxAllowedResults = usableWidth - layout.widths.search - MANUAL_COLUMN_LIMITS.map.min;
  next.widths.results = clamp(
    rawResults,
    MANUAL_COLUMN_LIMITS.results.min,
    Math.max(MANUAL_COLUMN_LIMITS.results.min, maxAllowedResults)
  );
  return next;
}

export function resetManualSplitter(state: ManualColumnState, splitter: ManualColumnSplitter): ManualColumnState {
  const next: ManualColumnState = {
    collapsed: { ...state.collapsed, map: false },
    widths: { ...state.widths },
  };
  if (splitter === "search-map") {
    next.widths.search = MANUAL_COLUMN_LIMITS.search.default;
    next.collapsed.search = false;
  } else {
    next.widths.results = MANUAL_COLUMN_LIMITS.results.default;
    next.collapsed.results = false;
  }
  return next;
}

export function collapseManualColumn(state: ManualColumnState, column: ManualColumn): ManualColumnState {
  if (column === "map") return state; // 地图不提供折叠为竖条
  return {
    ...state,
    collapsed: { ...state.collapsed, [column]: true, map: false },
  };
}

export function expandManualColumn(state: ManualColumnState, column: ManualColumn): ManualColumnState {
  if (column === "map") return state;
  return {
    ...state,
    collapsed: { ...state.collapsed, [column]: false, map: false },
  };
}

function finiteWidth(value: unknown, fallback: number, min: number, max: number): number {
  return typeof value === "number" && Number.isFinite(value) && value >= min && value <= max
    ? clamp(value, min, max)
    : fallback;
}

export function parseManualColumnState(value: string | null): ManualColumnState {
  if (!value) return structuredClone(DEFAULT_MANUAL_COLUMNS);
  try {
    const parsed: unknown = JSON.parse(value);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return structuredClone(DEFAULT_MANUAL_COLUMNS);
    }
    const collapsed = (parsed as { collapsed?: unknown }).collapsed;
    if (!collapsed || typeof collapsed !== "object" || Array.isArray(collapsed)) {
      return structuredClone(DEFAULT_MANUAL_COLUMNS);
    }
    const record = collapsed as Record<string, unknown>;
    if (record.search === true && record.map === true && record.results === true) return structuredClone(DEFAULT_MANUAL_COLUMNS);
    const state: ManualColumnState = {
      collapsed: {
        search: record.search === true,
        map: false,
        results: record.results === true,
      },
      widths: { ...DEFAULT_MANUAL_COLUMNS.widths },
    };
    const widths = (parsed as { widths?: unknown }).widths;
    if (widths && typeof widths === "object" && !Array.isArray(widths)) {
      const record = widths as Record<string, unknown>;
      state.widths.search = finiteWidth(record.search, 320, 280, 480);
      state.widths.results = finiteWidth(record.results, 440, 340, Number.MAX_SAFE_INTEGER);
    }
    return state;
  } catch {
    return structuredClone(DEFAULT_MANUAL_COLUMNS);
  }
}

export function readManualColumnState(): ManualColumnState {
  try {
    return parseManualColumnState(
      typeof window !== "undefined" ? window.localStorage?.getItem(MANUAL_COLUMNS_STORAGE_KEY) : null
    );
  } catch {
    return structuredClone(DEFAULT_MANUAL_COLUMNS);
  }
}

export function writeManualColumnState(state: ManualColumnState): void {
  try {
    if (typeof window === "undefined" || !window.localStorage) return;
    window.localStorage.setItem(MANUAL_COLUMNS_STORAGE_KEY, JSON.stringify(state));
  } catch {
    // Local storage is best-effort.
  }
}
