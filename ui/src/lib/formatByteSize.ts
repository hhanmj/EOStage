/** Decimal SI sizes shared by catalogue, preview, and download progress. */
export function formatByteSize(value: number | null | undefined, unknownLabel = "—"): string {
  if (value == null || !Number.isFinite(value) || value < 0) return unknownLabel;
  if (value >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(1)} GB`;
  if (value >= 1_000_000) return `${Number((value / 1_000_000).toFixed(1))} MB`;
  if (value >= 1_000) return `${Number((value / 1_000).toFixed(1))} KB`;
  return `${Math.round(value)} B`;
}
