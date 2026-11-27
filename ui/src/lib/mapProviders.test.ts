import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import {
  DEFAULT_MAP_PROVIDER,
  ENABLED_MAP_PROVIDERS,
  MAP_PROVIDER_CONTRACT,
  mapTileLayer,
} from "./mapProviders.ts";
import {
  effectiveTiandituToken,
  hasEffectiveTiandituToken,
} from "./mapRuntimeCredentials.ts";

test("map provider contract keeps the enabled list and exact image origins bounded", async () => {
  const root = path.resolve(import.meta.dirname, "..", "..", "..");
  const contract = JSON.parse(
    await readFile(path.join(root, "contracts", "map-providers.v1.json"), "utf8"),
  ) as {
    default_provider_key: string;
    providers: Array<{
      key: string;
      exact_image_origins: string[];
      enabled_in_preview: boolean;
      production_available?: boolean;
      requires_token: boolean;
    }>;
  };
  const enabled = contract.providers.filter(
    (provider) => provider.enabled_in_preview && provider.production_available === true,
  );
  assert.equal(contract.default_provider_key, "arcgisSatellite");
  assert.deepEqual(
    enabled.map((provider) => provider.key),
    [
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
    ],
  );
  assert.equal(contract.providers.some((provider) => provider.key === "localFallback"), false);
  assert.ok(contract.providers.every((provider) =>
    provider.exact_image_origins.every((origin) => !origin.includes("*")),
  ));
  assert.equal(MAP_PROVIDER_CONTRACT.default_provider_key, "arcgisSatellite");
  assert.equal(DEFAULT_MAP_PROVIDER.key, "arcgisSatellite");
  assert.deepEqual(ENABLED_MAP_PROVIDERS.map((provider) => provider.key), enabled.map((provider) => provider.key));
});

test("all Leaflet surfaces can switch between reviewed provider identities through one authority", () => {
  const osm = mapTileLayer("osm");
  const satellite = mapTileLayer("arcgisSatellite");
  assert.notEqual(osm.url, satellite.url);
  assert.equal(osm.attribution, "&copy; OpenStreetMap contributors");
  assert.equal(satellite.attribution, "&copy; Esri");
});

test("the provider contract owns every Leaflet subdomain and exact resolved host", () => {
  const expected: Record<string, string[]> = {
    osm: [],
    cartoLight: ["a", "b", "c", "d"],
    cartoDark: ["a", "b", "c", "d"],
    arcgisSatellite: [],
    arcgisTopo: [],
    arcgisStreet: [],
    openTopoMap: ["a", "b", "c"],
    googleSatellite: ["0", "1", "2", "3"],
    googleMap: ["0", "1", "2", "3"],
    googleHybrid: ["0", "1", "2", "3"],
    gaodeMap: ["1", "2", "3", "4"],
    gaodeSatellite: ["1", "2", "3", "4"],
    tiandituSatellite: ["0", "1", "2", "3", "4", "5", "6", "7"],
    tiandituVector: ["0", "1", "2", "3", "4", "5", "6", "7"],
  };

  for (const provider of ENABLED_MAP_PROVIDERS) {
    assert.deepEqual(provider.subdomains, expected[provider.key], provider.key);
    assert.deepEqual(mapTileLayer(provider.key).subdomains, expected[provider.key], provider.key);
    const resolvedOrigins = provider.subdomains.length > 0
      ? provider.subdomains.map((subdomain) => new URL(
          provider.tile_url_template.replaceAll("{s}", subdomain)
            .replaceAll("{z}", "3")
            .replaceAll("{x}", "6")
            .replaceAll("{y}", "3")
            .replaceAll("{r}", "")
            .replaceAll("{token}", "redacted"),
        ).origin)
      : [new URL(
          provider.tile_url_template
            .replaceAll("{z}", "3")
            .replaceAll("{x}", "6")
            .replaceAll("{y}", "3")
            .replaceAll("{r}", "")
            .replaceAll("{token}", "redacted"),
        ).origin];
    assert.deepEqual(resolvedOrigins, provider.exact_image_origins, provider.key);
    assert.deepEqual(resolvedOrigins, provider.csp_origins, provider.key);
  }
});

test("the built-in Tianditu credential is available while a user override wins", () => {
  const builtIn = effectiveTiandituToken("");
  assert.equal(hasEffectiveTiandituToken(""), true);
  assert.ok(builtIn.length > 20);
  assert.equal(effectiveTiandituToken("  owner-override  "), "owner-override");
  assert.notEqual(builtIn, "owner-override");
});
