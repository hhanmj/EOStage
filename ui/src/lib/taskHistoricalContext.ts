export type TaskHistoricalSort = "source_order" | "name" | "acquisition_time";

export type TaskHistoricalUiState = {
  pageIndex: number;
  pageSize: number;
  filter: string;
  sort: TaskHistoricalSort;
  selectedIds: string[];
  activeProductKey: string | null;
};

export type TaskHistoricalUiCommand =
  | { kind: "set_page"; pageIndex: number }
  | { kind: "set_page_size"; pageSize: number }
  | { kind: "set_filter"; filter: string }
  | { kind: "set_sort"; sort: TaskHistoricalSort }
  | { kind: "toggle_item"; itemId: string }
  | { kind: "clear_selection" }
  | { kind: "set_active_product"; productKey: string | null };

export function createTaskHistoricalUiState(pageSize = 24): TaskHistoricalUiState {
  return {
    pageIndex: 0,
    pageSize: Math.min(200, Math.max(1, Math.trunc(pageSize) || 24)),
    filter: "",
    sort: "source_order",
    selectedIds: [],
    activeProductKey: null,
  };
}

export function updateTaskHistoricalUiState(
  state: TaskHistoricalUiState,
  command: TaskHistoricalUiCommand,
): TaskHistoricalUiState {
  switch (command.kind) {
    case "set_page":
      return { ...state, pageIndex: Math.max(0, Math.trunc(command.pageIndex) || 0) };
    case "set_page_size":
      return {
        ...state,
        pageIndex: 0,
        pageSize: Math.min(200, Math.max(1, Math.trunc(command.pageSize) || 1)),
      };
    case "set_filter":
      return { ...state, pageIndex: 0, filter: command.filter.slice(0, 240) };
    case "set_sort":
      return { ...state, pageIndex: 0, sort: command.sort };
    case "toggle_item": {
      const selected = new Set(state.selectedIds);
      if (selected.has(command.itemId)) selected.delete(command.itemId);
      else selected.add(command.itemId);
      return { ...state, selectedIds: Array.from(selected) };
    }
    case "clear_selection":
      return { ...state, selectedIds: [] };
    case "set_active_product":
      return { ...state, activeProductKey: command.productKey };
  }
}

export function taskHistoricalAction(
  status: string,
): "append" | "create_successor" | "unavailable" {
  const normalized = status.toLowerCase();
  if (["queued", "running", "paused"].includes(normalized)) return "append";
  if (["completed", "cancelled", "failed"].includes(normalized)) return "create_successor";
  return "unavailable";
}
