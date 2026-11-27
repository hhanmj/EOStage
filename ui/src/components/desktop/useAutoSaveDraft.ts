import { useCallback, useEffect, useRef, useState } from "react";
import { formatBridgeErrorValue } from "@/lib/bridgeErrorFormat";

export const ACCOUNT_AUTO_SAVE_DELAY_MS = 600;

type SaveResult = { ok: boolean; error?: unknown };
type AutoSaveState =
  | { status: "idle" | "saving" | "saved"; error: null }
  | { status: "error"; error: string };

/** Save only the latest complete draft; stale responses never overwrite newer feedback. */
export function useAutoSaveDraft({
  draftKey,
  complete,
  save,
  onSaved,
}: {
  draftKey: string;
  complete: boolean;
  save: () => Promise<SaveResult>;
  onSaved?: () => void | Promise<void>;
}) {
  const [state, setState] = useState<AutoSaveState>({ status: "idle", error: null });
  const latest = useRef({ draftKey, complete, save, onSaved });
  latest.current = { draftKey, complete, save, onSaved };
  const generation = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inFlightDraft = useRef<string | null>(null);
  const lastSavedDraft = useRef<string | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (timer.current) clearTimeout(timer.current);
    };
  }, []);

  const saveNow = useCallback(async () => {
    const current = latest.current;
    if (!current.complete || inFlightDraft.current === current.draftKey || lastSavedDraft.current === current.draftKey) return;
    if (timer.current) clearTimeout(timer.current);
    const ticket = ++generation.current;
    inFlightDraft.current = current.draftKey;
    if (mounted.current) setState({ status: "saving", error: null });
    try {
      const result = await current.save();
      if (!mounted.current || generation.current !== ticket) return;
      if (!result.ok) {
        setState({ status: "error", error: formatBridgeErrorValue(result) });
        return;
      }
      lastSavedDraft.current = current.draftKey;
      setState({ status: "saved", error: null });
      await current.onSaved?.();
    } catch (error) {
      if (mounted.current && generation.current === ticket) {
        setState({ status: "error", error: formatBridgeErrorValue(error) });
      }
    } finally {
      if (inFlightDraft.current === current.draftKey) inFlightDraft.current = null;
    }
  }, []);

  useEffect(() => {
    if (inFlightDraft.current === draftKey || lastSavedDraft.current === draftKey) return;
    generation.current += 1;
    lastSavedDraft.current = null;
    setState({ status: "idle", error: null });
    if (complete) timer.current = setTimeout(() => void saveNow(), ACCOUNT_AUTO_SAVE_DELAY_MS);
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [draftKey, complete, saveNow]);

  const reset = useCallback(() => {
    generation.current += 1;
    lastSavedDraft.current = null;
    if (timer.current) clearTimeout(timer.current);
    setState({ status: "idle", error: null });
  }, []);

  return { state, saveNow, reset };
}
