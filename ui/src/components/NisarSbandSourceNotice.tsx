import type { DesktopJsonObject } from "@/lib/desktopShellBridge";
import {
  BHOONIDHI_COLLECTION_ID,
  BHOONIDHI_PROFILE_ID,
  BHOONIDHI_PROVIDER_ID,
} from "@/lib/bhoonidhiSsarCatalog";
import { Badge } from "@/components/ui/badge";

export function isSbandSubsetIntent(intent: DesktopJsonObject): boolean {
  const source = intent.source_product_identity;
  if (!source || typeof source !== "object" || Array.isArray(source)) return false;
  const identity = source as Record<string, unknown>;
  return identity.provider_id === BHOONIDHI_PROVIDER_ID
    && identity.collection_id === BHOONIDHI_COLLECTION_ID
    && identity.collection_profile_id === BHOONIDHI_PROFILE_ID;
}

export function NisarSbandSourceNotice() {
  return (
    <div
      role="note"
      aria-label="S-Band 已知差异"
      className="m-3 flex shrink-0 items-start gap-2 rounded-lg border border-primary/20 bg-primary/5 px-3 py-2.5 text-xs text-foreground"
    >
      <Badge variant="neutral" className="mt-0.5 shrink-0">S-band · Bhoonidhi</Badge>
      <p className="min-w-0 leading-5">
        约 9.6 cm 斜距参考差异；保持源数据约定。
      </p>
    </div>
  );
}
