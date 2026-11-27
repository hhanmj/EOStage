import type { SceneRow } from "./bridge.ts";

const MPC_HLS_PROVIDER_ID = "microsoft.planetary-computer";
const MPC_HLS_TILE_ORIGIN = "https://planetarycomputer.microsoft.com";

/** Disposable true-color projection of an exact frozen HLS snapshot identity. */
export function hlsMapTilePreview(scene: SceneRow): { urlTemplate: string } | null {
  const metadata = scene.native_metadata;
  if (!metadata || typeof metadata !== "object" || Array.isArray(metadata)) return null;
  const product = metadata.logical_dataset === "optical.hls.l30.v2" ? "L30"
    : metadata.logical_dataset === "optical.hls.s30.v2" ? "S30" : null;
  const collection = product === "L30" ? "hls2-l30" : product === "S30" ? "hls2-s30" : null;
  const itemId = metadata.provider_product_id;
  if (
    !product || !collection
    || scene.provider !== MPC_HLS_PROVIDER_ID
    || metadata.native_collection_id !== collection
    || typeof itemId !== "string"
    || !/^HLS\.(L30|S30)\.T\d{2}[A-Z]{3}\.\d{7}T\d{6}\.v2\.0$/.test(itemId)
    || itemId.split(".")[1] !== product
  ) return null;
  const key = `PROVIDER::${MPC_HLS_PROVIDER_ID}::COLLECTION::${collection}::ITEM::${itemId}`;
  if (metadata.provider_product_key !== key || scene.product_key !== key) return null;

  const params = new URLSearchParams();
  params.set("collection", collection);
  params.set("item", itemId);
  params.append("assets", "B04");
  params.append("assets", "B03");
  params.append("assets", "B02");
  params.set("rescale", "0,3000");
  params.set("color_formula", "gamma RGB 2.7, saturation 1.5, sigmoidal RGB 15 0.55");
  // HLS v2.0 reflectance bands use -9999 as fill; 0 would paint the tile's outside as opaque black.
  params.set("nodata", "-9999");
  params.set("format", "png");
  return {
    urlTemplate: `${MPC_HLS_TILE_ORIGIN}/api/data/v1/item/tiles/WebMercatorQuad/{z}/{x}/{y}@1x?${params.toString()}`,
  };
}
