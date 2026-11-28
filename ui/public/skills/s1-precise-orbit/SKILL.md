---
name: s1-precise-orbit
description: "Scans Sentinel-1 SLC metadata/SAFE timestamps and matches/downloads Precise Orbit Ephemerides (AUX_POEORB) or Restituted Orbits (AUX_RESORB) by UTC sensing window and satellite platform (S1A/S1B/S1C)."
license: MIT
metadata:
  author: EOStage
---

# Sentinel-1 Precise Orbit Ephemerides Matching (`s1-precise-orbit`)

## Trigger

Activate this skill whenever matching, validating, or batch-downloading orbit ephemeris files (`.EOF`) for Sentinel-1 `IW` `SLC` or `SLC-BURST` scenes (from search batches or local `.SAFE` directories).

## Inputs & Units

- **Satellite Platform**: `S1A`, `S1B`, or `S1C` (strictly matched per scene).
- **Sensing Window (UTC)**: Exact `[Sensing_Start, Sensing_Stop]` timestamps parsed from local `.SAFE` directory names/manifests or search batch results.
- **Validity Margin**: $\pm 60\text{ s}$ enclosure buffer around the scene sensing window.
- **Latency Threshold**: $20\text{ days}$ post-acquisition for ESA `AUX_POEORB` publication; `< 5 cm` 3D RMS orbit accuracy.

## Ephemeris Selection & Workflow Rules

1. **Extract & Verify Scene Timestamps**: Read platform (`S1A`/`S1B`/`S1C`) and `[Sensing_Start, Sensing_Stop]` UTC timestamps across all selected SLC granules (or local `.SAFE` folders).
2. **Prioritize `AUX_POEORB` (Precise Orbit)**:
   - Availability: published ~20 days after satellite sensing.
   - 3D RMS accuracy: $< 5\text{ cm}$ — mandatory for SBAS / PS-InSAR time series, ESD burst coregistration, and tectonic deformation.
3. **Controlled Fallback to `AUX_RESORB` (Restituted Orbit)**:
   - Availability: within hours of acquisition.
   - Allow `AUX_RESORB` **only** when sensing age is $< 20\text{ days}$ (`AUX_POEORB` not yet released) and the user accepts preliminary rapid emergency/co-seismic processing; flag those scenes for `AUX_POEORB` replacement later.
4. **Strict Validity-Window Enclosure Rule**:
   - Selected `.EOF` (`[Validity_Start, Validity_Stop]`) must strictly enclose `[Sensing_Start - 60s, Sensing_Stop + 60s]` for the exact matching satellite platform.
5. **Batch Plan Approval**: Submit matched orbit download tasks via `propose_plan` so the user verifies and approves on the desktop confirmation card.

## Failure & Stop Conditions

- Never pair an `S1A` scene with an `S1C` (or `S1B`) orbit file, and never accept an `.EOF` whose validity window clips `[Sensing_Start - 60s, Sensing_Stop + 60s]`.
- If upstream CDSE/ASF returns no matching `AUX_POEORB` or `AUX_RESORB`, report the affected scene IDs and UTC timestamps without fabricating orbit filenames.

## Deliverable & Verification Protocol

Consult `references/s1-orbit-matching.md` for canonical `.SAFE` and `.EOF` naming conventions, and report a per-scene match table (`Scene ID`, `Platform`, `Sensing UTC`, `Matched Orbit Class POEORB/RESORB`, `EOF Filename / Validity Enclosure`, `Plan Status`).
