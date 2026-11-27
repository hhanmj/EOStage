#!/usr/bin/env python3
"""PyQGIS layout, UTM/equal-area CRS recommender, and publication cartography validator."""
from __future__ import annotations

import argparse
import json
import math
import sys
from typing import Any, Dict, List, Sequence, Tuple


def is_projected_crs(epsg_code: int) -> bool:
    return epsg_code != 4326 and epsg_code > 2000


def recommend_utm_epsg(lon: float, lat: float) -> int:
    """Compute the WGS84 UTM EPSG code (32601..32660 North, 32701..32760 South) for (lon, lat)."""
    zone = int(math.floor((float(lon) + 180.0) / 6.0) % 60) + 1
    return (32600 if float(lat) >= 0.0 else 32700) + zone


def recommend_cartographic_spec(
    bbox: Tuple[float, float, float, float],
    value_range: Tuple[float, float] = (-35.0, 20.0),
    dpi: int = 300,
) -> Dict[str, Any]:
    """Recommend projected CRS, symmetric diverging color limits, and scalebar step for a map bbox."""
    west, south, east, north = (float(x) for x in bbox)
    center_lon = 0.5 * (west + east)
    center_lat = 0.5 * (south + north)
    utm_epsg = recommend_utm_epsg(center_lon, center_lat)

    # Approximate east-west width in km
    width_km = max(
        0.1,
        abs(east - west) * 111.32 * max(0.1, math.cos(math.radians(center_lat))),
    )
    raw_segment_km = width_km / 5.0
    nice_steps_km = [0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0]
    scalebar_step_km = min(nice_steps_km, key=lambda s: abs(s - raw_segment_km))

    vmin, vmax = float(value_range[0]), float(value_range[1])
    sym_limit = round(max(abs(vmin), abs(vmax)), 2)

    return {
        "recommended_epsg": utm_epsg,
        "is_projected": is_projected_crs(utm_epsg),
        "center_lon_lat": [round(center_lon, 5), round(center_lat, 5)],
        "approx_width_km": round(width_km, 2),
        "scalebar_step_km": scalebar_step_km,
        "diverging_colorbar_limits": [-sym_limit, sym_limit],
        "publication_dpi": max(300, int(dpi)),
        "publication_ready": is_projected_crs(utm_epsg) and int(dpi) >= 300,
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-json", help="Optional JSON string with bbox and value_range")
    args = parser.parse_args(argv)

    if not args.input_json:
        print(
            json.dumps(
                recommend_cartographic_spec(
                    bbox=(117.10, 31.70, 117.45, 31.95),
                    value_range=(-42.5, 18.0),
                    dpi=300,
                )
            )
        )
        return 0

    payload = json.loads(args.input_json)
    bbox = tuple(payload.get("bbox", [117.10, 31.70, 117.45, 31.95]))
    vrange = tuple(payload.get("value_range", [-35.0, 20.0]))
    dpi = int(payload.get("dpi", 300))
    print(json.dumps(recommend_cartographic_spec(bbox=bbox, value_range=vrange, dpi=dpi)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
