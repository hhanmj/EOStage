# Sentinel-1 IW SLC Spatiotemporal Baseline Pairing Branch

Use this reference alongside `radar-satellite-analysis/SKILL.md` whenever preparing Sentinel-1 TOPS IW interferometric pairs for coseismic deformation, landslide monitoring, or urban subsidence time series.

## 1. Search Parameters & Constraints

1. **AOI & Monitoring Window**: Confirm `working_aoi` or `context_aoi` and establish event pre-/post-acquisition dates or multi-month time-series bounds.
2. **Source & Product Query**: Verify available sources with `source_list`, then submit `search_start` via `propose_plan` targeting:
   - Sensor / Collection: Sentinel-1 `IW` mode `SLC` (or `SLC-BURST` when only localized burst coverage is needed).
   - Flight Direction: `ASCENDING` or `DESCENDING` (never pair scenes across opposite look directions).
   - Polarization: Include `VV` (`VV` or `VV+VH` IW dual-pol).
   - Relative Orbit: Group strictly by matching `pathNumber` (and consistent burst IDs for `SLC-BURST`).

## 2. Pair Selection Thresholds

- **Perpendicular Baseline ($B_\perp$)**: Target $B_\perp < 150\text{ m}$ to control topographic phase sensitivity and geometric decorrelation. If the search catalog does not return $B_\perp$ values directly, state that perpendicular baseline must be computed after state-vector ingestion rather than guessing numeric baselines.
- **Temporal Baseline ($B_t$)**: Prefer nominal Sentinel-1 repeat multiples of $6$, $12$, or $24\text{ days}$ for single interferograms or connected short-baseline networks (SBAS); span pre-event and first clean post-event acquisitions for coseismic pairs.
- **Geometric Coverage Parity**: Confirm identical swath/sub-swath (`IW1`/`IW2`/`IW3`) and burst overlap over the target AOI.

## 3. Mandatory Companion Steps

- **Precision Orbits (`s1-precise-orbit`)**: Match `AUX_POEORB` ($\ge 20\text{ days}$ post-sensing) or emergency `AUX_RESORB` covering `[Sensing_Start - 60s, Sensing_Stop + 60s]` for every selected SLC.
- **Ellipsoidal DEM (`dem-datum-conversion`)**: Plan Copernicus GLO-30 (`EGM2008`) or NASADEM/SRTM (`EGM96`) covering the full burst footprint plus buffer, converted to WGS84 ellipsoidal height.

## 4. Handoff Deliverable

Present a sorted candidate pair table (`Master`, `Slave`, `Direction`, `Path`, `Bt (d)`, `Bperp (m)`, `Polarization`, `Orbit Type`) and submit selected scene + orbit downloads through `propose_plan` for desktop confirmation-card approval.
