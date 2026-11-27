import type {
  DesktopApplicationBridge,
  DesktopApplicationError,
  DesktopCoreRestoreSnapshot,
} from "./desktopShellBridge.ts";
import { projectCoreRestore, type CoreRestoreProjection } from "./coreRestoreProjection.ts";
import { LatestStatusCoordinator } from "./latestStatusCoordinator.ts";
import { RevisionGuard } from "./revisionGuard.ts";

export type ReadonlyApplicationServiceStatus =
  | {
      ok: true;
      readinessStatus: string;
      storeSchemaVersion: number | null;
      revisionCount: number;
    }
  | DesktopApplicationError;

type RevisionEntry = { stream: string; revision: number };

type CoordinatedRead = {
  status: ReadonlyApplicationServiceStatus;
  revisions: RevisionEntry[] | null;
};

type Options = {
  bridge: Pick<DesktopApplicationBridge, "coreRestore">;
  includeTerminal?: boolean;
  publish: (status: ReadonlyApplicationServiceStatus) => void;
  onError?: (error: unknown) => void;
};

function coreRevisions(core: CoreRestoreProjection): RevisionEntry[] {
  const entries: RevisionEntry[] = [];
  if (core.workbench !== null) {
    if (
      core.workbench.batch_id.trim() === ""
    ) {
      return entries;
    }
    entries.push({
      stream: `workbench:${core.workbench.batch_id}`,
      revision: core.workbench.revision,
    });
  }

  for (const task of core.tasks) {
    entries.push({ stream: `task:${task.task_id}`, revision: task.revision });
  }
  return entries;
}

function statusFor(
  snapshot: Extract<DesktopCoreRestoreSnapshot, { ok: true }>,
  revisionCount: number,
): ReadonlyApplicationServiceStatus {
  return {
    ok: true,
    readinessStatus: snapshot.readiness.status,
    storeSchemaVersion: snapshot.readiness.store_schema_version,
    revisionCount,
  };
}

/**
 * Presentation-only coordinator for the read-only Application Service route.
 * It never retains the CoreRestore payload: only revision numbers are kept in
 * the guard, and callers receive a bounded status projection.
 */
export class ReadonlyApplicationServiceCoordinator {
  private readonly guard = new RevisionGuard();
  private readonly coordinator: LatestStatusCoordinator<CoordinatedRead>;

  constructor(options: Options) {
    const includeTerminal = options.includeTerminal ?? false;
    this.coordinator = new LatestStatusCoordinator<CoordinatedRead>({
      read: async () => {
        const snapshot = await options.bridge.coreRestore(includeTerminal);
        if (!snapshot.ok) {
          return { status: snapshot, revisions: null };
        }
        const projected = projectCoreRestore(snapshot);
        if (!projected.ok) return { status: projected, revisions: null };
        const entries = coreRevisions(projected.core);
        return {
          status: statusFor(snapshot, entries.length),
          revisions: entries,
        };
      },
      accept: (value) =>
        value.revisions === null || this.guard.acceptSnapshotBatch(value.revisions),
      publish: (value) => options.publish(value.status),
      onError: options.onError,
    });
  }

  poll(): Promise<ReadonlyApplicationServiceStatus | null> | null {
    const request = this.coordinator.poll();
    return request?.then((value) => value?.status ?? null) ?? null;
  }

  async refresh(): Promise<ReadonlyApplicationServiceStatus | null> {
    const value = await this.coordinator.refresh();
    return value?.status ?? null;
  }

  /** Invalidate in-flight responses and permit the next restore revision. */
  reconnect(): void {
    this.guard.reset();
    this.coordinator.dispose();
    this.coordinator.activate();
  }

  dispose(): void {
    this.coordinator.dispose();
  }
}
