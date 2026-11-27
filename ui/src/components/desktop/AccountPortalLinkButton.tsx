import React from "react";
import { ArrowUpRight } from "lucide-react";
import { useI18n } from "@/i18n";
import { openExternalUrl } from "@/lib/bridge";
import { formatBridgeErrorValue } from "@/lib/bridgeErrorFormat";
import { cn } from "@/lib/utils";
import {
  resolveAccountPortalLink,
  type AccountPortalTarget,
} from "@/shell/api/accountPortalLinks";

export interface AccountPortalLinkButtonProps {
  target: AccountPortalTarget | string;
  testId?: string;
  className?: string;
  onError?: (message: string) => void;
}

/**
 * 简约透明底官网/凭据获取直达图标按钮。
 * 采用与在线裁剪角标一致的右上斜向轻量线条视觉，默认全透明底色，悬停仅微弱浮现中性底纹。
 */
export function AccountPortalLinkButton({
  target,
  testId,
  className,
  onError,
}: AccountPortalLinkButtonProps) {
  const { t } = useI18n();
  const spec = resolveAccountPortalLink(target);
  if (!spec) return null;

  const label = spec.label;
  const portalUrl = spec.url;

  async function handleClick(event: React.MouseEvent<HTMLButtonElement>) {
    event.stopPropagation();
    try {
      const result = await openExternalUrl(portalUrl);
      if (!result.ok) {
        onError?.(formatBridgeErrorValue(result) || t("accounts.portalLinks.openFailed"));
      }
    } catch (error) {
      onError?.(formatBridgeErrorValue(error) || t("accounts.portalLinks.openFailed"));
    }
  }

  return (
    <button
      type="button"
      data-testid={testId ?? `account-portal-link-${spec.target}`}
      aria-label={label}
      title={`${label} (${spec.url})`}
      onClick={(event) => void handleClick(event)}
      className={cn(
        "inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-sp-sm bg-transparent text-[13px] text-sp-text-muted transition-colors hover:bg-sp-hover hover:text-sp-text focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sp-accent cursor-pointer",
        className,
      )}
    >
      <ArrowUpRight className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
    </button>
  );
}
