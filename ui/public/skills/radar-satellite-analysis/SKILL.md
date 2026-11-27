---
name: radar-satellite-analysis
description: EOStage SAR & InSAR analysis workflow covering Sentinel-1 SLC/GRD/SLC-BURST, NISAR L/S-band, ALOS-2, precision orbits, DEM datum conversion, and backscatter/coherence planning.
---

# Radar & SAR Satellite Analysis (EOStage)

Use this skill when designing or troubleshooting synthetic aperture radar (SAR) and interferometric SAR (InSAR) data preparation workflows in EOStage.

## Core EOStage SAR Capabilities

1. **Sentinel-1 (ASF / CDSE)**:
   - Prefer `SLC-BURST` when only localized sub-swath coverage is required to minimize transfer volume.
   - Always match `POEORB` (available ~20 days after acquisition) or `RESORB` precision orbit files for interferometric pairs.
   - Keep `ASCENDING` and `DESCENDING` tracks separated; verify consistent `pathNumber` (relative orbit) and polarization (`VV` / `VV+VH`) before forming pairs.
2. **NISAR (L-band & S-band)**:
   - Keep L-band (ASF/NASA Earthdata) and S-band (Bhoonidhi) product identities and polarizations strictly separate.
   - Use EOStage AOI HDF5 range subsetting (`_SUBSET.h5`) when full-scene downloads exceed local storage budgets, verifying common frequency (`frequencyA` / `frequencyB`) and polarization across selected dates.
3. **Auxiliary Inputs for InSAR**:
   - **DEM**: Acquire Copernicus GLO-30 (EGM2008) or SRTM/NASADEM (EGM96) covering the full scene burst footprint, and convert to ellipsoidal height (`WGS84`) with EOStage native GIS runtime when targeting SARscape/ISCE/SNAP.
   - **Atmospheric Correction**: Batch GACOS zenith total delay (`.ztd` + `.ztd.rsc`) requests by exact UTC acquisition timestamps (up to 20 dates per batch, <= 10 deg x 10 deg extent).
4. **Backscatter & Change Analysis**:
   - Convert Sentinel-1 GRD `VV`/`VH` decibel (dB) values back to linear power (`10^(dB / 10)`) before spatial averaging or temporal compositing, then convert back to dB for visualization.
   - Apply Lee/Refined Lee speckle filtering before threshold-based water or flood mapping, and account for foreshortening/layover/shadow in steep terrain.
