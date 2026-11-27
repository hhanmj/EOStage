/**
 * 下载与任务中心抽屉 Esc 键响应决策纯逻辑
 *
 * 严格按照层级优先级决策：
 * 1. 若打开了两级重试兜底确认 -> 关兜底确认
 * 2. 若打开了清除历史确认 -> 关清除历史确认
 * 3. 若正在查看某任务详情 -> 返回任务列表 (关任务详情)
 * 4. 若无任何子对话框 -> 关闭抽屉本身
 */

export type DownloadDrawerEscapeAction =
  | "close_fallback"
  | "close_clear_history"
  | "close_task_detail"
  | "close_drawer";

export interface DownloadDrawerEscapeState {
  hasFallbackConfirm: boolean;
  hasClearHistoryConfirm: boolean;
  hasSelectedTaskDetail: boolean;
}

export function resolveDownloadDrawerEscapeAction(
  state: DownloadDrawerEscapeState
): DownloadDrawerEscapeAction {
  if (state.hasFallbackConfirm) {
    return "close_fallback";
  }
  if (state.hasClearHistoryConfirm) {
    return "close_clear_history";
  }
  if (state.hasSelectedTaskDetail) {
    return "close_task_detail";
  }
  return "close_drawer";
}
