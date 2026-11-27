export type PreparedQueueTask = {
  id: string;
  status: "pending" | "starting" | "accepted" | "failed";
  error?: string | null;
};

export function markQueueTaskStarting<T extends PreparedQueueTask>(
  tasks: T[],
  taskId: string,
): T[] {
  return tasks.map((task) =>
    task.id === taskId
      ? ({ ...task, status: "starting", error: null } as T)
      : task,
  );
}

export function markQueueTaskAccepted<T extends PreparedQueueTask>(
  tasks: T[],
  taskId: string,
): T[] {
  return tasks.map((task) =>
    task.id === taskId ? ({ ...task, status: "accepted", error: null } as T) : task,
  );
}

export function reconcilePreparedTasks<T extends PreparedQueueTask>(
  tasks: T[],
  liveTaskIds: ReadonlySet<string>,
): T[] {
  const next = tasks.filter(
    (task) => task.status === "failed" || !liveTaskIds.has(task.id),
  );
  return next.length === tasks.length ? tasks : next;
}
