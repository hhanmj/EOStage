import assert from "node:assert/strict";
import test from "node:test";

import {
  ENABLED_MAP_ANNOTATION_PROVIDERS,
  MAP_ANNOTATION_PROVIDER_KEYS,
  mapAnnotationProvider,
} from "./mapAnnotationProviders.ts";

test("map annotation contract owns the two independent Tianditu label overlays", () => {
  assert.deepEqual(MAP_ANNOTATION_PROVIDER_KEYS, [
    "tiandituImageLabels",
    "tiandituVectorLabels",
  ]);
  assert.deepEqual(
    ENABLED_MAP_ANNOTATION_PROVIDERS.map((provider) => provider.key),
    MAP_ANNOTATION_PROVIDER_KEYS,
  );

  const image = mapAnnotationProvider("tiandituImageLabels");
  const vector = mapAnnotationProvider("tiandituVectorLabels");
  assert.equal(image.source_key, "cia_w");
  assert.equal(image.layer, "cia");
  assert.equal(image.z_index_role, "imageAnnotation");
  assert.equal(vector.source_key, "cva_w");
  assert.equal(vector.layer, "cva");
  assert.equal(vector.z_index_role, "vectorAnnotation");
  assert.ok(image.requires_token && vector.requires_token);
  assert.ok(image.tile_url_template.includes("tk={token}"));
  assert.ok(vector.tile_url_template.includes("tk={token}"));
  assert.deepEqual(image.subdomains, ["0", "1", "2", "3", "4", "5", "6", "7"]);
  assert.deepEqual(vector.subdomains, image.subdomains);
  assert.ok(
    [...image.exact_image_origins, ...vector.exact_image_origins].every(
      (origin) => origin.startsWith("https://") && !origin.includes("*"),
    ),
  );
});
