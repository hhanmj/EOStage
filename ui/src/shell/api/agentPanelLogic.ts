/**
 * agentPanelLogic.ts
 * Agent 右侧面板纯函数逻辑模块（开合裁决、模式判断、页签守卫、宽度计算）
 * 规范：W5-2c
 */

export type PanelTab = "map" | "results" | "task";
export type PanelMode = "docked" | "overlay";
export type PanelOpenIntent = "map-view" | "rect-draw" | null;

export interface ResolvePanelOpenParams {
  requested: boolean;
  viewportWidth: number;
  openIntent: PanelOpenIntent;
}

/**
 * 裁决面板是否处于打开状态：
 * - 默认关闭
 * - map-view 或 rect-draw 意图强制打开
 * - requested 为 true 时打开
 */
export function resolvePanelOpen(params: ResolvePanelOpenParams): boolean {
  if (params.openIntent === "map-view" || params.openIntent === "rect-draw") {
    return true;
  }
  return Boolean(params.requested);
}

/**
 * 裁决面板展示模式：>=1280 为并列停靠(docked)，<1280 为右侧浮层(overlay)
 */
export function resolvePanelMode(viewportWidth: number): PanelMode {
  return viewportWidth >= 1280 ? "docked" : "overlay";
}

/**
 * 守卫面板当前选中的页签值（未知值回退为 map）
 */
export function resolvePanelTab(
  activeTab: unknown,
  _payloadCount?: number
): PanelTab {
  if (activeTab === "results" || activeTab === "task") {
    return activeTab;
  }
  return "map";
}

/**
 * 计算 docked 模式下的停靠面板宽度：
 * - >= 1280: Math.min(Math.round(viewportWidth * 0.42), 760)
 * - < 1280: 返回 null（由 overlay 决定固定宽度）
 */
export function panelWidth(viewportWidth: number): number | null {
  if (viewportWidth < 1280) {
    return null;
  }
  return Math.min(Math.round(viewportWidth * 0.42), 760);
}

/**
 * 裁决与限制 docked 模式下的停靠面板宽度：
 * - 规则：panel >= 360px，chat column >= 420px
 * - 可用总宽度为 viewportWidth - (sidebarWidth ?? 0)
 * - 最大允许 panel 宽度为 available - 420 (且不低于 360)
 * - 最小允许 panel 宽度为 360
 */
export function clampPanelWidth(
  desired: number,
  viewportWidth: number,
  sidebarWidth?: number
): number {
  const sidebar = sidebarWidth ?? 0;
  const available = viewportWidth - sidebar;
  const minPanel = 360;
  const maxPanel = Math.max(minPanel, available - 420);

  if (Number.isNaN(desired) || !Number.isFinite(desired)) {
    return minPanel;
  }
  if (desired < minPanel) return minPanel;
  if (desired > maxPanel) return maxPanel;
  return Math.round(desired);
}
