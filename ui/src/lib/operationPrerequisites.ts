export type CatalogSearchProvider = "asf" | "sentinel-2" | "landsat" | "catalogue";

export type CatalogSearchInput = {
  provider: CatalogSearchProvider;
  /** Product label for schema-described catalogue searches (Issue #3 R2 B5). */
  datasetLabel?: string;
  aoiGeojson: unknown | null;
  filtersValid: boolean;
};

export type CatalogSearchDecision =
  | {
      ok: true;
      request: {
        provider: CatalogSearchProvider;
        aoi_geojson: unknown | null;
      };
    }
  | {
      ok: false;
      code: "CATALOG_SEARCH_INVALID_INPUT" | "CATALOG_SEARCH_AOI_REQUIRED";
      message: string;
    };

export type DownloadStartDecision =
  | { ok: true; outputDirectory: string }
  | {
      ok: false;
      code: "DOWNLOAD_OUTPUT_REQUIRED" | "DOWNLOAD_SELECTION_REQUIRED";
      message: string;
    };

const PROVIDER_LABEL: Record<Exclude<CatalogSearchProvider, "asf">, string> = {
  "sentinel-2": "Sentinel-2",
  landsat: "Landsat",
  catalogue: "目录",
};

export function catalogSearchDecision(input: CatalogSearchInput): CatalogSearchDecision {
  if (!input.filtersValid) {
    return {
      ok: false,
      code: "CATALOG_SEARCH_INVALID_INPUT",
      message: "检索参数不完整，请检查数据集和必要筛选条件。",
    };
  }
  if (input.provider !== "asf" && !input.aoiGeojson) {
    return {
      ok: false,
      code: "CATALOG_SEARCH_AOI_REQUIRED",
      message: `${input.datasetLabel ?? PROVIDER_LABEL[input.provider]} 检索需要先设置有效 AOI。`,
    };
  }
  return {
    ok: true,
    request: {
      provider: input.provider,
      aoi_geojson: input.aoiGeojson,
    },
  };
}

export function downloadStartDecision(input: {
  outputDirectory: string | null | undefined;
  selectedProductCount: number;
}): DownloadStartDecision {
  const outputDirectory = input.outputDirectory?.trim() ?? "";
  if (!outputDirectory) {
    return {
      ok: false,
      code: "DOWNLOAD_OUTPUT_REQUIRED",
      message: "开始下载前请选择输出目录。",
    };
  }
  if (!Number.isInteger(input.selectedProductCount) || input.selectedProductCount < 1) {
    return {
      ok: false,
      code: "DOWNLOAD_SELECTION_REQUIRED",
      message: "开始下载前请选择至少一个产品。",
    };
  }
  return { ok: true, outputDirectory };
}
