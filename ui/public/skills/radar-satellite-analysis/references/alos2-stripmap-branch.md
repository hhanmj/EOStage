# ALOS-2 PALSAR-2 L-Band Stripmap (SM1/SM3) Pairing Branch

Use this reference alongside `radar-satellite-analysis/SKILL.md` for densely vegetated mountain terrain, active landslides under forest canopy, or high-gradient coseismic deformation where C-band (Sentinel-1) suffers severe temporal decorrelation.

## 1. Sensor Rationale & Scope

- **L-band Penetration**: Wavelength $\lambda \approx 23.6\text{ cm}$ penetrates vegetation canopy and tolerates larger pixel-scale phase gradients than C-band ($\lambda \approx 5.6\text{ cm}$), preserving interferometric coherence across vegetated slopes and rapid displacement zones.
- **Supported Stripmap Modes**: Prioritize PALSAR-2 Fine Beam Stripmap modes (`SM1` ultra-fine ~3 m or `SM3` high-sensitive ~10 m) over ScanSAR when local deformation resolution is required.

## 2. Search Parameters & Track Discipline

1. **Confirm AOI & Window**: Lock `working_aoi` or `context_aoi` and key monitoring epochs (pre-event baseline + post-event acquisitions).
2. **Search via `propose_plan`**: Call `search_start` targeting ALOS-2 / PALSAR-2 products with:
   - Beam / Mode: `SM1` or `SM3` (keep pairs strictly within the same beam mode and off-nadir angle).
   - Polarization: `HH` or dual-pol `HH+HV` (verify identical co-pol channel `HH` across master and slave).
   - Geometry Lock: Enforce strict uniformity across `ASCENDING` vs `DESCENDING`, `Path` (track), and `Frame`.

## 3. Auxiliary Elevation & Deliverable Rules

- **Ellipsoidal DEM (`dem-datum-conversion`)**: Pair the ALOS-2 frame footprint with Copernicus DEM GLO-30 (`EGM2008`) or NASADEM (`EGM96`) converted to WGS84 ellipsoidal heights (`h = H + N`) via the native GIS runtime.
- **Zero-Result / Entitlement Stop Rule**: ALOS-2 archives often have sparse temporal revisit or provider credential constraints. If `search_start` returns zero matching `SM1`/`SM3` scenes for the requested track/polarization, report the gap honestly (do not substitute ScanSAR silently or invent candidate scenes) and suggest checking polarization/mode constraints or alternative L-band sources (such as NISAR L-band via `nisar-rslc-subset`).
- **Approval Handoff**: Package verified ALOS-2 scene candidates and paired DEM tasks into `propose_plan` for user approval on the desktop confirmation card.
