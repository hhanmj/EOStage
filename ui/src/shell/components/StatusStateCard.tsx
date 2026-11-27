import React from "react";
import { Loader2, type LucideIcon } from "lucide-react";
import { cn } from "../../lib/utils.ts";
import {
  getStatusStateConfig,
  type StateVariant,
} from "../api/statusStateLogic.ts";
import { EmptyStateIcon, type EmptyStateIconName } from "@/components/ui/EmptyStateVisuals";

export interface StatusStateAction {
  label: string;
  onClick: () => void;
  icon?: LucideIcon;
  variant?: "primary" | "secondary" | "link";
  disabled?: boolean;
}

export interface StatusStateCardProps {
  variant: StateVariant;
  title?: string;
  description?: string;
  action?: StatusStateAction;
  icon?: LucideIcon;
  className?: string;
  compact?: boolean;
}

const VARIANT_EMPTY_ICON: Record<Exclude<StateVariant, "loading">, EmptyStateIconName> = {
  empty: "folder",
  error: "alert",
  not_connected: "plug",
  login_required: "lock",
};

export const StatusStateCard: React.FC<StatusStateCardProps> = ({
  variant,
  title,
  description,
  action,
  icon: CustomIcon,
  className,
  compact = false,
}) => {
  const meta = getStatusStateConfig(variant);

  const displayTitle = title || meta.title;
  const displayDescription = description !== undefined ? description : meta.description;
  const isNeutral = variant === "not_connected" || variant === "login_required";

  return (
    <div
      className={cn(
        "sp-empty-enter flex flex-col items-center justify-center text-center select-none",
        compact ? "p-3 space-y-2" : "p-6 space-y-3",
        className
      )}
    >
      {CustomIcon ? (
        <div
          className={cn(
            "flex items-center justify-center rounded-full border border-sp-line bg-sp-surface-muted",
            compact ? "h-8 w-8 text-sp-text-muted" : "h-11 w-11 text-sp-text-muted",
            variant === "loading" && "text-sp-accent",
            variant === "error" && "text-sp-danger-strong bg-sp-danger-bg border-sp-line",
            variant === "login_required" && "text-sp-accent-strong bg-sp-selected"
          )}
        >
          <CustomIcon
            className={cn(
              compact ? "h-4 w-4" : "h-5 w-5",
              variant === "loading" && "animate-spin"
            )}
          />
        </div>
      ) : variant === "loading" ? (
        <div
          className={cn(
            "flex items-center justify-center rounded-full border border-sp-line bg-sp-surface-muted text-sp-accent",
            compact ? "h-8 w-8" : "h-11 w-11"
          )}
        >
          <Loader2 className={cn("animate-spin", compact ? "h-4 w-4" : "h-5 w-5")} />
        </div>
      ) : (
        <div className="flex shrink-0 items-center justify-center">
          <EmptyStateIcon
            name={VARIANT_EMPTY_ICON[variant]}
            size={compact ? 22 : 48}
            neutral={isNeutral}
          />
        </div>
      )}

      <div className="space-y-1 max-w-sm">
        <div
          className={cn(
            "font-semibold text-sp-text tracking-tight",
            compact ? "text-xs" : "text-sm"
          )}
        >
          {displayTitle}
        </div>
        {displayDescription && (
          <p
            className={cn(
              "text-sp-text-muted leading-relaxed",
              "text-xs"
            )}
          >
            {displayDescription}
          </p>
        )}
      </div>

      {action && (
        <div className="pt-1">
          {action.variant === "link" ? (
            <button
              type="button"
              onClick={action.onClick}
              disabled={action.disabled}
              className="inline-flex items-center gap-1 text-[13px] font-semibold text-sp-accent-strong hover:underline cursor-pointer disabled:opacity-50"
            >
              {action.icon && React.createElement(action.icon, { className: "h-3.5 w-3.5" })}
              <span>{action.label}</span>
            </button>
          ) : action.variant === "primary" ? (
            <button
              type="button"
              onClick={action.onClick}
              disabled={action.disabled}
              className="inline-flex h-sp-control items-center gap-2 rounded-sp-sm bg-sp-accent-fill px-4 text-[13px] font-semibold text-white shadow-card enabled:hover:opacity-90 disabled:opacity-50 transition-colors duration-fast ease-sp cursor-pointer"
            >
              {action.icon && React.createElement(action.icon, { className: "h-3.5 w-3.5" })}
              <span>{action.label}</span>
            </button>
          ) : (
            <button
              type="button"
              onClick={action.onClick}
              disabled={action.disabled}
              className="cred-action-btn transition-colors duration-fast ease-sp cursor-pointer"
            >
              {action.icon && React.createElement(action.icon, { className: "h-3.5 w-3.5" })}
              <span>{action.label}</span>
            </button>
          )}
        </div>
      )}
    </div>
  );
};
