import { t } from "../i18n/index.ts";
import type { DatasetCapabilities, SarProduct } from "./generated/v3-contracts";

export type BurstAssetPolicy = "all" | "primary" | "metadata";

export type LegacySceneRow = {
  product_key?: string;
  scene_id: string;
  platform: string;
  product_type: string;
  beam_mode: string;
  polarization: string;
  acquisition_datetime: string;
  absolute_orbit: number | null;
  relative_orbit: number | null;
  path: number | null;
  frame: number | null;
  orbit_direction: string;
  file_size_remote?: number | null;
  footprint_bbox?: {
    west: number;
    east: number;
    south: number;
    north: number;
    crs?: string;
  } | null;
  footprint_geojson?: Record<string, unknown> | null;
  has_url: boolean;
  download_url?: string;
};

export type SarSceneProjection = LegacySceneRow & {
  product_key: string;
  product_id: string;
  provider: string;
  dataset: string;
  instrument: string | null;
  processing_level: string | null;
  product_family: string;
  file_name: string | null;
  native_metadata: Record<string, unknown>;
  sar_product: SarProduct;
};

function escapeProductKeyPart(value: string): string {
  return value.replaceAll("%", "%25").replaceAll("::", "%3A%3A");
}

export function canonicalProductKey(
  provider: string,
  dataset: string,
  productId: string,
): string {
  const normalizedProvider = provider.trim().toUpperCase();
  const normalizedDataset = dataset.trim().toUpperCase();
  const normalizedProductId = productId.trim();
  if (!normalizedProvider || !normalizedDataset || !normalizedProductId) {
    throw new Error("provider, dataset and productId are required for product_key");
  }
  return [normalizedProvider, normalizedDataset, normalizedProductId]
    .map(escapeProductKeyPart)
    .join("::");
}

export function productKey(product: SarProduct): string {
  const key = product.product_key.trim();
  if (!key) throw new Error("SAR product is missing product_key");
  return key;
}

export function mergeProducts(...groups: SarProduct[][]): SarProduct[] {
  const merged = new Map<string, SarProduct>();
  for (const group of groups) {
    for (const product of group) merged.set(productKey(product), product);
  }
  return [...merged.values()];
}

export function productsForKeys(
  products: SarProduct[],
  selectedKeys: Iterable<string>,
): SarProduct[] {
  const selected = new Set(selectedKeys);
  return products.filter((product) => selected.has(productKey(product)));
}

export function selectBurstAssetsForDownload(
  product: SarProduct,
  policy: BurstAssetPolicy,
): SarProduct {
  if (product.dataset.trim().toUpperCase() !== "SLC-BURST") return product;
  const assets = product.assets ?? [];
  const requested = policy === "all" ? ["primary", "metadata"] : [policy];
  const selected = requested.map((role) => {
    const label = role === "primary" ? t("shared.sarProduct.rolePrimaryTiff") : t("shared.sarProduct.roleMetadataXml");
    const matches = assets.filter((asset) => asset.role === role);
    if (matches.length !== 1) {
      throw new Error(t("shared.sarProduct.burstMissingUniqueAsset", { productId: product.product_id, label }));
    }
    const asset = matches[0];
    if (!asset.url?.trim() || !asset.file_name?.trim()) {
      throw new Error(t("shared.sarProduct.burstMissingUrlOrFilename", { productId: product.product_id, label }));
    }
    return asset;
  });
  return { ...product, assets: selected };
}

export function burstAssetItemIds(
  product: SarProduct,
  policy: BurstAssetPolicy,
): string[] {
  const selected = selectBurstAssetsForDownload(product, policy);
  if (selected === product) return [productKey(product)];
  return (selected.assets ?? []).map(
    (asset) => `${productKey(product)}::asset::${asset.role}`,
  );
}

export function normalizeLegacyScene(scene: LegacySceneRow): SarProduct {
  const family = scene.product_type.trim().toUpperCase() || "OTHER";
  const bbox = scene.footprint_bbox
    ? { ...scene.footprint_bbox, crs: scene.footprint_bbox.crs ?? "EPSG:4326" }
    : null;
  return {
    product_key:
      scene.product_key?.trim()
      || canonicalProductKey("ASF", "SENTINEL-1", scene.scene_id),
    product_id: scene.scene_id,
    scene_id: scene.scene_id,
    provider: "ASF",
    dataset: "SENTINEL-1",
    platform: scene.platform.trim() || null,
    instrument: "C-SAR",
    processing_level: family === "GRD" ? null : family,
    product_family: family,
    beam_mode: scene.beam_mode.trim() || null,
    beam_swath: null,
    polarization: scene.polarization.trim() || null,
    flight_direction: scene.orbit_direction.trim() || null,
    look_direction: null,
    relative_orbit: scene.relative_orbit,
    absolute_orbit: scene.absolute_orbit,
    frame: scene.frame,
    asf_frame: null,
    path: scene.path,
    acquisition_start: scene.acquisition_datetime || null,
    acquisition_stop: null,
    footprint_geojson: scene.footprint_geojson ?? null,
    footprint_bbox: bbox,
    download_url: scene.download_url ?? null,
    file_name: null,
    file_size_remote: scene.file_size_remote ?? null,
    checksum: null,
    local_path: null,
    native_metadata: {
      legacy_scene_id: scene.scene_id,
      legacy_product_type: scene.product_type,
    },
    burst: null,
  };
}

export function sceneProductKey(
  scene: LegacySceneRow & { sar_product?: SarProduct },
): string {
  return productKey(scene.sar_product ?? normalizeLegacyScene(scene));
}

export function projectSarProductToScene(product: SarProduct): SarSceneProjection {
  const sceneId = product.scene_id?.trim() || product.product_id;
  return {
    ...product,
    product_key: productKey(product),
    product_id: product.product_id,
    scene_id: sceneId,
    platform: product.platform ?? "",
    product_type: product.processing_level ?? product.product_family,
    beam_mode: product.beam_mode ?? "",
    polarization: product.polarization ?? "",
    acquisition_datetime: product.acquisition_start ?? "",
    absolute_orbit: product.absolute_orbit,
    relative_orbit: product.relative_orbit,
    path: product.path,
    frame: product.frame,
    orbit_direction: product.flight_direction ?? "",
    file_size_remote: product.file_size_remote,
    footprint_bbox: product.footprint_bbox,
    footprint_geojson: product.footprint_geojson,
    has_url: Boolean(product.download_url),
    download_url: product.download_url ?? undefined,
    sar_product: product,
  };
}

export function supportsProductAction(
  product: SarProduct,
  capabilities: DatasetCapabilities | null | undefined,
  action: string,
): boolean {
  const rules = capabilities?.supported_actions.filter((rule) => rule.action === action) ?? [];
  return rules.some((rule) => {
    if (rule.platforms.length && (!product.platform || !rule.platforms.includes(product.platform))) {
      return false;
    }
    if (rule.product_families.length && !rule.product_families.includes(product.product_family)) {
      return false;
    }
    return rule.required_fields.every((field) => {
      const value = product[field as keyof SarProduct];
      return value !== null && value !== undefined && value !== "";
    });
  });
}
