import type { DownloadBatchContext } from "./bridge";

type OrbitBatchContext = DownloadBatchContext & { query: Record<string, unknown> };

function queryRecord(context: DownloadBatchContext | null | undefined): Record<string, unknown> {
  const query = context?.query;
  return query && typeof query === "object" && !Array.isArray(query)
    ? query as Record<string, unknown>
    : {};
}

export function orbitBatchContext(
  context: DownloadBatchContext | null,
): OrbitBatchContext | null {
  if (!context) return null;
  return {
    ...context,
    batch_id: `orbit-${context.batch_id}`,
    query: {
      ...queryRecord(context),
      orbit_type: "POEORB",
      source_batch_id: context.batch_id,
    },
  };
}

export function orbitSnapshotAoiName(
  existing: string | null | undefined,
  batchContext: Pick<DownloadBatchContext, "aoi_name"> | null | undefined,
  current: string | null | undefined,
): string {
  return (
    existing?.trim()
    || batchContext?.aoi_name?.trim()
    || current?.trim()
    || "当前 AOI"
  );
}

export function orbitSnapshotAoiProvenance(
  batchContext: DownloadBatchContext | null | undefined,
): "随源 SAR 批次冻结" | "导入时冻结" | "当前 AOI" {
  if (!batchContext) return "当前 AOI";
  return queryRecord(batchContext).source === "orbit_scene_import"
    ? "导入时冻结"
    : "随源 SAR 批次冻结";
}
