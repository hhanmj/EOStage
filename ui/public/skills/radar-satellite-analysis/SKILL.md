---
name: radar-satellite-analysis
description: "Canonical EOStage SAR & InSAR workflow covering Sentinel-1 IW SLC baseline pairing, ALOS-2 PALSAR-2 L-band stripmap, NISAR L/S-band RSLC subsetting, backscatter/coherence planning, and companion orbit/DEM/GACOS preparation."
license: GPL-3.0-or-later
metadata:
  author: EOStage
---

# Radar & SAR Satellite Analysis (`radar-satellite-analysis`)

## Trigger

Activate this skill when the user asks to design, search, filter, pair, or troubleshoot synthetic aperture radar (SAR) or interferometric SAR (InSAR) workflows in EOStage, including:
- Sentinel-1 IW `SLC` / `SLC-BURST` interferometric pair selection and time-series planning
- ALOS-2 PALSAR-2 L-band Fine Beam (`SM1` / `SM3`) vegetated landslide or large-gradient coseismic pairing
- NISAR L-band / S-band `RSLC` spatial subsetting and polarization/frequency verification
- Sentinel-1 `GRD` backscatter, flood/water thresholding, and multi-temporal compositing
- Auxiliary orbit, DEM vertical datum, and GACOS atmospheric delay planning

## Inputs & Units

- **AOI Reference**: `working_aoi` or `context_aoi` (never fabricate raw polygon coordinates).
- **Sensing Window**: Explicit UTC start and stop timestamps (`YYYY-MM-DDTHH:MM:SSZ` or ISO date bounds) aligned to an earthquake, landslide, flood event, or monitoring period.
- **Track & Geometry**: Flight direction (`ASCENDING` or `DESCENDING`), relative orbit (`pathNumber`), frame (`frameNumber`), and swath/beam mode (`IW`, `SM1`, `SM3`, etc.).
- **Polarization & Frequency**: Polarization channels (`VV`, `VV+VH`, `HH`, `HH+HV`) and NISAR center band (`L-band` vs `S-band`, `frequencyA` / `frequencyB`).
- **Baselines & Backscatter Units**:
  - Perpendicular baseline $B_\perp$ in meters ($\text{m}$); target $B_\perp < 150\text{ m}$ for C-band Sentinel-1 IW interferograms when baseline metadata is available.
  - Temporal baseline $B_t$ in days (Sentinel-1 nominal multiples of $6 / 12 / 24\text{ d}$).
  - GRD backscatter values in decibels ($\text{dB}$) vs linear power ($\sigma^0_{\text{lin}} = 10^{\text{dB}/10}$).

## Progressive Disclosure References

Call `skill_read(skill_id="radar-satellite-analysis", file_path=...)` or search with `query=...` to load sensor-specific interferometric pairing branches:

| Reference File | Focus | When to Read |
|---|---|---|
| `references/s1-baseline-pair-branch.md` | Sentinel-1 IW `SLC` / `SLC-BURST` spatiotemporal baseline pairing, $B_\perp < 150\text{ m}$ and 6/12/24 d cadence, path/polarization consistency | Sentinel-1 coseismic, landslide, or urban subsidence interferogram planning |
| `references/alos2-stripmap-branch.md` | ALOS-2 PALSAR-2 L-band ($\lambda \approx 23.6\text{ cm}$) `SM1`/`SM3` `HH`/`HH+HV` pairing for dense vegetation and high-gradient deformation | C-band temporal decorrelation risk, mountain forests, or ALOS-2 PALSAR-2 tasks |

## Companion Built-in Skills

Invoke companion skills via `skill_read` instead of duplicating their rules:
- `s1-precise-orbit`: Mandatory `AUX_POEORB` ($\sim 20\text{ d}$ latency) vs `AUX_RESORB` validity-window matching for Sentinel-1 SLC scenes.
- `dem-datum-conversion`: Runtime GDAL/PROJ + EGM96/EGM2008 grid check and orthometric-to-WGS84 ellipsoidal height conversion ($h = H + N$) covering the SAR footprint with buffer.
- `nisar-rslc-subset`: Remote HDF5 byte-range subsetting (`_SUBSET.h5`) for large NISAR L/S-band RSLC granules.
- `gacos-ztd-atmo`: Batching GACOS zenith total delay (`.ztd` + `.ztd.rsc`) requests by exact UTC acquisition timestamps ($\le 20$ dates/batch, $\le 10^\circ \times 10^\circ$).

## Decision Rules

1. **Confirm AOI and Window First**: Verify the active AOI reference (`working_aoi` or `context_aoi`) and UTC date bounds via read-only tools before running `search_start` via `propose_plan`.
2. **Never Mix Incompatible Geometry or Frequency**: Strictly separate `ASCENDING` and `DESCENDING` tracks, require identical `pathNumber` (relative orbit) across interferometric pairs, and never mix NISAR L-band with S-band or Sentinel-1 `VV` with ALOS-2 `HH` in one interferometric stack.
3. **Prefer Subsets When Full Archives Are Excessive**: Use `SLC-BURST` for localized Sentinel-1 targets and EOStage HDF5 subsetting for NISAR when full-scene transfers exceed storage budgets.
4. **Linear Power Before GRD Averaging**: Convert Sentinel-1 GRD $\text{dB}$ values to linear power ($10^{\text{dB}/10}$) prior to spatial averaging or temporal compositing, apply Lee or Refined Lee speckle filtering before threshold-based water/flood mapping, and convert back to $\text{dB}$ only for visualization. Account for foreshortening, layover, and shadow in steep terrain.
5. **Two-Stage Plan Submission**: Always submit searches (`search_start`) and download tasks (`create_task`) via `propose_plan` so the user reviews and approves on the desktop confirmation card.

## Failure & Stop Conditions

Stop and report transparently without fabricating candidates if any of the following occur:
- No AOI is attached or selected in the workspace.
- Search returns zero products matching consistent beam mode, flight direction, `pathNumber`, and polarization.
- Required baseline metadata ($B_\perp$) is unavailable from the upstream catalog: mark $B_\perp$ as `unknown — verify baseline after download` rather than inventing numbers.
- Native GIS / geoid runtime (`dem-datum-conversion`) is missing when the user specifically requires local WGS84 ellipsoidal DEM output: separate data acquisition from unready local conversion.
- A plan in `propose_plan` is `rejected` or still pending user approval: never claim scenes or pairs were queued or downloaded.

## Deliverable & Verification Protocol

Every pairing or data-preparation response MUST conclude with:
1. **Track & Sensor Summary**: Sensor, beam/mode, flight direction (`ASCENDING`/`DESCENDING`), relative orbit (`pathNumber`/Frame), polarization, and total candidate scenes inspected.
2. **Structured Candidate Pair Table**: Columns for `Master UTC`, `Slave UTC`, `Path / Track`, `Direction`, `Bt (days)`, `Bperp (m)` (or catalog-reported / unknown notice), `Polarization`, and `Companion Prerequisites` (`POEORB`/`RESORB`, `WGS84 DEM`, `GACOS`).
3. **Submitted Plan Status**: Exact `propose_plan` status (`pending_approval`, `executed`, or `rejected`) grounded solely in observed tool output.
