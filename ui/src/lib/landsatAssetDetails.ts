import { getCurrentLocale, t } from "../i18n/index.ts";
import { MESSAGES } from "../i18n/locales.ts";
import type { DesktopLandsatAssetDetails, DesktopLandsatAssetDetailRole } from "./desktopShellBridge.ts";

export type LandsatAssetDetailsState =
  | { status: "loading"; productKey: string; batchId: string; requestId: string }
  | { status: "available"; productKey: string; batchId: string; details: DesktopLandsatAssetDetails }
  | { status: "error"; productKey: string; batchId: string; error: string }
  | null;

export function landsatAssetDisplayName(assetKey: string): string {
  const labels = MESSAGES[getCurrentLocale()].shared.landsatDetails.labels as Readonly<Record<string, string>>;
  return Object.prototype.hasOwnProperty.call(labels, assetKey) ? labels[assetKey] : assetKey;
}

export function landsatAssetRoleLabel(role: DesktopLandsatAssetDetailRole): string {
  if (role === "primary") return t("shared.landsatDetails.roles.primary");
  if (role === "metadata") return t("shared.landsatDetails.roles.metadata");
  return t("shared.landsatDetails.roles.auxiliary");
}

export class LandsatAssetDetailsCoordinator {
  private generation = 0;
  private cancelCurrent: (() => void) | null = null;
  private readonly publish: (state: LandsatAssetDetailsState) => void;

  constructor(publish: (state: LandsatAssetDetailsState) => void) {
    this.publish = publish;
  }

  async request(
    input: { productKey: string; batchId: string; requestId: string },
    load: () => Promise<DesktopLandsatAssetDetails>,
    cancel: () => void,
  ): Promise<void> {
    const generation = ++this.generation;
    this.cancelCurrent?.();
    this.cancelCurrent = cancel;
    this.publish({ status: "loading", ...input });
    try {
      const details = await load();
      if (generation !== this.generation) return;
      if (details.product_key !== input.productKey) {
        throw new Error(t("shared.landsatDetails.identityMismatch"));
      }
      this.cancelCurrent = null;
      this.publish({ status: "available", productKey: input.productKey, batchId: input.batchId, details });
    } catch (error) {
      if (generation !== this.generation) return;
      this.cancelCurrent = null;
      this.publish({
        status: "error",
        productKey: input.productKey,
        batchId: input.batchId,
        error: error instanceof Error && error.message.trim() ? error.message : t("shared.landsatDetails.loadFailed"),
      });
    }
  }

  cancel(): void {
    ++this.generation;
    this.cancelCurrent?.();
    this.cancelCurrent = null;
    this.publish(null);
  }

  dispose(): void {
    this.cancel();
  }
}
