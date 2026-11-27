import React, { useState } from "react";
import { useI18n } from "@/i18n";
import { Trash2, Loader2 } from "lucide-react";

import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";

export interface ChannelDeleteControlProps {
  onConfirmDelete(): void;
  deleting?: boolean;
  disabled?: boolean;
  variant?: "default" | "menu-item";
}

export function ChannelDeleteControl({
  onConfirmDelete,
  deleting = false,
  disabled = false,
  variant = "default",
}: ChannelDeleteControlProps) {
  const { t } = useI18n();
  const [confirming, setConfirming] = useState<boolean>(false);
  const showDeleteSpinner = useDelayedFlag(deleting);
  const isBusy = Boolean(deleting || disabled);

  if (!confirming) {
    if (variant === "menu-item") {
      return (
        <button
          type="button"
          onClick={() => {
            if (isBusy) return;
            setConfirming(true);
          }}
          disabled={isBusy}
          aria-label={t("settings.channels.delete")}
          title={t("settings.channels.delete")}
          className="flex w-full items-center gap-2 rounded-sp-sm px-2 py-1 text-[13px] text-sp-danger-text hover:bg-sp-danger-bg disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer"
        >
          <span>{t("settings.channels.delete")}</span>
        </button>
      );
    }
    return (
      <button
        type="button"
        onClick={() => {
          if (isBusy) return;
          setConfirming(true);
        }}
        disabled={isBusy}
        aria-label={t("settings.channels.delete")}
        title={t("settings.channels.delete")}
        style={{ whiteSpace: "nowrap" }}
        className="inline-flex h-7 shrink-0 whitespace-nowrap items-center justify-center gap-1 rounded-sp-sm border border-sp-danger-text/25 bg-sp-surface px-3 text-[13px] font-semibold text-sp-danger-text transition-colors duration-fast ease-sp hover:bg-sp-danger-bg disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer"
      >
        <Trash2 className="h-3.5 w-3.5 shrink-0" />
        <span>{t("settings.channels.delete")}</span>
      </button>
    );
  }

  return (
    <div className="flex flex-wrap items-center justify-between gap-2 rounded-sp-sm bg-sp-danger-bg px-3 py-1.5 text-xs text-sp-danger-text">
      <span style={{ whiteSpace: "normal" }}>
        {t("settings.channels.deleteWarning")}
      </span>
      <div className="flex shrink-0 items-center gap-1.5">
        <button
          type="button"
          disabled={deleting}
          onClick={() => {
            if (deleting) return;
            setConfirming(false);
          }}
          aria-label={t("settings.channels.cancel")}
          title={t("settings.channels.cancel")}
          style={{ whiteSpace: "nowrap" }}
          className="flex h-7 shrink-0 whitespace-nowrap items-center px-3 rounded-sp-sm text-[13px] text-sp-text bg-sp-surface border border-sp-line hover:bg-sp-hover transition-colors duration-fast ease-sp disabled:opacity-50 cursor-pointer"
        >
          {t("settings.channels.cancel")}
        </button>
        <button
          type="button"
          disabled={isBusy}
          onClick={() => {
            if (isBusy) return;
            setConfirming(false);
            onConfirmDelete();
          }}
          aria-label={t("settings.channels.confirmDelete")}
          title={t("settings.channels.confirmDelete")}
          style={{ whiteSpace: "nowrap" }}
          aria-busy={deleting}
          className="flex h-7 min-w-[72px] shrink-0 whitespace-nowrap items-center justify-center gap-1.5 px-3 rounded-sp-sm text-[13px] font-semibold bg-sp-danger-strong text-white hover:opacity-90 transition-opacity duration-fast ease-sp disabled:opacity-50 cursor-pointer"
        >
          {showDeleteSpinner && <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />}
          <span>{t("settings.channels.confirmDelete")}</span>
        </button>
      </div>
    </div>
  );
}
