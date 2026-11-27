import contract from "../../../contracts/map-providers.v1.json" with { type: "json" };

export const MAP_ANNOTATION_PROVIDER_KEYS = [
  "tiandituImageLabels",
  "tiandituVectorLabels",
] as const;

export type MapAnnotationProviderKey = (typeof MAP_ANNOTATION_PROVIDER_KEYS)[number];
export type MapAnnotationZIndexRole = "imageAnnotation" | "vectorAnnotation";

export type MapAnnotationProviderDefinition = {
  key: MapAnnotationProviderKey;
  display_name: string;
  tile_url_template: string;
  subdomains: string[];
  exact_image_origins: string[];
  requires_token: boolean;
  enabled_in_preview: boolean;
  production_available: boolean;
  layer: "cia" | "cva";
  source_key: "cia_w" | "cva_w";
  csp_origins: string[];
  z_index_role: MapAnnotationZIndexRole;
  attribution: string;
};

const annotationContract = contract as {
  annotation_overlays: MapAnnotationProviderDefinition[];
};

export const MAP_ANNOTATION_PROVIDERS = annotationContract.annotation_overlays;
export const ENABLED_MAP_ANNOTATION_PROVIDERS = MAP_ANNOTATION_PROVIDERS.filter(
  (provider) => provider.enabled_in_preview && provider.production_available,
);

export function mapAnnotationProvider(
  key: MapAnnotationProviderKey,
): MapAnnotationProviderDefinition {
  const provider = ENABLED_MAP_ANNOTATION_PROVIDERS.find((candidate) => candidate.key === key);
  if (!provider) throw new Error("MAP_ANNOTATION_PROVIDER_NOT_ENABLED");
  return provider;
}
