import React from "react";
import { useI18n } from "@/i18n";

export interface ChannelEditorTitleProps {
  isNewDraft: boolean;
  channelId: string;
  isDefault: boolean;
}

/** 渠道编辑页标题：单行显示，过长的渠道 id 截断并在悬停时给出完整值。 */
export function ChannelEditorTitle({ isNewDraft, channelId, isDefault }: ChannelEditorTitleProps) {
  const { t } = useI18n();
  return (
    <div className="flex min-w-0 flex-1 items-center gap-2 text-sm font-semibold text-sp-text">
      {isNewDraft ? (
        <span className="truncate">{t("settings.channels.new")}</span>
      ) : (
        <span className="flex min-w-0 items-center whitespace-nowrap">
          <span className="shrink-0">{t("settings.channels.editPrefix")}</span>
          <span className="min-w-0 truncate" title={channelId}>
            {channelId}
          </span>
        </span>
      )}
      {isDefault && !isNewDraft && (
        <span className="shrink-0 whitespace-nowrap rounded-full bg-sp-surface-muted text-sp-accent-strong px-2 py-1 text-xs font-normal">
          {t("settings.channels.currentDefault")}
        </span>
      )}
    </div>
  );
}
