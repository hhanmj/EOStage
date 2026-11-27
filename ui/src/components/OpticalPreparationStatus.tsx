import { useEffect, useState } from "react";
import { useI18n } from "@/i18n";
import { getDesktopShellBridge } from "@/lib/desktopShellBridge";
import { observeOpticalPreparation, type OpticalPreparationProgress } from "@/lib/opticalPreparation";

export function OpticalPreparationStatus({ requestId, total }: { requestId: string; total: number }) {
  const { t } = useI18n();
  const [observation, setObservation] = useState<{ id: string; value: OpticalPreparationProgress | null; unavailable: boolean } | null>(null);
  useEffect(() => observeOpticalPreparation(requestId,
    id => getDesktopShellBridge().application.opticalPreparationProgress(id),
    (value, unavailable) => setObservation({ id: requestId, value, unavailable })), [requestId]);
  const current = observation?.id === requestId ? observation : null;
  const value = current?.value;
  const invalid = current?.unavailable || value && value.requested_count !== total;
  return <div role="status" aria-live="polite" className="mt-3 space-y-1 text-xs text-muted-foreground">
    {invalid ? <span>{t("panels.opticalPreparation.unavailable")}</span> : <>
      <span>{value?.phase === "committing" ? t("panels.opticalPreparation.committing") : t("panels.opticalPreparation.checked", { prepared: value?.prepared_count ?? 0, total })}
        {!!value?.rejected_count && t("panels.opticalPreparation.rejected", { rejected: value.rejected_count })}</span>
      <progress aria-label={t("panels.opticalPreparation.progress")} className="block h-1.5 w-full accent-primary" max={total} value={value?.prepared_count ?? 0} />
    </>}
  </div>;
}
