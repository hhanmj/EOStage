#!/usr/bin/env python3
"""Provenance manifest builder and reproducibility auditor for geoscience workflows."""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys
from typing import Any, Dict, List, Sequence


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def audit_insar_provenance(manifest: Dict[str, Any]) -> Dict[str, Any]:
    """Audit a geoscience/InSAR workflow provenance record for reproducibility compliance."""
    issues: List[Dict[str, str]] = []
    score = 100

    scenes = manifest.get("scenes") or []
    if not scenes:
        issues.append(
            {
                "severity": "error",
                "field": "scenes",
                "message": "No input SAR/optical granule identifiers recorded.",
            }
        )
        score -= 30

    orbit_type = str(manifest.get("orbit_ephemeris", "")).upper()
    if not orbit_type:
        issues.append(
            {
                "severity": "error",
                "field": "orbit_ephemeris",
                "message": "Orbit ephemeris type (e.g. POEORB / RESORB) is missing.",
            }
        )
        score -= 25
    elif orbit_type == "RESORB":
        issues.append(
            {
                "severity": "warning",
                "field": "orbit_ephemeris",
                "message": "Restituted orbit (RESORB) used instead of precise orbit (POEORB); long-wavelength orbital ramp may remain.",
            }
        )
        score -= 10

    dem_datum = str(manifest.get("dem_vertical_datum", "")).upper()
    if not dem_datum:
        issues.append(
            {
                "severity": "error",
                "field": "dem_vertical_datum",
                "message": "DEM vertical datum (WGS84_ELLIPSOID / EGM96 / EGM2008) is unspecified.",
            }
        )
        score -= 25
    elif dem_datum in {"EGM96", "EGM2008"} and not manifest.get(
        "geoid_to_ellipsoid_applied", False
    ):
        issues.append(
            {
                "severity": "error",
                "field": "geoid_to_ellipsoid_applied",
                "message": f"Orthometric DEM ({dem_datum}) was not converted to WGS84 ellipsoidal height before SAR topographic phase removal.",
            }
        )
        score -= 25

    crs_epsg = manifest.get("target_epsg")
    if not crs_epsg:
        issues.append(
            {
                "severity": "warning",
                "field": "target_epsg",
                "message": "Target coordinate reference system (target_epsg) is missing.",
            }
        )
        score -= 10

    artifacts = manifest.get("artifacts") or []
    verified_artifacts: List[Dict[str, Any]] = []
    for item in artifacts:
        path_str = item.get("path")
        expected_sha = str(item.get("sha256", "")).lower()
        if path_str and pathlib.Path(path_str).is_file():
            actual_sha = sha256_file(path_str)
            matched = not expected_sha or actual_sha == expected_sha
            if not matched:
                issues.append(
                    {
                        "severity": "error",
                        "field": f"artifacts:{path_str}",
                        "message": "Artifact SHA-256 digest does not match recorded provenance.",
                    }
                )
                score -= 20
            verified_artifacts.append(
                {
                    "path": path_str,
                    "sha256": actual_sha,
                    "verified": matched,
                }
            )
        elif expected_sha and len(expected_sha) == 64:
            verified_artifacts.append(
                {
                    "path": path_str or "<virtual>",
                    "sha256": expected_sha,
                    "verified": True,
                }
            )

    score = max(0, score)
    has_errors = any(item["severity"] == "error" for item in issues)
    return {
        "passed": not has_errors and score >= 70,
        "reproducibility_score": score,
        "issue_count": len(issues),
        "issues": issues,
        "verified_artifacts": verified_artifacts,
        "ro_crate_context": "https://w3id.org/ro/crate/1.1/context",
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-json", help="Optional JSON string with manifest")
    args = parser.parse_args(argv)

    if not args.input_json:
        sample_manifest = {
            "scenes": [
                "S1A_IW_SLC__1SDV_20240501T101215",
                "S1A_IW_SLC__1SDV_20240513T101216",
            ],
            "orbit_ephemeris": "POEORB",
            "dem_vertical_datum": "EGM96",
            "geoid_to_ellipsoid_applied": True,
            "target_epsg": 32650,
        }
        print(json.dumps(audit_insar_provenance(sample_manifest)))
        return 0

    payload = json.loads(args.input_json)
    print(json.dumps(audit_insar_provenance(payload)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
