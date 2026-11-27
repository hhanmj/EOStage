import { useEffect, useRef } from "react";
import { TaskPageRequestCoordinator } from "./taskPageRequestCoordinator.ts";

/** The production Workbench owns exactly one bounded task-page controller for
 * open, page navigation, close, switch, and unmount invalidation. */
export function useTaskPageController(): TaskPageRequestCoordinator {
  const controller = useRef<TaskPageRequestCoordinator | null>(null);
  if (controller.current === null) controller.current = new TaskPageRequestCoordinator();
  useEffect(() => () => controller.current?.invalidate(), []);
  return controller.current;
}
