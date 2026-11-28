---
name: gee-textbook-cookbook
description: "Unified Google Earth Engine (GEE) methodology handbook and engineering guardrail guide: Cloud Score+ / SCL cloud masking, baseline harmonization, harmonic & Theil-Sen trends, Olofsson (2014) spatial CV & unbiased area estimation, hydrology/terrain chunked exports, EECU quota control, cross-sensor harmonization, provenance ledgers, and server-side DAG debugging. Pairs with eo-analysis-recipes."
license: Apache-2.0
metadata:
  author: EOStage
---

# Google Earth Engine (GEE) Methodology & Engineering Guardrails (`gee-textbook-cookbook`)

## Trigger

Activate this skill when designing, reviewing, optimizing, debugging, or auditing Google Earth Engine (GEE) Python remote sensing workflows—especially for optical cloud masking, cross-sensor harmonization, time-series modeling, classification accuracy/area estimation, Sentinel-1 backscatter workflows, EECU quota/memory safety, and raster export preflight.

Pair with `eo-analysis-recipes` (99 runnable sensor/algorithm recipes) when ready-to-run operator templates are needed; use `gee-textbook-cookbook` for scientific methodology decisions, quality/scale guardrails, and DAG performance debugging.

## Inputs & Units

- **AOI & Spatial Scale**: Target geometry/bounds, nominal pixel scale ($\text{m/px}$, e.g. $10\text{ m}$ S2, $30\text{ m}$ Landsat), and output projection/CRS (`EPSG:*`).
- **Radiation & Unit Semantics**:
  - Surface Reflectance (`SR`) scaled to unitless reflectance $[0, 1]$ vs Top-of-Atmosphere (`TOA`)—never mix levels in one stack.
  - Thermal surface temperature in Kelvin ($\text{K}$) or Celsius ($^\circ\text{C}$).
  - Sentinel-1 GRD backscatter in $\text{dB}$ vs linear power ($10^{\text{dB}/10}$).
  - Compute cost in EECU-seconds / pixel budget (`maxPixels`, `tileScale`).

## Unified Progressive-Disclosure Reference Index

Call `skill_read(skill_id="gee-textbook-cookbook", file_path=...)` or `skill_read(skill_id="gee-textbook-cookbook", query=...)` on demand to load only the needed chapters or guardrail cards:

### Part A — Core Methodology Handbook (5 Chapters)

| Reference File | Core Methodology & Decision Scope |
|---|---|
| `references/optical-cloudmask-compositing.md` | Sentinel-2 `COPERNICUS/S2_SR_HARMONIZED` + Cloud Score+ (`linkCollection`) vs SCL fallback (`QA60` deprecation); Landsat C2 L2 `QA_PIXEL` + `QA_RADSAT` & official scale factors |
| `references/time-series-harmonic-trend.md` | Harmonic OLS regression (`sin/cos`), Theil-Sen / Mann-Kendall (`select(["year", "NDVI"])`), and criteria vs `LandTrendr`/`CCDC` |
| `references/classification-accuracy-assessment.md` | Spatially blocked CV (anti-leakage), `smileRandomForest`, confusion matrix, and Olofsson (2014) unbiased stratified area & 95% CI |
| `references/hydrology-terrain-export.md` | Copernicus DEM terrain/slope guards against false flood inundation, JRC water frequency baseline verification, `gee_chunked_download` & VRT alignment |
| `references/performance-joins-debugging.md` | Client/server DAG isolation, filter-early, avoiding premature `.clip`/`.reproject`, `tileScale`/`maxPixels`, `ee.Join` selection, and OOM/timeout diagnostics |

### Part B — Engineering Practice Guardrail Cards (10 Cards)

| Reference File | Operational Guardrail & Verification Contract |
|---|---|
| `references/gee-workflow-review-and-routing.md` | End-to-end GEE agent loop, pre-delivery review checklist, and structured escalation/debugging ladder |
| `references/eecu-quota-and-cost-control.md` | EECU-second awareness, interactive vs batch limits, reducing redundant reductions, and budget estimation before large runs |
| `references/export-preflight-and-routing.md` | Mandatory small-window preflight validation, Drive/Cloud/chunked export channel routing, and post-export disk verification |
| `references/boundary-and-tiling-guardrails.md` | AOI boundary buffering vs premature `.clip`, large-region tiling strategies, and preventing edge artifacts |
| `references/cross-sensor-harmonization.md` | Harmonizing Landsat 7/8/9 and Sentinel-2 spectral bandpasses, common spatial grid alignment, and overlap validation |
| `references/multi-sensor-qa-scale-matrix.md` | Quick-reference QA bitmask, radiometric scale/offset, valid range, and native resolution matrix across major collections |
| `references/landsat-s2-cloud-mask-boundaries.md` | Strict decision boundaries between Landsat `QA_PIXEL`/`QA_RADSAT` and Sentinel-2 Cloud Score+ vs `SCL` masks |
| `references/sentinel1-sar-method-boundaries.md` | Sentinel-1 `COPERNICUS/S1_GRD` linear-power averaging invariant, incidence/orbit stratification, and speckle/terrain limits |
| `references/temporal-compositing-semantics.md` | Temporal aggregation semantics (median vs medoid vs greenest-pixel vs quantile), window symmetry, and empty-pixel safeguards |
| `references/data-layer-provenance-ledger.md` | Mandatory data-layer provenance tracking (collection ID, date filters, mask bitmask/threshold, scale factor, pixel count, CRS) |

## Decision Rules & Invariants

1. **Runtime & Plugin Precondition**: `gee_*` tools require an enabled GEE plugin (`eostage-gee-plugin`) and active GEE OAuth / Cloud Project credentials (`ee.Initialize(project=...)`, with network proxy configured if required). If unconfigured or disabled, report the blocker cleanly instead of guessing server results.
2. **Radiometric Level Discipline**: Never combine `TOA` and `SR` collections in a single temporal stack or change metric; standardize on `COPERNICUS/S2_SR_HARMONIZED` and `LANDSAT/LC08/C02/T1_L2` / `LC09/C02/T1_L2` (applying official C2 L2 scale/offset once).
3. **No Double Baseline Shift on Post-2022 Sentinel-2**: `COPERNICUS/S2_SR_HARMONIZED` already aligns scenes after `2022-01-25` to the pre-2022 baseline; **never** manually add or subtract `1000` (`PB 04.00` offset) on `_HARMONIZED` assets.
4. **Compute & Export Safety**: Filter early in space and time, defer `.clip()` / `.reproject()`, tune `tileScale` when approaching user memory limits, and run preflight scale/pixel checks before large exports.
5. **Provenance Discipline**: Record exact dataset IDs, mask rules, scale factors, nominal resolution, and CRS in the output summary so every map layer or table is reproducible.

## Failure & Stop Conditions

- Stop and surface exact setup gaps if `eostage-gee-plugin` is disabled, GEE credentials/project initialization fail, or EECU/memory errors persist after `tileScale` / tiling remediation.
- Refuse to claim area estimates or accuracies without confusion-matrix counts, nor claim raster exports completed before verifying non-empty target files/task completion.

## Deliverable & Verification Protocol

Deliver structured GEE outputs containing:
1. Selected collection(s), cloud/QA mask rule, radiometric scaling formula, and temporal compositing method.
2. Client/server safety checks (`maxPixels`, `tileScale`, chunking/export route, EECU guardrail notes).
3. Data-layer provenance ledger and verification result (preflight sample stats, accuracy/Olofsson CI if classifying, or verified output file/task status).
