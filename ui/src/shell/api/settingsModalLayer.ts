/**
 * 偏好设置弹窗是否打开（跨组件的 Esc 层级判断）。
 * 地图浮层在 window 捕获阶段处理 Esc 并 stopPropagation，下载抽屉在冒泡阶段先于弹窗注册；
 * 弹窗打开时这些下层处理必须让位，否则 Esc 关的是下层而不是弹窗。
 */
let openCount = 0;

/** 标记弹窗已打开，返回释放函数（多次调用释放只生效一次）。 */
export function markSettingsModalOpen(): () => void {
  openCount += 1;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    openCount = Math.max(0, openCount - 1);
  };
}

export function isSettingsModalOpen(): boolean {
  return openCount > 0;
}
