---
name: dem-datum-conversion
description: "Converts orthometric DEMs (EGM96 / EGM2008 geoid heights such as Copernicus GLO-30, SRTM, FABDEM, NASADEM) to WGS84 ellipsoidal heights required by InSAR processors (ISCE2, GAMMA, SARscape, GMTSAR, SNAP), including runtime grid readiness checks and SAR footprint buffer planning."
license: MIT
metadata:
  author: EOStage
---

# DEM Vertical Datum Conversion (`dem-datum-conversion`)

## Trigger

Activate this skill whenever planning, downloading, inspecting, or converting a DEM for InSAR processing, topographic phase removal, geocoding, or SAR geometric alignment (ISCE2, GAMMA, SARscape, GMTSAR, SNAP).

## Inputs & Units

- **Source DEM**: Raster GeoTIFF or catalog dataset (`Copernicus GLO-30` / `GLO-90`, `SRTM GL1`, `NASADEM`, `FABDEM`, `ALOS AW3D30`, or local GeoTIFF).
- **Vertical Quantities (meters, $\text{m}$)**:
  - Orthometric height $H$ above geoid (`EGM2008` / `EPSG:3855` or `EGM96` / `EPSG:5773`).
  - Geoid undulation $N(\phi, \lambda)$ interpolated from verified grid files.
  - Target WGS84 ellipsoidal height $h = H + N$.
- **Footprint Alignment**: Active SAR burst/swath or working AOI polygon plus spatial buffer covering full radar geometry bounds.

## Decision Rules

1. **Identify Source Vertical Datum ($H$) Before Acting**:
   - Copernicus DEM `GLO-30` / `GLO-90`: `EGM2008` (`us_nga_egm08_25.tif`).
   - `SRTM GL1` / `GL3` and `NASADEM`: `EGM96` (`us_nga_egm96_15.tif`).
   - Local GeoTIFF: Inspect vertical CRS WKT (`VERT_CS`, `GEOIDMODEL`) or explicit user metadata; never assume ellipsoidal heights without verification (datum errors reach $\pm 100\text{ m}$ globally).
2. **Verify Local Native Runtime & Geoid Grids First**:
   - Confirm Native GIS runtime (`runtime/gis`, GDAL/PROJ) readiness via read-only runtime status checks before claiming local conversion can run.
   - Verify the required SHA-256-checked geoid grid (`egm96/us_nga_egm96_15.tif` or `egm2008/us_nga_egm08_25.tif`) is installed; if missing, guide installation of the shared vertical-grid runtime component rather than faking conversion.
3. **Footprint Coverage & Grid Integrity**:
   - Ensure the DEM extent encloses the full SAR swath/burst footprint with margin for geometric coregistration.
   - Evaluate $h = H + N$ at native source pixel centers without unintended resampling; strictly preserve `NoData` masks and antimeridian continuity.
   - Submit DEM downloads via `propose_plan` for desktop confirmation-card approval.

## Failure & Stop Conditions

- Stop if source vertical datum cannot be determined from product metadata or user confirmation; ask before applying an arbitrary geoid offset.
- Stop and report `runtime_not_ready` if `runtime/gis` or the required EGM96/EGM2008 grid file is missing locally—never emit a fake output path or claim ellipsoidal conversion completed.

## Deliverable & Verification Protocol

Consult `references/vertical-datum-protocol.md` for the standard product-to-datum mapping table and conversion formula, and report:
1. Source DEM product, horizontal CRS, and verified source vertical datum ($H$).
2. Readiness state of `runtime/gis` and the matched geoid grid file ($N$).
3. Coverage verification against the target SAR/AOI footprint and output target (via `propose_plan` or confirmed local path).
