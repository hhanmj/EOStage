---
name: dem-datum-conversion
description: "Converts orthometric DEMs (EGM96 / EGM2008 geoid heights such as Copernicus GLO-30, SRTM, FABDEM, NASADEM) to WGS84 ellipsoidal heights required by InSAR processors (ISCE2, GAMMA, SARscape, GMTSAR, SNAP)."
license: MIT
metadata:
  author: EOStage
---

# DEM Vertical Datum Conversion (EGM96 / EGM2008 → WGS84 Ellipsoid)

## Purpose

InSAR phase simulation and topographic phase removal require heights referenced to the **WGS84 reference ellipsoid** ($h = H + N$), whereas most public global DEMs (Copernicus DEM GLO-30, SRTM GL1, ALOS AW3D30, FABDEM) distribute orthometric heights ($H$) referenced to a geoid model (**EGM2008** or **EGM96**). Skipping geoid-to-ellipsoid conversion introduces a spatially varying vertical datum error up to $\pm 100\text{ m}$ across regional interferograms.

## Required Checks Before Conversion

1. **Identify Source Vertical Datum ($H$)**:
   - Copernicus DEM GLO-30 / GLO-90: `EGM2008`
   - SRTM GL1 / GL3, NASADEM: `EGM96`
   - Local GeoTIFF: inspect vertical CRS WKT (`VERT_CS` / `GEOIDMODEL`) or user declaration.
2. **Verify Local Runtime & Geoid Grids**:
   - Native GDAL/PROJ runtime (`runtime/gis`) must be ready.
   - `egm96/us_nga_egm96_15.tif` (SHA-256 verified) for EGM96 inputs.
   - `egm2008/us_nga_egm08_25.tif` (SHA-256 verified) for EGM2008 inputs.
3. **Preserve Grid Integrity**:
   - Never resample the source DEM grid unless explicitly requested. Evaluate geoid undulation $N(\lambda, \phi)$ at each pixel center and add $h = H + N$ while preserving `NoData` masks and antimeridian continuity.
