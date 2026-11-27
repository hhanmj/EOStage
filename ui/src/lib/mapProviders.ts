import contract from "../../../contracts/map-providers.v1.json" with { type: "json" };

export const MAP_PROVIDER_KEYS = [
  "osm",
  "cartoLight",
  "cartoDark",
  "arcgisSatellite",
  "arcgisTopo",
  "arcgisStreet",
  "openTopoMap",
  "googleSatellite",
  "googleMap",
  "googleHybrid",
  "gaodeMap",
  "gaodeSatellite",
  "tiandituSatellite",
  "tiandituVector",
] as const;

export type MapProviderKey = (typeof MAP_PROVIDER_KEYS)[number];

export type MapProviderDefinition = {
  key: MapProviderKey;
  display_name: string;
  tile_url_template: string;
  subdomains: string[];
  exact_image_origins: string[];
  requires_token: boolean;
  enabled_in_preview: boolean;
  production_available?: boolean;
  tile_route?: string;
  readiness?: "reviewed" | "requires_token" | string;
  crs?: string;
  csp_origins?: string[];
  contains_labels: boolean;
  attribution: string;
  attribution_full?: string;
  category_note?: string;
};

export const MAP_PROVIDER_CONTRACT = contract as {
  contract_id: "map-providers.v1";
  version: 1;
  default_provider_key: MapProviderKey;
  providers: MapProviderDefinition[];
};

export const MAP_PROVIDERS = MAP_PROVIDER_CONTRACT.providers;
export const ENABLED_MAP_PROVIDERS = MAP_PROVIDERS.filter(
  (provider) => provider.enabled_in_preview && provider.production_available === true,
);

const configuredDefaultProvider = MAP_PROVIDERS.find(
  (provider) => provider.key === MAP_PROVIDER_CONTRACT.default_provider_key,
);
if (
  !configuredDefaultProvider ||
  !configuredDefaultProvider.enabled_in_preview ||
  configuredDefaultProvider.production_available !== true
) {
  throw new Error("MAP_PROVIDER_DEFAULT_NOT_ENABLED");
}
export const DEFAULT_MAP_PROVIDER = configuredDefaultProvider;

export function mapProvider(key: MapProviderKey): MapProviderDefinition {
  return MAP_PROVIDERS.find((provider) => provider.key === key) ?? DEFAULT_MAP_PROVIDER;
}

export function isEnabledMapProvider(key: MapProviderKey): boolean {
  return ENABLED_MAP_PROVIDERS.some((provider) => provider.key === key);
}

/** One provider authority for every Leaflet surface, including small cards. */
export function mapTileLayer(key: MapProviderKey = DEFAULT_MAP_PROVIDER.key): {
  url: string;
  attribution: string;
  subdomains: string[];
} {
  const provider = mapProvider(key);
  return {
    url: provider.tile_url_template,
    attribution: provider.attribution,
    subdomains: [...provider.subdomains],
  };
}
