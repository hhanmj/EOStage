---
name: gacos-ztd-atmo
description: "Plans time-matched GACOS zenith total delay (ZTD) tropospheric correction requests from SAR acquisition timestamps and imports delivered GACOS archives with spatial/temporal coverage verification."
license: MIT
metadata:
  author: EOStage
---

# GACOS Tropospheric Delay Correction Planning & Import

## Purpose

Tropospheric water vapor stratification and turbulence frequently mask slow tectonic, volcanic, or land subsidence signals ($1\text{–}10\text{ cm}$ in L/C/X-band interferograms). GACOS (Generic Atmospheric Correction Online Service for InSAR) provides high-resolution Zenith Total Delay (ZTD) maps combining ECMWF HRES and GNSS stations.

## Protocol

1. Extract exact UTC sensing mid-times (`HH:MM` UTC) and bounding box (`[W, E, S, N]` with $0.1^\circ$ margin) from the selected SAR stack.
2. Deduplicate acquisition dates while validating that UTC time-of-day remains consistent across the same relative orbit (`Path`).
3. When importing delivered GACOS `.tar.gz` / `.zip` archives, verify `.ztd` / `.ztd.rsc` or GeoTIFF grid extent and date completeness against every interferometric epoch.
