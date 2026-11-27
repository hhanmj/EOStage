/**
 * 地图面板布局与交互纯逻辑（W5-3d）
 */

export type MapPanelMode =
  | "docked-wide"
  | "docked"
  | "collapsed"
  | "overlay"
  | "hidden";

/**
 * 根据视口宽度、用户折叠状态与浮层展开状态计算地图面板所处的模式：
 * - viewportWidth >= 1600: 用户未折叠为 "docked-wide"，折叠为 "collapsed"
 * - 1280 <= viewportWidth < 1600: 用户未折叠为 "docked"，折叠为 "collapsed"
 * - viewportWidth < 1280: 浮层展开为 "overlay"，未展开为 "hidden"
 */
export function mapPanelMode(
  viewportWidth: number,
  userCollapsed: boolean,
  overlayOpen: boolean
): MapPanelMode {
  if (viewportWidth >= 1280) {
    if (userCollapsed) {
      return "collapsed";
    }
    return viewportWidth >= 1600 ? "docked-wide" : "docked";
  } else {
    return overlayOpen ? "overlay" : "hidden";
  }
}

/**
 * 判定汇总栏是否展示「地图」切换按钮：
 * 仅在 "collapsed"（宽屏折叠）和 "hidden"（窄屏未打开浮层）模式下展示
 */
export function showMapButton(mode: MapPanelMode): boolean {
  return mode === "collapsed" || mode === "hidden";
}

/**
 * 按 Esc 键时浮层的下一个状态（关闭）
 */
export function nextOverlayStateOnEscape(_open: boolean): boolean {
  return false;
}

/**
 * 判定地图列是否应当在工作台主栅格中停靠挂载渲染。
 * 依托 resolveManualColumnLayout 的响应式伸缩能力（支持 920px~2560px），
 * 只要地图列未主动处于折叠态，主栅格地图即保持挂载停靠，杜绝视口缩小时出现纯白屏。
 */
export function shouldMountDockedMap(mapCollapsed: boolean): boolean {
  return !mapCollapsed;
}
