import React from "react";
import { DesktopDownloadDrawer, type DesktopPendingDownloadPreparation, type TaskOutputRef } from "./DesktopDownloadDrawer";
import type { AccountId, ApplicationTask } from "@/shell/api/types";
import type { DownloadLayout } from "@/lib/generated/v3-contracts";

export interface DownloadCenterPageProps {
  tasks?: ApplicationTask[];
  onOpenFolder?: (task: TaskOutputRef) => void | Promise<void>;
  onTaskAction?: (
    taskId: string,
    revision: number,
    action: "pause" | "resume" | "retry" | "cancel"
  ) => Promise<void>;
  onHistoricalMutationApplied?: () => void | Promise<void>;
  newTaskPolicy?: { concurrency: number; layout: DownloadLayout } | null;
  pendingPreparations?: readonly DesktopPendingDownloadPreparation[];
  onDismissPreparation?: (id: string) => void;
  onConfigureAccount?: (accountId?: AccountId) => void;
}

export function DownloadCenterPage({
  tasks,
  onOpenFolder,
  onTaskAction,
  onHistoricalMutationApplied,
  newTaskPolicy,
  pendingPreparations,
  onDismissPreparation,
  onConfigureAccount,
}: DownloadCenterPageProps) {
  return (
    <div className="flex h-full w-full min-w-0 flex-1 overflow-hidden bg-sp-surface">
      <DesktopDownloadDrawer
        open={true}
        variant="page"
        tasks={tasks}
        onOpenFolder={onOpenFolder}
        onTaskAction={onTaskAction}
        onHistoricalMutationApplied={onHistoricalMutationApplied}
        newTaskPolicy={newTaskPolicy}
        pendingPreparations={pendingPreparations}
        onDismissPreparation={onDismissPreparation}
        onConfigureAccount={onConfigureAccount}
      />
    </div>
  );
}
