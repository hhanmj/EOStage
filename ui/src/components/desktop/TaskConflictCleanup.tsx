import React, { useCallback, useEffect, useRef, useState } from "react";
import { Loader2, Trash2 } from "lucide-react";
import { useI18n } from "../../i18n/index.ts";
import { formatByteSize } from "../../lib/formatByteSize.ts";
import type { TaskConflictFileList } from "../../lib/generated/v3-contracts.ts";
import {
  deleteApplicationTaskConflictFiles,
  listApplicationTaskConflictFiles,
} from "../../lib/bridge.ts";
import {
  conflictCleanupErrorText,
  conflictDeleteSummary,
  conflictFileExpectations,
  conflictFilePreview,
  taskConflictCleanupIsActive,
} from "../../lib/taskConflictCleanup.ts";
import type { ApplicationTask } from "../../shell/api/types.ts";

interface TaskConflictCleanupProps {
  task: ApplicationTask;
  expanded: boolean;
}

export function TaskConflictCleanup({ task, expanded }: TaskConflictCleanupProps) {
  const { t } = useI18n();
  const taskId = task.task_id;
  const taskActive = taskConflictCleanupIsActive(task);
  const [listing, setListing] = useState<TaskConflictFileList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [result, setResult] = useState<{ deleted: number; skipped: number } | null>(null);
  const [retryVersion, setRetryVersion] = useState(0);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const cancelRef = useRef<HTMLButtonElement | null>(null);
  const latestTask = useRef(task);
  latestTask.current = task;
  const mountedRef = useRef(true);
  useEffect(() => () => { mountedRef.current = false; }, []);

  useEffect(() => {
    setListing(null);
    setError(null);
    setDeleteError(null);
    setConfirming(false);
    setDeleting(false);
    setResult(null);
    setRetryVersion(0);
  }, [taskId]);

  useEffect(() => {
    if (taskActive) {
      setListing(null);
      setConfirming(false);
      setError(null);
      setDeleteError(null);
    }
  }, [taskActive]);

  useEffect(() => {
    if (!expanded || taskActive) return;
    let cancelled = false;
    const load = async () => {
      try {
        const response = await listApplicationTaskConflictFiles({ task_id: taskId });
        if (cancelled || latestTask.current.task_id !== taskId) return;
        if (!response.ok || response.list.task_id !== taskId) {
          setListing(null);
          if (
            !response.ok &&
            (response.code === "APPLICATION_INVALID_REQUEST" ||
              response.code === "V3_APPLICATION_INVALID_REQUEST")
          ) {
            setError(null);
            return;
          }
          setError(conflictCleanupErrorText(response, t("downloads.conflictCleanup.loadFailed")));
          return;
        }
        setListing(response.list);
        setError(null);
      } catch (cause: unknown) {
        if (cancelled || latestTask.current.task_id !== taskId) return;
        setListing(null);
        setError(conflictCleanupErrorText(cause, t("downloads.conflictCleanup.loadFailed")));
      }
    };
    void load();
    return () => { cancelled = true; };
  }, [expanded, taskId, taskActive, retryVersion, t]);

  const closeConfirmation = useCallback(() => setConfirming(false), []);

  useEffect(() => {
    if (!confirming) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        closeConfirmation();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    const timer = window.setTimeout(() => cancelRef.current?.focus(), 0);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.clearTimeout(timer);
      triggerRef.current?.focus();
    };
  }, [confirming, closeConfirmation]);

  const confirmDelete = async () => {
    if (!listing || deleting || listing.files.length === 0) return;
    const expected = conflictFileExpectations(listing.files);
    const requestTaskId = taskId;
    setDeleting(true);
    setError(null);
    setDeleteError(null);
    setResult(null);
    try {
      const response = await deleteApplicationTaskConflictFiles({
        task_id: requestTaskId,
        expected,
      });
      if (!mountedRef.current || latestTask.current.task_id !== requestTaskId) return;
      if (!response.ok || response.result.task_id !== requestTaskId) {
        setDeleteError(conflictCleanupErrorText(response, t("downloads.conflictCleanup.deleteFailed")));
        return;
      }
      setResult(conflictDeleteSummary(response.result.outcomes));
      setConfirming(false);
      setRetryVersion((version) => version + 1);
    } catch (cause: unknown) {
      if (mountedRef.current && latestTask.current.task_id === requestTaskId) {
        setDeleteError(conflictCleanupErrorText(cause, t("downloads.conflictCleanup.deleteFailed")));
      }
    } finally {
      if (mountedRef.current && latestTask.current.task_id === requestTaskId) setDeleting(false);
    }
  };

  if (taskActive) return null;

  const visible = listing?.total_count ? conflictFilePreview(listing.files) : null;

  return (
    <>
      {error && <div role="alert" className="px-2 py-1 text-xs text-sp-danger-text">{error}</div>}
      {deleteError && !confirming && <div role="alert" className="px-2 py-1 text-xs text-sp-danger-text">{deleteError}</div>}
      {listing && listing.total_count > 0 && (
        <div className="flex h-7 items-center gap-2 px-2 text-xs text-sp-text-muted">
          <span className="min-w-0 flex-1 truncate">
            {t("downloads.conflictCleanup.summary", {
              amount: listing.total_count,
              size: formatByteSize(listing.total_size_bytes),
            })}
          </span>
          <button
            ref={triggerRef}
            type="button"
            disabled={listing.files.length === 0 || deleting}
            onClick={() => { setError(null); setConfirming(true); }}
            className="flex shrink-0 items-center gap-1 rounded-sp-sm px-1.5 py-0.5 text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text cursor-pointer disabled:cursor-not-allowed disabled:opacity-40 outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
          >
            {deleting
              ? <Loader2 className="h-3 w-3 animate-spin" aria-hidden="true" />
              : <Trash2 className="h-3 w-3" aria-hidden="true" />}
            {t(deleting ? "downloads.conflictCleanup.busy" : "downloads.conflictCleanup.clean")}
          </button>
        </div>
      )}
      {result && (
        <div role="status" className="px-2 py-1 text-xs text-sp-text-muted">
          <span>{t("downloads.conflictCleanup.deleted", { amount: result.deleted })}</span>
          {result.skipped > 0 && <span> · {t("downloads.conflictCleanup.skipped", { amount: result.skipped })}</span>}
        </div>
      )}
      {confirming && listing && visible && (
        <div
          role="dialog"
          aria-modal="true"
          aria-labelledby={`conflict-cleanup-title-${taskId}`}
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/20 p-4 animate-in fade-in duration-fast"
        >
          <div aria-hidden="true" className="absolute inset-0" onClick={deleting ? undefined : closeConfirmation} />
          <div className="relative w-full max-w-[470px] rounded-modal bg-sp-surface p-6 shadow-modal animate-in zoom-in-95 duration-fast">
            <h2 id={`conflict-cleanup-title-${taskId}`} className="text-base font-semibold text-sp-text">
              {t("downloads.conflictCleanup.title")}
            </h2>
            <div className="mt-5 overflow-hidden rounded-card bg-sp-card px-4 py-2">
              <ul className="divide-y divide-sp-divider">
                {visible.files.map((file) => (
                  <li key={file.path} className="flex items-center gap-3 py-2.5 text-xs">
                    <span className="min-w-0 flex-1 truncate font-mono text-sp-text" title={file.path}>{file.path}</span>
                    <span className="shrink-0 tabular-nums text-sp-text-muted">{formatByteSize(file.size_bytes)}</span>
                  </li>
                ))}
              </ul>
              {visible.hiddenCount > 0 && (
                <div className="py-2 text-xs text-sp-text-muted">
                  {t("downloads.conflictCleanup.more", { amount: visible.hiddenCount })}
                </div>
              )}
              <div className="border-t border-sp-divider py-2.5 text-xs font-medium text-sp-text-secondary">
                {t("downloads.conflictCleanup.total", { size: formatByteSize(listing.total_size_bytes) })}
              </div>
            </div>
            {deleteError && <div role="alert" className="mt-3 text-xs text-sp-danger-text">{deleteError}</div>}
            <div className="mt-6 grid grid-cols-2 gap-3">
              <button
                ref={cancelRef}
                type="button"
                disabled={deleting}
                onClick={closeConfirmation}
                className="h-9 rounded-sp-sm bg-sp-card px-4 text-sm text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer disabled:cursor-not-allowed disabled:opacity-40 outline-none focus-visible:ring-2 focus-visible:ring-sp-accent"
              >
                {t("downloads.conflictCleanup.cancel")}
              </button>
              <button
                type="button"
                disabled={deleting}
                onClick={() => void confirmDelete()}
                className="flex h-9 items-center justify-center gap-2 rounded-sp-sm bg-sp-danger-strong px-4 text-sm font-medium text-white hover:opacity-90 transition-opacity duration-fast ease-sp cursor-pointer disabled:cursor-not-allowed disabled:opacity-40 outline-none focus-visible:ring-2 focus-visible:ring-sp-accent focus-visible:ring-offset-2 focus-visible:ring-offset-sp-surface"
              >
                {deleting && <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />}
                {t(deleting ? "downloads.conflictCleanup.busy" : "downloads.conflictCleanup.delete")}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
