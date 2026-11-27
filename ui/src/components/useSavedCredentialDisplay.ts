import { t } from "@/i18n";
import { useEffect, useRef, useState } from "react";
import type { DesktopCredentialField, DesktopCredentialRevealResult } from "@/lib/desktopShellBridge";

export type SavedCredentialBinding = {
  identity: unknown;
  epoch?: number;
  field: DesktopCredentialField;
  onReveal: (field: DesktopCredentialField) => Promise<DesktopCredentialRevealResult>;
};

type Display = {
  identity: unknown;
  field: DesktopCredentialField;
  epoch?: number;
  shown: boolean;
  pending: boolean;
  value: string;
  error: string | null;
};

/** One disposable reveal lifecycle for read-only and editable credential fields. */
export function useSavedCredentialDisplay(config: SavedCredentialBinding | undefined, enabled = true) {
  const generation = useRef(0);
  const [display, setDisplay] = useState<Display | null>(null);
  const active = enabled && config !== undefined;
  const identity = config?.identity;
  const field = config?.field;
  const epoch = config?.epoch;

  useEffect(() => {
    generation.current++;
    setDisplay(null);
    return () => { generation.current++; };
  }, [active, identity, field, epoch]);

  // Reject old ownership in render as well as in the async completion guard.
  const currentDisplay = active && display?.identity === identity
    && display?.field === field && display?.epoch === epoch ? display : null;

  function hide() {
    generation.current++;
    setDisplay(null);
  }

  async function toggle() {
    if (!active || !config) return;
    if (currentDisplay?.shown || currentDisplay?.pending) { hide(); return; }
    const current = ++generation.current;
    const owner = { identity: config.identity, field: config.field, epoch: config.epoch };
    setDisplay({ ...owner, shown: true, pending: true, value: "", error: null });
    try {
      const result = await config.onReveal(config.field);
      if (generation.current !== current) return;
      const error = !result.ok ? t("accounts.fields.readFailed") : result.value === null
        ? t("accounts.fields.externalSourceNotStored") : null;
      setDisplay({ ...owner, shown: error === null, pending: false,
        value: result.ok && error === null ? result.value ?? "" : "", error });
    } catch {
      if (generation.current === current) {
        setDisplay({ ...owner, shown: false, pending: false, value: "",
          error: t("accounts.fields.readFailed") });
      }
    }
  }

  return {
    shown: currentDisplay?.shown ?? false,
    pending: currentDisplay?.pending ?? false,
    value: currentDisplay?.shown ? currentDisplay.value : "",
    error: currentDisplay?.error ?? null,
    hide,
    toggle,
  };
}
