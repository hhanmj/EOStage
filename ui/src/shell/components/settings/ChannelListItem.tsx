import React from "react";
import { useI18n } from "@/i18n";
import { cn } from "../../../lib/utils.ts";
import type { AgentChannelView } from "../../api/agentBridgeTypes.ts";
import {
  getChannelHealth,
  resolveChannelHealthDotState,
  type ChannelHealthDotState,
} from "../../api/channelHealth.ts";
import { resolveChannelAuthKind, type ChannelAuthKind } from "../../api/agentChannelsLogic.ts";
import { ProviderBrandLogo } from "./ProviderBrandLogo.tsx";

export interface ChannelListItemProps {
  channel: AgentChannelView;
  isSelected: boolean;
  onSelect: (channel: AgentChannelView) => void;
  healthState?: ChannelHealthDotState;
  enabled?: boolean;
  authKind?: ChannelAuthKind;
}

export function ChannelListItem({
  channel,
  isSelected,
  onSelect,
  healthState,
  enabled = true,
  authKind,
}: ChannelListItemProps) {
  const { t } = useI18n();
  const resolvedAuthKind = authKind ?? resolveChannelAuthKind(channel);
  const resolvedHealth =
    healthState ?? resolveChannelHealthDotState(channel, getChannelHealth(channel.id), enabled);
  const keyTitle =
    resolvedHealth === "ok"
      ? t("settings.channels.healthReady")
      : resolvedHealth === "error"
        ? t("settings.channels.healthError")
        : resolvedHealth === "disabled"
          ? t("settings.channels.healthDisabled")
          : resolvedHealth === "missing_key"
            ? t("settings.channels.keyMissing")
            : t("settings.channels.healthUnverified");
  const dotColorClass =
    resolvedHealth === "ok"
      ? "bg-sp-status-success"
      : resolvedHealth === "error"
        ? "bg-sp-status-danger"
        : "bg-sp-text-disabled";
  const modelHint = [channel.model, ...(channel.models?.map((m) => m.id) ?? [])]
    .filter(Boolean)
    .join(" ");

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={t("settings.channels.channelLabel", { id: channel.id, default: channel.is_default ? t("settings.channels.defaultSuffix") : "" })}
      aria-pressed={isSelected}
      onClick={() => onSelect(channel)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect(channel);
        }
      }}
      className={cn(
        "eo-settings-channel-pill rounded-sp-sm h-9",
        isSelected && "eo-settings-channel-pill-active"
      )}
    >
      <ProviderBrandLogo
        provider={channel.id}
        baseUrl={channel.base_url}
        modelHint={modelHint}
        className="h-4 w-4 shrink-0"
      />

      {/* 渠道名称标识（单行截断，支持界面字体 break-all 兜底） */}
      <span
        title={channel.id}
        className="truncate break-all text-[13px] font-semibold text-sp-text flex-1 min-w-0"
      >
        {channel.id}
      </span>

      {/* 渠道计费/认证类型标识（API vs 订阅） */}
      {resolvedAuthKind !== "official" && (
        <span
          data-testid="channel-auth-kind-badge"
          data-auth-kind={resolvedAuthKind}
          className="eo-settings-badge shrink-0"
        >
          {resolvedAuthKind === "subscription"
            ? t("settings.channels.authKindSubscription")
            : t("settings.channels.authKindApi")}
        </span>
      )}

      {/* 默认渠道标识 */}
      {channel.is_default && (
        <span className="eo-settings-badge shrink-0">
          {t("settings.channels.default")}
        </span>
      )}

      {/* 3 态健康状态圆点（绿=已启用且就绪、红=失败/401、灰=停用/未配置/未登录） */}
      <span
        data-testid="channel-health-dot"
        data-health-state={resolvedHealth}
        title={keyTitle}
        className={cn(
          "h-2 w-2 rounded-full shrink-0",
          dotColorClass
        )}
      >
        <span className="sr-only">{keyTitle}</span>
      </span>
    </div>
  );
}

