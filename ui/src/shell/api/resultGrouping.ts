import type { SceneRow } from "../../lib/bridge.ts";
import { describeSceneStack, stackKey } from "./mapLinkLogic.ts";

export interface SceneStackGroup {
  key: string;
  label: string;
  shortTag: string;
  items: SceneRow[];
}

export interface GroupScenesOptions {
  sortByStackSize?: boolean;
}

/**
 * 计算单景影像所属同幅归组键（与 `groupScenesByStack` 完全一致）。
 */
export function sceneStackGroupKey(scene: SceneRow): string {
  const computed = stackKey(scene);
  if (computed) return computed;
  const { groupLabel } = describeSceneStack(scene);
  return `other:${groupLabel || scene.scene_id}`;
}

/**
 * 按同轨同幅（Path-Frame）、光学 MGRS 图幅或 Landsat WRS 条带将结果聚合成可折叠分组。
 * 地图工作台与 Agent 地图分屏清单共用此唯一真源逻辑。
 */
export function groupScenesByStack(
  scenes: readonly SceneRow[],
  options: GroupScenesOptions = {}
): SceneStackGroup[] {
  const { sortByStackSize = false } = options;
  const groups = new Map<string, SceneStackGroup>();

  for (const scene of scenes) {
    const desc = describeSceneStack(scene);
    const computed = stackKey(scene);
    const key = computed ?? `other:${desc.groupLabel || scene.scene_id}`;
    const existing = groups.get(key);
    if (existing) {
      existing.items.push(scene);
    } else {
      groups.set(key, {
        key,
        label: desc.groupLabel,
        shortTag: desc.shortTag,
        items: [scene],
      });
    }
  }

  const list = Array.from(groups.values());
  if (sortByStackSize) {
    list.sort((a, b) => b.items.length - a.items.length);
  }
  return list;
}
