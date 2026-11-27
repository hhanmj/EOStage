import React, { useId, useState } from "react";
import { Loader2, RefreshCw, X } from "lucide-react";
import { useI18n } from "../../i18n/index.ts";
import { cn } from "../../lib/utils.ts";
import type { ApplicationTask, TaskSceneItem } from "../../shell/api/types.ts";
import type { OrbitRowAction } from "../../shell/api/orbitChildRows.ts";
import {
  formatOrbitCheckTime,
  getAvailableOrbitRowActions,
  isOrbitPublicationLate,
  orbitFileInfoLines,
  orbitRowStatus,
  resolvedOrbitType,
} from "../../shell/api/orbitChildRows.ts";

function OrbitFileInfo({ item }: { item: TaskSceneItem }) {
  const { t } = useI18n();
  const tooltipId = useId();
  const [pinned, setPinned] = useState(false);
  const [open, setOpen] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const visible = pinned || (open && !dismissed);
  const lines = orbitFileInfoLines(item);
  const isResorb = resolvedOrbitType(item) === "RESORB";
  const kindLabel = isResorb ? "AUX_RESORB" : "AUX_POEORB";
  return (
    <span
      className="group/orbit-info relative shrink-0"
      onMouseEnter={() => { if (!pinned) setOpen(true); }}
      onMouseLeave={() => { if (!pinned) setOpen(false); }}
      onFocus={() => { if (!pinned) { setDismissed(false); setOpen(true); } }}
      onBlur={() => { if (!pinned) setOpen(false); }}
    >
      <button
        type="button"
        aria-label={t(isResorb ? "downloads.orbit.resorbFileInfoAria" : "downloads.orbit.fileInfoAria")}
        aria-describedby={visible ? tooltipId : undefined}
        aria-expanded={visible}
        onClick={() => {
          if (pinned) {
            setPinned(false);
            setDismissed(true);
            setOpen(false);
          } else {
            setPinned(true);
            setDismissed(false);
            setOpen(true);
          }
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            setPinned(false);
            setDismissed(true);
            setOpen(false);
          }
        }}
        className="font-mono text-xs text-sp-text cursor-pointer rounded-sp-sm focus-visible:ring-1 focus-visible:ring-sp-accent"
      >
        {kindLabel}
      </button>
      {visible && <span
        id={tooltipId}
        role="tooltip"
        className="absolute left-0 top-full z-50 mt-1 block w-max max-w-[min(420px,calc(100vw-2rem))] whitespace-normal break-all rounded-sp-sm bg-sp-surface px-2 py-1.5 text-xs font-normal leading-relaxed text-sp-text shadow-sp-overlay"
      >
        {lines.map((line) => <span key={line} className="block">{line}</span>)}
      </span>}
    </span>
  );
}

export function OrbitChildRow({
  task,
  item,
  sourcePaused,
  indented = true,
  revisionChanged,
  action,
  error,
  onAction,
}: {
  task: ApplicationTask;
  item: TaskSceneItem;
  sourcePaused: boolean;
  indented?: boolean;
  revisionChanged: boolean;
  action: OrbitRowAction | null;
  error: string | null;
  onAction: (task: ApplicationTask, item: TaskSceneItem, action: OrbitRowAction) => void;
}) {
  const { t } = useI18n();
  const presentation = orbitRowStatus(item);
  const checkTime = formatOrbitCheckTime(item);
  const late = isOrbitPublicationLate(item.orbit_recheck);
  const actions = getAvailableOrbitRowActions(item, {
    sourcePaused,
    companionPaused: task.status === "paused",
    revisionChanged,
    inFlight: action !== null,
    loadFailed: false,
  });
  const textAction = actions.find(
    (entry) => entry.kind === "use_resorb" || entry.kind === "upgrade_poeorb"
  );
  const slots: Array<OrbitRowAction | null> = [
    actions.some((entry) => entry.kind === "check") ? "check" : null,
    actions.some((entry) => entry.kind === "cancel") ? "cancel" : null,
  ];
  return (
    <div role="listitem" className="space-y-0.5">
      <div
        aria-busy={action !== null}
        className={cn(
          "group/orbit-row flex h-8 items-center gap-2 rounded-sp-sm bg-sp-card py-1 text-xs transition-colors duration-fast ease-sp hover:bg-sp-hover",
          indented ? "ml-5 mr-2 pl-2 pr-1" : "px-2",
        )}
      >
        <OrbitFileInfo item={item} />
        <span className="min-w-0 flex-1 truncate text-sp-text-muted" title={checkTime ?? undefined}>
          {checkTime && <span className="whitespace-nowrap tabular-nums text-sp-text-secondary">{checkTime}</span>}
          {late && <span className="ml-2 text-sp-warning-text">{t("downloads.orbit.publicationLate")}</span>}
        </span>
        <span className={cn("flex h-5 shrink-0 items-center rounded-full px-2 text-xs", presentation.badgeClass)}>
          {presentation.label}
        </span>
        {textAction && (() => {
          const pending = action === textAction.kind;
          const label =
            textAction.kind === "use_resorb"
              ? t(pending ? "downloads.orbit.useResorbSubmitting" : "downloads.orbit.useResorb")
              : t(pending ? "downloads.orbit.upgradePoeorbSubmitting" : "downloads.orbit.upgradePoeorb");
          return (
            <button
              type="button"
              disabled={textAction.disabled}
              aria-label={label}
              title={label}
              onClick={() => onAction(task, item, textAction.kind)}
              className={cn(
                "flex h-6 shrink-0 items-center gap-1 rounded-sp-sm border border-sp-line px-1.5 text-xs font-medium text-sp-text-secondary transition-colors duration-fast ease-sp cursor-pointer hover:bg-sp-surface hover:text-sp-text focus-visible:ring-1 focus-visible:ring-sp-accent",
                "disabled:cursor-default disabled:text-sp-text-disabled disabled:hover:bg-transparent",
              )}
            >
              {pending && <Loader2 aria-hidden className="h-3 w-3 animate-spin" />}
              <span>{label}</span>
            </button>
          );
        })()}
        {slots.map((kind, index) => {
          const availability = kind ? actions.find((entry) => entry.kind === kind) : null;
          const pending = kind !== null && action === kind;
          const actionLabel = pending
            ? t(kind === "check" ? "downloads.orbit.checkSubmitting" : "downloads.orbit.cancelSubmitting")
            : t(kind === "check" ? "downloads.orbit.checkNow" : "downloads.orbit.cancel");
          return (
            <span key={kind ?? `empty-${index}`} className="flex h-7 w-7 shrink-0 items-center justify-center">
              {kind && availability && (
                <button
                  type="button"
                  disabled={availability.disabled}
                  aria-label={actionLabel}
                  title={actionLabel}
                  onClick={() => onAction(task, item, kind)}
                  className={cn(
                    "flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-text-secondary transition-colors duration-fast ease-sp cursor-pointer focus-visible:ring-1 focus-visible:ring-sp-accent",
                    "opacity-0 group-hover/orbit-row:opacity-100 focus:opacity-100 group-focus-within/orbit-row:opacity-100 [@media(hover:none)]:opacity-100",
                    kind === "check" && "text-sp-accent-strong hover:bg-sp-hover hover:text-sp-text",
                    kind === "cancel" && "hover:bg-sp-danger-bg hover:text-sp-danger-text",
                    "disabled:cursor-default disabled:text-sp-text-disabled disabled:hover:bg-transparent",
                  )}
                >
                  {pending ? (
                    <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" />
                  ) : kind === "check" ? (
                    <RefreshCw aria-hidden className="h-3.5 w-3.5" />
                  ) : (
                    <X aria-hidden className="h-3.5 w-3.5" />
                  )}
                </button>
              )}
            </span>
          );
        })}
      </div>
      {error && <div role="alert" className={cn("rounded-sp-sm bg-sp-danger-bg px-2 py-1 text-xs text-sp-danger-text", indented && "ml-5 mr-2")}>{error}</div>}
    </div>
  );
}

export function OrbitPageMessage({ failed, onRetry }: { failed: boolean; onRetry: () => void }) {
  const { t } = useI18n();
  return (
    <div role="listitem" className="flex h-8 items-center gap-2 rounded-sp-sm bg-sp-card px-2 text-xs text-sp-text-muted hover:bg-sp-hover">
      <span className="flex-1">{t(failed ? "downloads.orbit.detailsUnavailable" : "downloads.orbit.detailsLoading")}</span>
      {failed && (
        <button
          type="button"
          onClick={onRetry}
          aria-label={t("downloads.orbit.retryDetails")}
          title={t("downloads.orbit.retry")}
          className="flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-accent-strong hover:bg-sp-hover focus-visible:ring-1 focus-visible:ring-sp-accent"
        >
          <RefreshCw aria-hidden className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );
}
