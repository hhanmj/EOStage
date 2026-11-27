import type {
  DesktopEffectiveCapabilityState,
  DesktopNisarRemoteSubsetCapability,
} from "./desktopShellBridge.ts";
import type { SarProduct } from "./generated/v3-contracts.ts";
import {
  bhoonidhiSsarSubsetSelection,
  isBhoonidhiSsarCatalogProduct,
} from "./bhoonidhiSsarCatalog.ts";

export type NisarDeliveryMethod = "complete_product" | "remote_derived_subset";
export type NisarSubsetSelection = { frequency: "A" | "B"; polarizations: string[] };

export function isNisarRslcProduct(product: SarProduct | null | undefined): boolean {
  return Boolean(
    product
      && product.processing_level === "RSLC"
      && (
        product.dataset === "sar.nisar.l.rslc.provisional.v1"
        || isBhoonidhiSsarCatalogProduct(product)
      ),
  );
}

function metadataText(product: SarProduct, key: string): string | null {
  const direct = product.native_metadata[key];
  if (typeof direct === "string") return direct;
  const properties = product.native_metadata.properties;
  if (typeof properties !== "object" || properties === null || Array.isArray(properties)) return null;
  const nested = (properties as Record<string, unknown>)[key];
  return typeof nested === "string" ? nested : null;
}

function productPolarizations(product: SarProduct, frequency: "A" | "B"): string[] {
  const label = metadataText(
    product,
    frequency === "A" ? "frequency_a_polarization" : "frequency_b_polarization",
  );
  if (!label) return [];
  return label.split("+").map((item) => item.trim()).filter(Boolean);
}

export function availableNisarSubsetSelections(
  capability: DesktopNisarRemoteSubsetCapability | null,
  product: SarProduct | null | undefined,
): NisarSubsetSelection[] {
  if (!capability?.visible || !isNisarRslcProduct(product)) return [];
  const sBandSelection = bhoonidhiSsarSubsetSelection(product);
  if (sBandSelection) return [sBandSelection];
  const selections: NisarSubsetSelection[] = [];
  for (const frequency of capability.supported_frequencies) {
    if (frequency !== "A" && frequency !== "B") continue;
    const providerValues = new Set(productPolarizations(product!, frequency));
    const polarizations = [...new Set((capability.supported_polarizations[frequency] ?? [])
      .map((value) => value.trim())
      .filter((value) => value && providerValues.has(value)))]
      .sort((left, right) => left.localeCompare(right));
    if (polarizations.length > 0) selections.push({ frequency, polarizations });
  }
  return selections;
}

export function canExecuteNisarSubset(
  capability: DesktopNisarRemoteSubsetCapability | null,
  selection: NisarSubsetSelection | null,
  available: readonly NisarSubsetSelection[],
  effectiveState: DesktopEffectiveCapabilityState | null = null,
): boolean {
  return Boolean(
    capability?.visible &&
      (effectiveState === null ? capability.enabled : effectiveState === "executable") &&
      selection &&
      selection.polarizations.length > 0 &&
      available.some((item) => {
        if (item.frequency !== selection.frequency) return false;
        const allowed = new Set(item.polarizations);
        const chosen = [...new Set(selection.polarizations)];
        return chosen.length > 0 && chosen.every((value) => allowed.has(value));
      }),
  );
}
