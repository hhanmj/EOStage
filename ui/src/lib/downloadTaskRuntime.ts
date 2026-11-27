import { useEffect } from "react";

/** Production Workbench lifecycle boundary. A fresh effect setup is important
 * because StrictMode disposes and replays it during development. */
export type DownloadStatusLifecycle = {
  hydrate(): void | Promise<unknown>;
  dispose(): void;
  setTaskStates(states: readonly string[]): void;
};

/**
 * A task-creation command already returns the Store snapshot produced by the
 * same transaction. Publish that status directly so an independent restore
 * cannot race the just-created queued task. Older/non-Tauri callers retain the
 * bounded refresh fallback.
 */
export async function publishCreatedDownloadStatus<T>(
  result: { download_status?: T },
  publish: (status: T) => void,
  refresh: () => Promise<unknown>,
): Promise<boolean> {
  if (result.download_status !== undefined) {
    publish(result.download_status);
    return true;
  }
  await refresh();
  return false;
}

export function useDownloadStatusLifecycle(
  lifecycle: DownloadStatusLifecycle,
  taskStates: readonly string[],
) {
  useEffect(() => {
    void lifecycle.hydrate();
    return () => lifecycle.dispose();
  }, [lifecycle]);
  useEffect(() => {
    lifecycle.setTaskStates(taskStates);
  }, [lifecycle, taskStates]);
}
