#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${INSAR_GIS_RUNTIME_ROOT:-}" ]]; then
  echo "ERROR: INSAR_GIS_RUNTIME_ROOT must point to an unpacked runtime/gis directory." >&2
  exit 1
fi
if [[ ! -f "${INSAR_GIS_RUNTIME_ROOT}/manifest.json" ]]; then
  echo "ERROR: ${INSAR_GIS_RUNTIME_ROOT}/manifest.json not found." >&2
  exit 1
fi

export HDF5_PLUGIN_PRELOAD="${HDF5_PLUGIN_PRELOAD:-::}"

echo "Running strict insar-gis optical_clip native GDAL tests..."
cargo test --locked -p insar-gis --lib optical_clip::tests:: -- --ignored --test-threads=1

echo "Running strict insar-gis stream_ops native GDAL tests..."
cargo test --locked -p insar-gis --lib stream_ops::tests:: -- --ignored --test-threads=1

echo "Running strict insar-gis vector_ops native PROJ/GDAL test..."
cargo test --locked -p insar-gis --lib vector_ops::tests:: -- --ignored --test-threads=1

echo "Running strict insar-gis raster native GDAL tests..."
cargo test --locked -p insar-gis --lib raster::tests:: -- --ignored --test-threads=1

echo "Running strict insar-gis concurrent initialization test..."
cargo test --locked -p insar-gis --lib tests::concurrent_from_product_root_initialization_is_race_free -- --exact --ignored --test-threads=1

echo "Running strict insar-application-service optical_derived_delivery tests..."
cargo test --locked -p insar-application-service --test optical_derived_delivery -- --ignored --test-threads=1

echo "Running strict eostage-desktop native GIS tests..."
for test_name in \
  agent::tests::gee_extension_gis_local_process_buffers_geojson_with_native_gis \
  external_gis_bridge::tests::deliver_project_builds_catalog_geojson_and_well_formed_qgs_xml \
  geodata_tools::tests::test_c4_super_zoom_z20_subtile_crop_and_upsample \
  geodata_tools::tests::test_lane_b_gee_verify_strict_auth_and_collection_mosaic_retry \
  geodata_tools::tests::test_lane_b_satellite_tiles_port_parser_mbtiles_zip_cog_webp_and_preview \
  geodata_tools::tests::test_gee_v1_expression_dag_route2_search_download_and_scutil_proxy
do
  cargo test --locked -p eostage-desktop --lib "$test_name" -- --exact --ignored --test-threads=1
done

echo "All 23 strict native GIS regression tests passed."
