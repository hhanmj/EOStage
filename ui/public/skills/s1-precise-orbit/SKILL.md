---
name: s1-precise-orbit
description: "Matches and downloads Sentinel-1 Precise Orbit Ephemerides (AUX_POEORB) or Restituted Orbits (AUX_RESORB) for SLC scenes by sensing start/stop UTC window and satellite platform (S1A/S1B/S1C)."
license: MIT
metadata:
  author: EOStage
---

# Sentinel-1 Precise Orbit Ephemerides Matching (POEORB / RESORB)

## Purpose

Accurate baseline estimation, burst coregistration (ESD), and topographic phase removal for Sentinel-1 TOPS SLC require state vectors covering the full sensing interval plus margin.

## Ephemeris Selection Rules

1. **AUX_POEORB (Precise Orbit)**:
   - Availability: ~20 days after acquisition.
   - 3D RMS accuracy: $< 5\text{ cm}$.
   - Mandatory for time-series InSAR (SBAS / PS-InSAR) and tectonic deformation studies.
2. **AUX_RESORB (Restituted Orbit)**:
   - Availability: within hours of acquisition.
   - Use only for rapid emergency co-seismic or flood response when `AUX_POEORB` is not yet published.
3. **Validity Window Rule**:
   - The orbit file's `[Validity_Start, Validity_Stop]` must strictly enclose `[Sensing_Start - 60s, Sensing_Stop + 60s]` for the same platform (`S1A`, `S1B`, or `S1C`).
