import { useI18n } from "@/i18n";
import type { ReactNode } from "react";

import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

export type CredentialRuntimeTone = "success" | "warning" | "neutral";

function StatusCell({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone: CredentialRuntimeTone;
}) {
  return (
    <div className="min-w-0 rounded-lg border bg-background px-2.5 py-2">
      <div className="mb-1 text-xs font-medium text-muted-foreground">
        {label}
      </div>
      <Badge variant={tone} className="max-w-full truncate">
        {value}
      </Badge>
    </div>
  );
}

/** Shared hierarchy for provider storage state, runtime availability, actions, and links. */
export function ProviderCredentialSurface({
  provider,
  saved,
  runtimeLabel,
  runtimeTone = "neutral",
  children,
  actions,
  feedback,
  resources,
  className,
}: {
  provider: string;
  saved: boolean;
  runtimeLabel: string;
  runtimeTone?: CredentialRuntimeTone;
  children: ReactNode;
  actions?: ReactNode;
  feedback?: ReactNode;
  resources?: ReactNode;
  className?: string;
}) {
  const { t } = useI18n();
  return (
    <div
      className={cn("flex h-full min-h-0 flex-col gap-3", className)}
      data-provider-credential-surface={provider}
    >
      <div className="grid grid-cols-2 gap-2 rounded-xl border bg-muted/20 p-2">
        <StatusCell label={t("accounts.surface.localCredentials")} value={saved ? t("accounts.saved") : t("accounts.surface.notSaved")} tone={saved ? "success" : "neutral"} />
        <StatusCell label={t("accounts.surface.runtime")} value={runtimeLabel} tone={runtimeTone} />
      </div>
      <div className="min-w-0">{children}</div>
      <div className="mt-auto space-y-2.5">
        {actions && <div>{actions}</div>}
        {feedback}
        {resources && (
          <div className="border-t pt-2.5" data-provider-credential-resources={provider}>
            {resources}
          </div>
        )}
      </div>
    </div>
  );
}
