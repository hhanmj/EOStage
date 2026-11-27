---
name: remote-sensing-analysis
description: "Classical satellite, aerial or drone imagery analysis: STAC search, processing levels, scale/offset, the Sentinel-2 2022 baseline offset, cloud masks, NDVI/NDWI, composites, land-cover classification, spatial validation, Sentinel-1 SAR. Run before change-detection; neural methods: geo-deep-learning."
license: MIT
metadata:
  author: Muhammed Enes Duran
---

# Remote Sensing Analysis

Purpose: turn raw Earth observation imagery into defensible analytical
products. The failure modes here are subtle — uncorrected DNs treated as
reflectance, clouds counted as land cover change, indices computed on the
wrong bands — so this skill front-loads the checks.

## Data access (STAC-first)

Search via STAC APIs rather than per-provider portals; the workflow is
uniform and scriptable:

```python
import pystac_client
import odc.stac

catalog = pystac_client.Client.open("https://earth-search.aws.element84.com/v1")
items = catalog.search(
    collections=["sentinel-2-l2a"],
    bbox=[27.0, 38.3, 27.4, 38.6],
    datetime="2025-05-01/2025-09-30",
    query={"eo:cloud_cover": {"lt": 20}},
).item_collection()
ds = odc.stac.load(items, bands=["red", "green", "nir", "swir16", "scl"], resolution=10, chunks={})
```

Key collections: `sentinel-2-l2a` (10 m optical, surface reflectance),
`landsat-c2-l2` (30 m, 1982→), `sentinel-1-grd` / `sentinel-1-rtc` (SAR,
weather-independent). From local Python environments, Earth
Search serves Sentinel-2 COGs and Copernicus DEM (`cop-dem-glo-30`; rewrite its
`s3://copernicus-dem-30m/` href to `https://copernicus-dem-30m.s3.amazonaws.com/`)
anonymously, but its Landsat and Sentinel-1 assets are requester-pays `s3://`
paths. Take those from Microsoft Planetary Computer
(`https://planetarycomputer.microsoft.com/api/stac/v1`, opened with
`modifier=planetary_computer.sign_inplace`; install with
`python3 -m pip install --user planetary-computer==1.0.0`).
Earth Search COG reads from mainland China vary from about 20 KB/s to 600 KB/s; Planetary
Computer's `sentinel-2-l2a` (band keys `B03`, `B08`) is several times faster.
Read each needed window once into a local file and run later steps from that copy.

For continental extents or decades-long stacks, stay on these anonymous STAC
APIs (prefer Planetary Computer for archive-scale Sentinel-2; keep Earth Search
to small areas) and let `odc.stac.load(items, chunks={})` load lazily: a coarser
`resolution`, a smaller `bbox` or a temporal aggregate keeps memory usage
bounded on modest workstations. Google Earth Engine is an option only with an account
and network access to it (unreachable from mainland China). Record collection +
item IDs + search parameters for reproducibility.

## Processing-level discipline

| Level | Meaning | Analysis-ready? |
|---|---|---|
| L1C / L1TP | Top-of-atmosphere (TOA) | Indices OK-ish; cross-date comparison risky |
| **L2A / L2SP** | Surface reflectance (BOA) | Yes — default choice |
| GRD (SAR) | Detected amplitude | Needs terrain correction + speckle filter |

Always state which level you used. Never mix TOA and BOA scenes in one
composite or time series. Landsat Collection 2 L2 needs its scale factors
applied (`reflectance = DN * 0.0000275 - 0.2`).

### The Sentinel-2 baseline discontinuity — passes the level check above

Processing Baseline 04.00, applied from **25 January 2022**, added a constant
`BOA_ADD_OFFSET` (currently −1000) to L2A digital numbers so that negative
surface reflectance can be encoded. Two scenes on opposite sides of that date
are **both L2A**: the level check above sees nothing wrong while their DNs sit
1000 apart. Differencing them yields a systematic reflectance shift that reads
as real change and survives every mask, threshold and accuracy report you
apply afterwards.

- Read `BOA_ADD_OFFSET` and `QUANTIFICATION_VALUE` from each product's
  metadata rather than hardcoding −1000 and 10000; both are per-band and the
  baseline has changed before. Where each STAC collection keeps them is under
  **On STAC** below; Earth Search's `granule_metadata` asset is the tile XML
  and carries neither.
- Convert with `reflectance = (DN + BOA_ADD_OFFSET) / QUANTIFICATION_VALUE`.
- Record the **processing baseline of every scene** in the manifest, not just
  the product level. Two L2A scenes is not a sufficient statement.
- **Do not correct twice.** Harmonised collections — Earth Engine's
  `COPERNICUS/S2_SR_HARMONIZED` and several commercial mirrors — have already
  shifted post-baseline data back to the pre-2022 range. Applying the offset
  again inverts the error rather than removing it.
- If the baseline is undocumented for either scene, the comparison is not
  defensible. Say that instead of assuming pre- or post-2022.
- **On STAC**, `odc.stac.load` applies neither scale nor offset, and whether
  the pixels are already shifted depends on the collection; convert once.
  Earth Search `sentinel-2-l2a` lists `offset: -0.1` in `raster:bands` on
  baseline ≥ 04.00 items, but its pixels are mixed: items with
  `earthsearch:boa_offset_applied: true` have the −1000 already removed
  (multiply by `scale` only; adding the offset again turned a 2023 scene's
  land reflectance negative), while items flagged `false` may or may not have
  it removed (both seen near Hangzhou, tested 2026-10-12). For a `false` item,
  check dark pixels first: clear-water NIR DNs above 1000 mean the offset is
  still in, so apply both `scale` and `offset`. Earth Search `sentinel-2-c1-l2a`
  keeps ESA's DNs, so apply its `raster:bands` `scale` and `offset`. Planetary
  Computer `sentinel-2-l2a` also keeps ESA's DNs but lists no `raster:bands`:
  read `BOA_ADD_OFFSET` from the item's `product-metadata` asset.

## Cloud and quality masking — before anything else

- Sentinel-2: mask with SCL band (drop classes 3 cloud shadow, 8-9 clouds,
  10 cirrus, 11 snow — keep 4 vegetation, 5 bare, 6 water, 7 unclassified
  with care).
- Landsat C2: decode `QA_PIXEL` bitfields (cloud, shadow, cirrus bits).
- Report the % of valid pixels after masking per scene; scenes below ~60%
  valid usually deserve exclusion.
- For gap-free products, build median composites over a season rather than
  cherry-picking single scenes.

## Spectral indices

Compute on surface reflectance, guard against division by zero, and name
bands explicitly — band **numbers differ across sensors** (NIR is B8 on
Sentinel-2, B5 on Landsat 8/9):

```python
import numpy as np
import xarray as xr

def normalized_diff(a: xr.DataArray, b: xr.DataArray) -> xr.DataArray:
    """(a - b) / (a + b) with zero-denominator protection."""
    return xr.where(a + b == 0, np.nan, (a - b) / (a + b))

ndvi = normalized_diff(ds.nir, ds.red)     # vegetation
ndwi = normalized_diff(ds.green, ds.nir)   # open water (McFeeters)
ndbi = normalized_diff(ds.swir16, ds.nir)  # built-up
```

Interpretation guardrails: NDVI thresholds are scene- and season-dependent;
never hardcode "NDVI > 0.3 = vegetation" without checking the histogram.
Water confuses NDBI; shadows mimic water in NDWI — cross-check indices
against each other and against true-color.

## Classification workflow

1. Define a legend with mutually exclusive, imagery-separable classes.
2. Collect training samples spatially spread across the scene; record them
   as a versioned vector file.
3. Features: bands + indices + texture (GLCM) + temporal statistics if
   multi-date. For deep learning routes, hand off to geo-deep-learning.
4. Validate with a **spatially independent** test set (see
   `references/spatial-cv-protocol.md`) and
   report per-class F1/IoU plus a confusion matrix — overall accuracy alone
   hides rare-class failure.
5. Map the errors: a spatial plot of misclassifications reveals systematic
   problems (terrain shadow, urban/bare confusion) that global metrics hide.

## SAR notes (Sentinel-1)

Preprocess: orbit file → thermal noise removal → calibration (σ⁰) →
terrain correction (Range-Doppler with a DEM) → speckle filter (Lee/Refined
Lee) → dB conversion. Work in dB for statistics; VV/VH ratio is a strong
water/vegetation discriminator. SAR sees through clouds — prefer it for
flood mapping and continuous monitoring.

## Pitfalls checklist

- Comparing scenes across dates without consistent atmospheric correction.
- Mixing Sentinel-2 scenes across the 2022-01-25 baseline change without
  applying `BOA_ADD_OFFSET` — or applying it a second time on a collection
  that is already harmonised.
- Ignoring 20 m→10 m band mixing on Sentinel-2 (B11/B12 are natively 20 m).
- Computing indices on integer DNs without scale factors → nonsense ranges.
- Median composites of SAR in linear units (do statistics in dB).
- Training and test pixels from the same field/polygon → leaked accuracy.
- Forgetting nodata masks after reprojection (edges become zeros → fake
  land cover).

## Execution contract

- **Workflow:** define phenomenon and scale; select sensor, product level, and dates; harmonize calibration, masks, CRS, and resolution; derive features; analyze; validate spatially; publish provenance.
- **Decision rules:** use this skill for imagery preparation and classical analysis, change detection for explicit temporal differencing, geo-deep-learning for neural training, and anonymous STAC with lazy odc-stac loading for archive-scale work (Earth Engine only with an account and network access).
- **Verification protocol:** inspect masks and valid counts, confirm scale factors, offsets and processing baseline per scene, confirm band resolution, and overlay outputs; classification and change products add spatially independent validation, an error map, and seasonal or sensor sensitivity tests.
- **Failure modes:** reject results for cloud or shadow leakage, incomparable processing levels, undocumented or mismatched processing baselines, resampling artifacts, label leakage, nodata contamination, or claims beyond sensor resolution.
- **Deliverables:** analysis-ready imagery or features, processing manifest, masks, derived products, validation metrics and error map for classification and change products, reproducible code, and limitations.
- **Source freshness:** consult [the authoritative source registry](references/authoritative-sources.md) at execution time for product, calibration, and catalog changes.
