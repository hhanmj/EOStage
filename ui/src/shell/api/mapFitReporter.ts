/**
 * 地图已应用视图汇报通道 (MapFitReporter)
 *
 * 解决启动恢复 AOI 时 FitToData 触发 fitBounds 与 MapStatusOverlay 事件监听绑定之间的 React effect 顺序竞态。
 * 保持单例或可注入实例化，同时保留 getLatest() 确保挂载时能即时获取最新已应用视野。
 */

export interface MapFitView {
  center: { lat: number; lng: number };
  zoom: number;
  bbox?: { west: number; south: number; east: number; north: number } | null;
  timestamp?: number;
}

export type MapFitSubscriber = (view: MapFitView) => void;

export interface MapFitReporter {
  reportFit(view: MapFitView): void;
  subscribe(subscriber: MapFitSubscriber): () => void;
  getLatest(): MapFitView | null;
  clear(): void;
}

export function createMapFitReporter(): MapFitReporter {
  let latest: MapFitView | null = null;
  const subscribers = new Set<MapFitSubscriber>();

  return {
    reportFit(view: MapFitView) {
      latest = view;
      for (const subscriber of subscribers) {
        try {
          subscriber(view);
        } catch (err) {
          console.error("MapFitReporter subscriber error:", err);
        }
      }
    },
    subscribe(subscriber: MapFitSubscriber) {
      subscribers.add(subscriber);
      return () => {
        subscribers.delete(subscriber);
      };
    },
    getLatest() {
      return latest;
    },
    clear() {
      latest = null;
      subscribers.clear();
    },
  };
}

export const defaultMapFitReporter = createMapFitReporter();
