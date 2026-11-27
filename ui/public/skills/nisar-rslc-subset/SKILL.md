---
name: nisar-rslc-subset
description: "Plans and executes client-derived HTTP Range subsetting on remote NISAR L1/L2 HDF5 and ALOS-2 products over an AOI polygon without downloading full multi-decagigabyte granules."
license: MIT
metadata:
  author: EOStage
---

# NISAR RSLC & HDF5 HTTP Range Subsetting

## Purpose

Full NISAR L-band / S-band RSLC HDF5 granules frequently exceed $20\text{–}60\text{ GB}$. When an InSAR study targets a specific fault segment, landslide, or urban basin, EOStage's client-derived HDF5 HTTP Range engine reads only the HDF5 B-tree metadata prefix and fetches the exact chunk byte ranges intersecting the working AOI.

## Scientific Authority Rules

1. **Pre-Range Authority Freeze**:
   - Before issuing science chunk HTTP Range requests, freeze the source granule identity (`SHA-256` / `CMR concept ID`), effective AOI geometry digest, selected frequency (`A` or `B`), and polarizations (`HH`, `HV`, `VV`, `VH`, `RH`, `RV`).
2. **Client-Derived Provenance**:
   - Output HDF5 and sidecar JSON must explicitly record `derived_identity_authority = client_derived` so downstream pipelines never confuse a spatial subset with the full archive granule.
