import type { ApplicationTask, TaskSceneItem } from "./types.ts";
import { t } from "../../i18n/index.ts";
import { updateApplicationDownloadTask } from "../../lib/bridge.ts";
import type { OrbitRowAction } from "./orbitChildRows.ts";

/** 单条目命令：task_id 属于配套任务，item_id 属于该任务自己的页投影。 */
export async function orbitItemAction(
  task: ApplicationTask,
  item: TaskSceneItem,
  action: OrbitRowAction,
): Promise<void> {
  if (!item.item_id) throw new Error(t("downloads.orbit.identifierUnavailable"));
  const prefix =
    action === "use_resorb"
      ? "orbit-resorb"
      : action === "upgrade_poeorb"
        ? "orbit-upgrade"
        : action === "check"
          ? "orbit-check"
          : "orbit-row";
  const result = await updateApplicationDownloadTask({
    task_id: task.task_id,
    expected_revision: task.revision,
    command: {
      kind: action === "cancel" ? "cancel_items" : "retry_items",
      item_ids: [item.item_id],
    },
    event_id: `${prefix}-${globalThis.crypto.randomUUID()}`,
    occurred_at: new Date().toISOString(),
  });
  if (!result.ok) {
    throw Object.assign(new Error(result.error), {
      code: result.code,
      message_zh: result.error,
    });
  }
}
