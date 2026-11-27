import type { StoreTaskPageItem } from "../../lib/generated/v3-contracts.ts";
import type { TaskLifecycleStatus, TaskSceneItem } from "./types.ts";

type SceneProgressPageItem = Pick<
  StoreTaskPageItem,
  "status" | "bytes_completed" | "total_bytes" | "source_total_bytes" | "orbit_recheck" | "failure" | "destination"
> & {
  item: Pick<StoreTaskPageItem["item"], "item_id" | "display_name">;
};

export function sceneItemLifecycleStatus(status: string): TaskLifecycleStatus {
  switch (status) {
    case "active":
      return "running";
    case "paused":
      return "paused";
    case "completed":
    case "skipped":
      return "completed";
    case "failed":
      return "failed";
    case "cancelled":
      return "cancelled";
    case "interrupted":
      return "interrupted";
    case "queued":
    default:
      return "queued";
  }
}

export function taskPageToSceneItems(
  page: { items: SceneProgressPageItem[] },
): TaskSceneItem[] {
  return page.items.map((entry) => ({
    scene_id: entry.item.display_name.trim() || entry.item.item_id,
    status: sceneItemLifecycleStatus(entry.status),
    done_bytes: entry.bytes_completed ?? 0,
    total_bytes: entry.total_bytes ?? entry.source_total_bytes ?? null,
    ...(entry.orbit_recheck || entry.failure || entry.destination ? { item_id: entry.item.item_id } : {}),
    ...(entry.orbit_recheck ? { orbit_recheck: entry.orbit_recheck } : {}),
    ...(entry.failure ? { failure: entry.failure } : {}),
    ...(entry.destination ? { destination: entry.destination } : {}),
  }));
}
