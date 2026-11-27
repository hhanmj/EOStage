#!/usr/bin/env python3
"""Antecedent Precipitation Index (API) and I-D landslide threshold calculator."""
from __future__ import annotations

import argparse
import json
import sys
from typing import Any, Dict, List, Sequence


def compute_api(daily_precip_mm: Sequence[float], decay: float = 0.85) -> float:
    return sum((decay ** (idx + 1)) * p for idx, p in enumerate(daily_precip_mm))


def evaluate_id_threshold(
    duration_hours: float,
    intensity_mm_hr: float,
    alpha: float = 14.82,
    beta: float = -0.39,
) -> Dict[str, Any]:
    """Evaluate empirical power-law Rainfall Intensity-Duration threshold I = alpha * D^beta."""
    if duration_hours <= 0:
        raise ValueError("duration_hours must be positive")
    if intensity_mm_hr < 0:
        raise ValueError("intensity_mm_hr must be non-negative")

    threshold_mm_hr = alpha * (duration_hours ** beta)
    exceedance_ratio = intensity_mm_hr / threshold_mm_hr if threshold_mm_hr > 0 else 0.0
    margin_mm_hr = intensity_mm_hr - threshold_mm_hr

    if exceedance_ratio >= 1.25:
        hazard_level = "CRITICAL"
    elif exceedance_ratio >= 1.0:
        hazard_level = "WARNING"
    elif exceedance_ratio >= 0.75:
        hazard_level = "WATCH"
    else:
        hazard_level = "LOW"

    return {
        "duration_hours": round(float(duration_hours), 3),
        "intensity_mm_hr": round(float(intensity_mm_hr), 4),
        "threshold_intensity_mm_hr": round(threshold_mm_hr, 4),
        "exceedance_ratio": round(exceedance_ratio, 4),
        "margin_mm_hr": round(margin_mm_hr, 4),
        "exceeded": exceedance_ratio >= 1.0,
        "hazard_level": hazard_level,
    }


def analyze_precip_event(
    hourly_precip_mm: Sequence[float],
    antecedent_daily_mm: Sequence[float],
    decay: float = 0.85,
    alpha: float = 14.82,
    beta: float = -0.39,
    api_reference_mm: float = 50.0,
) -> Dict[str, Any]:
    """Scan multi-duration rolling windows and couple API soil-moisture modulation."""
    api_mm = compute_api(antecedent_daily_mm, decay=decay)
    saturation_factor = min(max(api_mm / max(api_reference_mm, 1e-6), 0.0), 1.0)
    # Wet antecedent soil lowers the triggering threshold by up to 35%
    effective_alpha = alpha * max(0.5, 1.0 - 0.35 * saturation_factor)

    n = len(hourly_precip_mm)
    if n == 0:
        return {
            "api_mm": round(api_mm, 3),
            "effective_alpha": round(effective_alpha, 4),
            "peak_exceedance_ratio": 0.0,
            "hazard_level": "LOW",
            "critical_window": None,
        }

    prefix = [0.0] * (n + 1)
    for i, val in enumerate(hourly_precip_mm):
        prefix[i + 1] = prefix[i] + max(0.0, float(val))

    candidate_durations = sorted({1, 3, 6, 12, 24, 48, 72, n})
    best_eval: Dict[str, Any] | None = None

    for dur in candidate_durations:
        if dur <= 0 or dur > n:
            continue
        max_sum = max(prefix[i + dur] - prefix[i] for i in range(n - dur + 1))
        mean_intensity = max_sum / float(dur)
        ev = evaluate_id_threshold(
            duration_hours=float(dur),
            intensity_mm_hr=mean_intensity,
            alpha=effective_alpha,
            beta=beta,
        )
        ev["cumulative_precip_mm"] = round(max_sum, 3)
        if best_eval is None or ev["exceedance_ratio"] > best_eval["exceedance_ratio"]:
            best_eval = ev

    assert best_eval is not None
    return {
        "api_mm": round(api_mm, 3),
        "saturation_factor": round(saturation_factor, 4),
        "effective_alpha": round(effective_alpha, 4),
        "peak_exceedance_ratio": best_eval["exceedance_ratio"],
        "hazard_level": best_eval["hazard_level"],
        "triggered": best_eval["exceeded"],
        "critical_window": best_eval,
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-json", help="Optional JSON string with event data")
    args = parser.parse_args(argv)

    if not args.input_json:
        sample_hourly = [2.0, 5.5, 14.0, 18.5, 11.0, 6.0]
        sample_daily = [28.0, 15.0, 8.0, 4.0, 0.0]
        print(json.dumps(analyze_precip_event(sample_hourly, sample_daily)))
        return 0

    payload = json.loads(args.input_json)
    print(
        json.dumps(
            analyze_precip_event(
                hourly_precip_mm=payload.get("hourly_precip_mm", []),
                antecedent_daily_mm=payload.get("antecedent_daily_mm", []),
                decay=float(payload.get("decay", 0.85)),
                alpha=float(payload.get("alpha", 14.82)),
                beta=float(payload.get("beta", -0.39)),
            )
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
