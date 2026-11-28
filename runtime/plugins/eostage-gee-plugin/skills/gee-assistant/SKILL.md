---
name: gee-assistant
description: "Operational workflow for EOStage native Google Earth Engine tools (gee_catalog_search, gee_dataset_advisor, gee_check_availability, gee_export_preflight, gee_cloud_analyze, gee_chunked_download, gee_script_inspect, gee_quota_status)."
author: EOStage
license: GPL-3.0-or-later
---

# Google Earth Engine Catalog & Cloud Compute Workflow (`gee-assistant`)

## Trigger

Activate this skill whenever `eostage-gee-plugin` is enabled and the user asks to discover GEE datasets, check scene availability over an AOI, run GEE cloud composites/indices, inspect or optimize an Earth Engine script, or export GeoTIFF rasters to disk.

## Inputs & Units

- **AOI Reference**: `working_aoi` or `context_aoi` (bounding geometry in WGS84 degrees; never invent raw coordinates).
- **Temporal Bounds**: ISO UTC start/end dates (`YYYY-MM-DD`).
- **Spatial Scale & Export Limits**: Nominal pixel spacing ($\text{m/px}$, e.g. $10\text{ m}$ for Sentinel-2, $30\text{ m}$ for Landsat), target `EPSG:*` projection, `tileScale`, `maxPixels`, and chunk size when writing multi-tile GeoTIFFs.
- **Radiometric Semantics**: Surface Reflectance (`SR`) scaled to unitless $[0, 1]$ vs Sentinel-1 `GRD` backscatter in linear power ($10^{\text{dB}/10}$) prior to temporal/spatial averaging.

## Native GEE Toolset Workflow

Follow this sequential pipeline using only the enabled native EOStage GEE tools:

1. **Preflight Credentials & Quota (`gee_quota_status`)**:
   - Confirm active GEE OAuth / Cloud Project configuration (`ee.Initialize(project=...)`) and proxy connectivity when needed before claiming cloud operations can run.
2. **Dataset Discovery & Selection (`gee_catalog_search` / `gee_dataset_advisor`)**:
   - Query matching public collections (prefer `COPERNICUS/S2_SR_HARMONIZED` and `LANDSAT/LC08/C02/T1_L2` / `LC09/C02/T1_L2`; never mix `TOA` and `SR`).
3. **Spatiotemporal Availability Check (`gee_check_availability`)**:
   - Verify concrete image counts, acquisition date span, and cloud coverage over the active AOI before heavy analysis or download planning.
4. **Cloud Computation or Script Review (`gee_cloud_analyze` / `gee_script_inspect`)**:
   - Filter early in time and space, defer `.clip()` / `.reproject()`, avoid double baseline shifting on `_HARMONIZED` Sentinel-2 assets, and validate index/reduction outputs.
5. **Preflight & Safe Chunked Export (`gee_export_preflight` -> `gee_chunked_download`)**:
   - Run export preflight to verify pixel count, byte estimate, and tiling safety before writing GeoTIFF chunks to the target directory.

## Companion Built-in Skills

- `gee-textbook-cookbook`: Consult via `skill_read` for 5 methodology handbook chapters and 10 engineering guardrail cards (Cloud Score+ vs SCL, harmonic/Theil-Sen trends, Olofsson 2014 unbiased area estimation, EECU quota safety, provenance ledger, DAG debugging).
- `eo-analysis-recipes`: Consult for 83 runnable GEE Python recipes covering specific sensors and environmental indices.

## Failure & Stop Conditions

- Stop immediately and instruct the user cleanly if `eostage-gee-plugin` is disabled, GEE OAuth / Cloud Project credentials are unconfigured, or `gee_quota_status` reports connectivity failure.
- Never extrapolate missing availability counts, fabricate exported file paths, or declare disk completion without checking actual `gee_chunked_download` output files.

## Deliverable & Verification Protocol

Every GEE task summary must report:
1. Exact verified collection ID(s), temporal range, cloud/QA mask rule, and radiometric scale rule.
2. Observed availability count or preflight dimensions from tool responses (`gee_check_availability` / `gee_export_preflight`).
3. Cloud analysis summary or verified output raster path(s) and CRS/resolution from `gee_chunked_download`.
