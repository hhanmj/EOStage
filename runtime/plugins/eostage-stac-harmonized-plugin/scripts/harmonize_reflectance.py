#!/usr/bin/env python3
"""Cross-sensor reflectance scaling and HLS bandpass adjustment for Landsat C2 L2 and Sentinel-2 L2A."""
from __future__ import annotations

import argparse
import json
import sys
from typing import Any, Dict, Sequence, Tuple

# NASA HLS (Claverie et al., 2018) Sentinel-2 MSI -> Landsat-8/9 OLI bandpass coefficients (slope, intercept)
HLS_BANDPASS_COEFFS: Dict[str, Tuple[float, float]] = {
    "blue": (0.9778, -0.0040),
    "green": (1.0053, -0.0009),
    "red": (0.9765, 0.0009),
    "nir08": (0.9983, -0.0001),
    "swir16": (0.9987, -0.0011),
    "swir22": (1.0030, -0.0012),
}


def scale_landsat_c2_sr(dn: float) -> float:
    return dn * 0.0000275 - 0.2


def scale_sentinel2_l2a_sr(
    dn: float, boa_add_offset: float = -1000.0, quant: float = 10000.0
) -> float:
    return (dn + boa_add_offset) / quant


def adjust_s2_to_oli_bandpass(s2_sr: float, band: str) -> float:
    """Apply NASA HLS linear bandpass adjustment from Sentinel-2 MSI to OLI equivalent."""
    slope, intercept = HLS_BANDPASS_COEFFS.get(band.lower(), (1.0, 0.0))
    return slope * float(s2_sr) + intercept


def decode_landsat_qa_pixel(qa_value: int) -> Dict[str, bool]:
    """Decode Landsat Collection 2 Level-2 QA_PIXEL 16-bit flag word."""
    val = int(qa_value)
    fill = bool(val & (1 << 0))
    dilated_cloud = bool(val & (1 << 1))
    cirrus = bool(val & (1 << 2))
    cloud = bool(val & (1 << 3))
    cloud_shadow = bool(val & (1 << 4))
    snow = bool(val & (1 << 5))
    clear = not (fill or dilated_cloud or cirrus or cloud or cloud_shadow or snow)
    return {
        "fill": fill,
        "dilated_cloud": dilated_cloud,
        "cirrus": cirrus,
        "cloud": cloud,
        "cloud_shadow": cloud_shadow,
        "snow": snow,
        "is_clear": clear,
    }


def decode_sentinel2_scl(scl_value: int) -> Dict[str, Any]:
    """Decode Sentinel-2 L2A Scene Classification Layer (SCL) class."""
    cls = int(scl_value)
    clear_classes = {4, 5, 6}  # vegetation, not_vegetated, water
    return {
        "scl_class": cls,
        "is_clear": cls in clear_classes,
    }


def harmonize_observation_pair(
    landsat_dn: Dict[str, float],
    sentinel2_dn: Dict[str, float],
    qa_pixel: int = 21824,
    scl: int = 4,
    boa_add_offset: float = -1000.0,
) -> Dict[str, Any]:
    """Harmonize Landsat C2 L2 and Sentinel-2 L2A DN observations into aligned OLI surface reflectance."""
    l_qa = decode_landsat_qa_pixel(qa_pixel)
    s2_qa = decode_sentinel2_scl(scl)

    bands: Dict[str, Dict[str, float]] = {}
    for band in sorted(set(landsat_dn.keys()) & set(sentinel2_dn.keys())):
        l_sr = max(0.0, min(1.0, scale_landsat_c2_sr(float(landsat_dn[band]))))
        s2_raw_sr = scale_sentinel2_l2a_sr(
            float(sentinel2_dn[band]), boa_add_offset=boa_add_offset
        )
        s2_oli_sr = max(0.0, min(1.0, adjust_s2_to_oli_bandpass(s2_raw_sr, band)))
        bands[band] = {
            "landsat_sr": round(l_sr, 6),
            "sentinel2_raw_sr": round(s2_raw_sr, 6),
            "sentinel2_harmonized_sr": round(s2_oli_sr, 6),
            "abs_bias": round(abs(l_sr - s2_oli_sr), 6),
        }

    ndvi_landsat = None
    ndvi_s2 = None
    if "red" in bands and "nir08" in bands:
        l_red, l_nir = bands["red"]["landsat_sr"], bands["nir08"]["landsat_sr"]
        s_red, s_nir = (
            bands["red"]["sentinel2_harmonized_sr"],
            bands["nir08"]["sentinel2_harmonized_sr"],
        )
        if (l_nir + l_red) > 1e-6:
            ndvi_landsat = round((l_nir - l_red) / (l_nir + l_red), 6)
        if (s_nir + s_red) > 1e-6:
            ndvi_s2 = round((s_nir - s_red) / (s_nir + s_red), 6)

    return {
        "landsat_clear": l_qa["is_clear"],
        "sentinel2_clear": s2_qa["is_clear"],
        "usable_pair": l_qa["is_clear"] and s2_qa["is_clear"],
        "bands": bands,
        "ndvi_landsat": ndvi_landsat,
        "ndvi_sentinel2_harmonized": ndvi_s2,
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-json", help="Optional JSON string with sensor DN values")
    args = parser.parse_args(argv)

    if not args.input_json:
        sample = harmonize_observation_pair(
            landsat_dn={"red": 10200.0, "nir08": 19500.0},
            sentinel2_dn={"red": 1820.0, "nir08": 4360.0},
        )
        print(json.dumps(sample))
        return 0

    payload = json.loads(args.input_json)
    print(
        json.dumps(
            harmonize_observation_pair(
                landsat_dn=payload.get("landsat_dn", {}),
                sentinel2_dn=payload.get("sentinel2_dn", {}),
                qa_pixel=int(payload.get("qa_pixel", 21824)),
                scl=int(payload.get("scl", 4)),
                boa_add_offset=float(payload.get("boa_add_offset", -1000.0)),
            )
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
