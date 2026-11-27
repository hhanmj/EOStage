import type { SarSourceCapability } from "./generated/v3-contracts.ts";

const PRESENTATION_ORDER: Readonly<Record<string, number>> = {
  NISAR: 0,
  "SENTINEL-1": 1,
  "ALOS-2": 2,
  "ALOS PALSAR": 3,
  "JERS-1": Number.MAX_SAFE_INTEGER,
};

const PRESENTATION_LABELS: Readonly<Record<string, string>> = {
  NISAR: "NISAR",
  "SENTINEL-1": "Sentinel-1",
  "SLC-BURST": "Sentinel-1 SLC Burst",
  "ALOS-2": "ALOS-2",
  "ALOS PALSAR": "ALOS PALSAR",
  ERS: "ERS-1 / ERS-2（ASF 已下线）",
  "JERS-1": "JERS-1（ASF 已下线）",
  "RADARSAT-1": "RADARSAT-1（ASF 已下线）",
  SEASAT: "SEASAT",
};

export function sarSourceDisplayLabel(source: SarSourceCapability): string {
  return PRESENTATION_LABELS[source.selector] ?? source.display_name ?? source.selector;
}

export function presentSarSources(
  sources: readonly SarSourceCapability[],
): SarSourceCapability[] {
  return sources
    .map((source, contractIndex) => ({ source, contractIndex }))
    .sort((left, right) => {
      const leftOrder = PRESENTATION_ORDER[left.source.selector] ?? 100 + left.contractIndex;
      const rightOrder = PRESENTATION_ORDER[right.source.selector] ?? 100 + right.contractIndex;
      return leftOrder - rightOrder || left.contractIndex - right.contractIndex;
    })
    .map(({ source }) => source);
}
