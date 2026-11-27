#!/usr/bin/env python3
"""GRACE TWSA to GWSA water balance decomposition and trend estimator."""
from __future__ import annotations

import argparse
import json
import math
import sys
from typing import Any, Dict, List, Sequence


def compute_gwsa_cm(
    twsa: float, smsa: float, swea: float = 0.0, swsa: float = 0.0
) -> float:
    return twsa - (smsa + swea + swsa)


def propagate_gwsa_uncertainty_cm(
    sigma_twsa: float,
    sigma_smsa: float,
    sigma_swea: float = 0.0,
    sigma_swsa: float = 0.0,
) -> float:
    """Propagate independent 1-sigma component uncertainties (cm EWH) into GWSA."""
    return math.sqrt(
        float(sigma_twsa) ** 2
        + float(sigma_smsa) ** 2
        + float(sigma_swea) ** 2
        + float(sigma_swsa) ** 2
    )


def analyze_gwsa_timeseries(
    records: Sequence[Dict[str, Any]],
    basin_area_km2: float = 10000.0,
) -> Dict[str, Any]:
    """Decompose GRACE TWSA records into GWSA (cm EWH & km^3) and fit linear trend.

    Each element in `records` should provide:
    - `decimal_year` (float)
    - `twsa_cm` (float)
    - `smsa_cm` (float)
    - optional `swea_cm`, `swsa_cm`, `sigma_twsa_cm`, `sigma_smsa_cm`, `sigma_swea_cm`, `sigma_swsa_cm`
    """
    if not records:
        return {
            "sample_count": 0,
            "basin_area_km2": float(basin_area_km2),
            "trend_cm_per_yr": 0.0,
            "trend_km3_per_yr": 0.0,
            "series": [],
        }

    series: List[Dict[str, Any]] = []
    years: List[float] = []
    gwsa_values: List[float] = []

    for rec in records:
        yr = float(rec.get("decimal_year", 0.0))
        twsa = float(rec["twsa_cm"])
        smsa = float(rec["smsa_cm"])
        swea = float(rec.get("swea_cm", 0.0))
        swsa = float(rec.get("swsa_cm", 0.0))
        gwsa_cm = compute_gwsa_cm(twsa, smsa, swea, swsa)
        sigma_cm = propagate_gwsa_uncertainty_cm(
            float(rec.get("sigma_twsa_cm", 1.5)),
            float(rec.get("sigma_smsa_cm", 0.8)),
            float(rec.get("sigma_swea_cm", 0.0)),
            float(rec.get("sigma_swsa_cm", 0.0)),
        )
        # 1 cm over 1 km^2 = 1e-5 km^3
        gwsa_km3 = gwsa_cm * 1e-5 * float(basin_area_km2)
        years.append(yr)
        gwsa_values.append(gwsa_cm)
        series.append(
            {
                "decimal_year": round(yr, 4),
                "gwsa_cm": round(gwsa_cm, 4),
                "sigma_gwsa_cm": round(sigma_cm, 4),
                "gwsa_km3": round(gwsa_km3, 6),
            }
        )

    n = len(years)
    mean_t = sum(years) / n
    mean_y = sum(gwsa_values) / n
    denom = sum((t - mean_t) ** 2 for t in years)
    slope_cm_yr = (
        sum((t - mean_t) * (y - mean_y) for t, y in zip(years, gwsa_values)) / denom
        if denom > 1e-12
        else 0.0
    )
    slope_km3_yr = slope_cm_yr * 1e-5 * float(basin_area_km2)

    return {
        "sample_count": n,
        "basin_area_km2": float(basin_area_km2),
        "mean_gwsa_cm": round(mean_y, 4),
        "trend_cm_per_yr": round(slope_cm_yr, 4),
        "trend_km3_per_yr": round(slope_km3_yr, 6),
        "depletion_detected": slope_cm_yr < -0.5,
        "series": series,
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-json", help="Optional JSON string with GRACE records")
    args = parser.parse_args(argv)

    if not args.input_json:
        sample_records = [
            {"decimal_year": 2022.0, "twsa_cm": -4.2, "smsa_cm": -1.1, "swea_cm": 0.2},
            {"decimal_year": 2023.0, "twsa_cm": -6.0, "smsa_cm": -1.3, "swea_cm": 0.1},
            {"decimal_year": 2024.0, "twsa_cm": -7.9, "smsa_cm": -1.2, "swea_cm": 0.1},
        ]
        print(json.dumps(analyze_gwsa_timeseries(sample_records, basin_area_km2=12000.0)))
        return 0

    payload = json.loads(args.input_json)
    print(
        json.dumps(
            analyze_gwsa_timeseries(
                records=payload.get("records", []),
                basin_area_km2=float(payload.get("basin_area_km2", 10000.0)),
            )
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
