#!/usr/bin/env python3
"""Build and verify a self-contained Unix (macOS / Linux) native GIS runtime.

Stages `runtime/gis` (private GDAL/PROJ dynamic library closure + `share/proj`
and `share/gdal` data directories + `manifest.json`) and optionally
`runtime/vertical-grids` (`EGM96` + `EGM2008` + `manifest.json`) for macOS
(`.dylib`) and Linux (`.so`).

Key guarantees enforced by `crates/insar-gis` and `scripts/native_gis_package_contract.mjs`:
1. Zero symbolic links inside `runtime/gis` (all staged `.dylib`/`.so` and data
   files are dereferenced regular files).
2. Private library search path:
   - macOS: rewrites Mach-O install names and non-system dependency paths to
     `@loader_path/<soname>` via `install_name_tool` and re-signs modified
     binaries with ad-hoc `codesign --force --sign -` (required on Apple Silicon).
   - Linux: sets `RPATH`/`RUNPATH` to `$ORIGIN` via `patchelf` when available.
3. Excludes forbidden files (`.py`, `.pyc`, `.sh`, `.exe`, `gdalplugins`, and
   vertical-grid `.tif` payloads inside `share/proj`).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Iterable

FORBIDDEN_DATA_SUFFIXES = (
    ".py",
    ".pyc",
    ".exe",
    ".bat",
    ".cmd",
    ".ps1",
    ".sh",
)

VERTICAL_GRID_NAME_PATTERN = re.compile(
    r"(egm96|egm08|egm2008|geoid|vertcon|napgd|ostn|osgm)",
    re.IGNORECASE,
)

LINUX_GLIBC_EXCLUDED_PREFIXES = (
    "linux-vdso.so",
    "ld-linux",
    "ld-musl",
    "libc.so",
    "libm.so",
    "libdl.so",
    "libpthread.so",
    "librt.so",
    "libresolv.so",
    "libutil.so",
    "libnss_",
    "libgcc_s.so",
    "libstdc++.so",
)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest().lower()


def normalize_os_tag(sys_platform: str | None = None) -> str:
    plat = (sys_platform or sys.platform).lower()
    if plat.startswith("darwin") or plat == "macos":
        return "macos"
    if plat.startswith("linux"):
        return "linux"
    raise ValueError(f"Unsupported Unix platform for native GIS builder: {plat}")


def normalize_arch_tag(machine: str | None = None) -> str:
    raw = (machine or platform.machine()).lower()
    if raw in ("arm64", "aarch64"):
        return "aarch64"
    if raw in ("x86_64", "amd64"):
        return "x86_64"
    return raw or "unknown"


def detect_library_version(lib_path: Path, fallback: str) -> str:
    real_name = lib_path.resolve().name
    match = re.search(r"(\d+\.\d+\.\d+)", real_name)
    if match:
        return match.group(1)
    match_short = re.search(r"(\d+\.\d+)", real_name)
    if match_short:
        return match_short.group(1)
    return fallback


def is_macos_system_library(dep_path: str) -> bool:
    return dep_path.startswith("/System/Library/") or dep_path.startswith("/usr/lib/")


def is_linux_system_library(dep_name: str) -> bool:
    base = os.path.basename(dep_name)
    return any(base.startswith(prefix) for prefix in LINUX_GLIBC_EXCLUDED_PREFIXES)


def parse_otool_dependencies(otool_output: str, self_path: Path) -> list[str]:
    deps: list[str] = []
    lines = otool_output.splitlines()
    for idx, raw_line in enumerate(lines):
        if idx == 0 and raw_line.strip().endswith(":"):
            continue
        stripped = raw_line.strip()
        if not stripped:
            continue
        dep = stripped.split(" (", 1)[0].strip()
        if not dep or dep == str(self_path):
            continue
        deps.append(dep)
    return deps


def parse_ldd_dependencies(ldd_output: str) -> list[tuple[str, str]]:
    """Return list of (soname, resolved_absolute_path) from `ldd` output."""
    results: list[tuple[str, str]] = []
    for raw_line in ldd_output.splitlines():
        line = raw_line.strip()
        if not line or "=>" not in line:
            continue
        left, right = line.split("=>", 1)
        soname = left.strip()
        right_part = right.strip().split(" (", 1)[0].strip()
        if not right_part or right_part == "not found":
            continue
        if os.path.isabs(right_part):
            results.append((soname, right_part))
    return results


def resolve_macos_dependency_path(
    dep: str,
    referencing_lib: Path,
    search_dirs: Iterable[Path],
) -> Path | None:
    if is_macos_system_library(dep):
        return None
    if os.path.isabs(dep):
        candidate = Path(dep)
        if candidate.exists():
            return candidate
    if dep.startswith("@loader_path/"):
        rel = dep[len("@loader_path/") :]
        candidate = referencing_lib.parent / rel
        if candidate.exists():
            return candidate
    if dep.startswith("@rpath/") or dep.startswith("@executable_path/"):
        leaf = os.path.basename(dep)
        for sdir in search_dirs:
            candidate = sdir / leaf
            if candidate.exists():
                return candidate
    return None


def collect_unix_library_closure(
    gdal_lib: Path,
    proj_lib: Path,
    os_tag: str,
    extra_search_dirs: Iterable[Path] = (),
) -> tuple[dict[str, Path], str, str]:
    """Discover transitive private dynamic library closure for GDAL and PROJ.

    Returns `(staged_name_to_real_source_path, gdal_staged_name, proj_staged_name)`.
    Every entry in `staged_name_to_real_source_path` maps a flat library filename
    inside `runtime/gis/lib/` to a real regular file on disk.
    """
    search_dirs: list[Path] = []
    for candidate_dir in [gdal_lib.parent, proj_lib.parent, *extra_search_dirs]:
        if candidate_dir not in search_dirs and candidate_dir.is_dir():
            search_dirs.append(candidate_dir)

    staged: dict[str, Path] = {}
    queue: list[tuple[str, Path]] = [
        (gdal_lib.name, gdal_lib.resolve()),
        (proj_lib.name, proj_lib.resolve()),
    ]
    visited_real: dict[Path, str] = {}

    otool_bin = shutil.which("otool") if os_tag == "macos" else None
    ldd_bin = shutil.which("ldd") if os_tag == "linux" else None

    while queue:
        preferred_name, real_path = queue.pop(0)
        if not real_path.is_file():
            raise FileNotFoundError(f"Required native GIS library not found: {real_path}")

        staged_name = preferred_name
        if staged_name not in staged:
            staged[staged_name] = real_path
        if real_path in visited_real:
            continue
        visited_real[real_path] = staged_name

        if os_tag == "macos" and otool_bin:
            proc = subprocess.run(
                [otool_bin, "-L", str(real_path)],
                check=False,
                capture_output=True,
                text=True,
            )
            if proc.returncode == 0:
                for dep in parse_otool_dependencies(proc.stdout, real_path):
                    resolved = resolve_macos_dependency_path(dep, real_path, search_dirs)
                    if resolved is not None and resolved.is_file():
                        dep_name = os.path.basename(dep)
                        queue.append((dep_name, resolved.resolve()))
        elif os_tag == "linux" and ldd_bin:
            proc = subprocess.run(
                [ldd_bin, str(real_path)],
                check=False,
                capture_output=True,
                text=True,
            )
            if proc.returncode == 0:
                for soname, resolved_str in parse_ldd_dependencies(proc.stdout):
                    if is_linux_system_library(soname):
                        continue
                    resolved = Path(resolved_str)
                    if resolved.is_file():
                        queue.append((os.path.basename(soname), resolved.resolve()))

    return staged, gdal_lib.name, proj_lib.name


def rewrite_macos_loader_paths(lib_dir: Path, staged_names: set[str]) -> None:
    install_name_tool = shutil.which("install_name_tool")
    codesign = shutil.which("codesign")
    otool = shutil.which("otool")
    if not install_name_tool or not otool:
        return

    for lib_name in sorted(staged_names):
        lib_path = lib_dir / lib_name
        os.chmod(lib_path, 0o755)
        proc = subprocess.run(
            [otool, "-L", str(lib_path)],
            check=False,
            capture_output=True,
            text=True,
        )
        if proc.returncode != 0:
            continue
        changes: list[str] = ["-id", f"@loader_path/{lib_name}"]
        for dep in parse_otool_dependencies(proc.stdout, lib_path):
            if is_macos_system_library(dep):
                continue
            dep_base = os.path.basename(dep)
            if dep_base in staged_names and dep != f"@loader_path/{dep_base}":
                changes.extend(["-change", dep, f"@loader_path/{dep_base}"])
        subprocess.run(
            [install_name_tool, *changes, str(lib_path)],
            check=False,
            capture_output=True,
            text=True,
        )
        if codesign:
            subprocess.run(
                [codesign, "--force", "--sign", "-", str(lib_path)],
                check=False,
                capture_output=True,
                text=True,
            )


def rewrite_linux_rpaths(lib_dir: Path, staged_names: set[str]) -> None:
    patchelf = shutil.which("patchelf")
    for lib_name in sorted(staged_names):
        lib_path = lib_dir / lib_name
        os.chmod(lib_path, 0o755)
        if patchelf:
            subprocess.run(
                [patchelf, "--set-rpath", "$ORIGIN", str(lib_path)],
                check=False,
                capture_output=True,
                text=True,
            )


def copy_filtered_data_tree(
    source_dir: Path,
    dest_dir: Path,
    *,
    exclude_vertical_grids: bool,
) -> None:
    if not source_dir.is_dir():
        raise FileNotFoundError(f"Required GIS data directory is missing: {source_dir}")
    dest_dir.mkdir(parents=True, exist_ok=True)
    for root, dirs, files in os.walk(source_dir, followlinks=False):
        root_path = Path(root)
        dirs[:] = [
            d
            for d in sorted(dirs)
            if d.lower() != "gdalplugins" and not (root_path / d).is_symlink()
        ]
        rel_root = root_path.relative_to(source_dir)
        target_root = dest_dir / rel_root
        target_root.mkdir(parents=True, exist_ok=True)
        for fname in sorted(files):
            src_file = root_path / fname
            if src_file.is_symlink() or not src_file.is_file():
                continue
            lower_name = fname.lower()
            if lower_name.endswith(FORBIDDEN_DATA_SUFFIXES):
                continue
            if (
                exclude_vertical_grids
                and lower_name.endswith(".tif")
                and VERTICAL_GRID_NAME_PATTERN.search(lower_name)
            ):
                continue
            shutil.copyfile(src_file, target_root / fname)


def build_vertical_grid_manifest(
    vertical_grid_root: Path,
    egm96_source: Path | None,
    egm2008_source: Path | None,
) -> Path | None:
    egm96_target = vertical_grid_root / "egm96" / "us_nga_egm96_15.tif"
    egm2008_target = vertical_grid_root / "egm2008" / "us_nga_egm08_25.tif"

    if egm96_source and egm96_source.is_file():
        egm96_target.parent.mkdir(parents=True, exist_ok=True)
        if egm96_source.resolve() != egm96_target.resolve() if egm96_target.exists() else True:
            shutil.copyfile(egm96_source.resolve(), egm96_target)

    if egm2008_source and egm2008_source.is_file():
        egm2008_target.parent.mkdir(parents=True, exist_ok=True)
        if (
            egm2008_source.resolve() != egm2008_target.resolve()
            if egm2008_target.exists()
            else True
        ):
            shutil.copyfile(egm2008_source.resolve(), egm2008_target)

    if not egm96_target.is_file() or not egm2008_target.is_file():
        return None

    manifest = [
        {
            "grid_id": "egm96-global",
            "model": "EGM96",
            "vertical_crs": "EPSG:5773",
            "target_vertical_crs": "EPSG:4979",
            "source_horizontal_crs": "EPSG:4326",
            "source_vertical_crs": "EPSG:5773",
            "target_horizontal_crs": "EPSG:4326",
            "direction": "geoid_to_ellipsoid",
            "filename": "egm96/us_nga_egm96_15.tif",
            "version": "1.0.0",
            "source": "https://cdn.proj.org/us_nga_egm96_15.tif",
            "sha256": sha256_file(egm96_target),
            "expected_bytes": egm96_target.stat().st_size,
            "coverage": "Global (-90..90, -180..180)",
            "resolution": "15 arc-minute",
            "license": "Public Domain (NGA / PROJ-data)",
            "bundled": True,
            "production_admitted": True,
            "applicable_transformations": ["EGM96_TO_WGS84_ELLIPSOID"],
        },
        {
            "grid_id": "egm2008-global",
            "model": "EGM2008",
            "vertical_crs": "EPSG:3855",
            "target_vertical_crs": "EPSG:4979",
            "source_horizontal_crs": "EPSG:4326",
            "source_vertical_crs": "EPSG:3855",
            "target_horizontal_crs": "EPSG:4326",
            "direction": "geoid_to_ellipsoid",
            "filename": "egm2008/us_nga_egm08_25.tif",
            "version": "1.0.0",
            "source": "https://cdn.proj.org/us_nga_egm08_25.tif",
            "sha256": sha256_file(egm2008_target),
            "expected_bytes": egm2008_target.stat().st_size,
            "coverage": "Global (-90..90, -180..180)",
            "resolution": "2.5 arc-minute",
            "license": "Public Domain (NGA / PROJ-data)",
            "bundled": True,
            "production_admitted": True,
            "applicable_transformations": ["EGM2008_TO_WGS84_ELLIPSOID"],
        },
    ]
    manifest_path = vertical_grid_root / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest_path


def stage_unix_native_gis_runtime(
    *,
    gdal_lib: Path,
    proj_lib: Path,
    proj_data: Path,
    gdal_data: Path,
    output_root: Path,
    vertical_grid_root: Path | None = None,
    egm96_source: Path | None = None,
    egm2008_source: Path | None = None,
    os_tag: str | None = None,
    arch_tag: str | None = None,
    gdal_version: str | None = None,
    proj_version: str | None = None,
    runtime_id: str | None = None,
    production_admitted: bool = False,
) -> dict:
    resolved_os = os_tag or normalize_os_tag()
    resolved_arch = arch_tag or normalize_arch_tag()
    resolved_gdal_ver = gdal_version or detect_library_version(gdal_lib, "3.10.0")
    resolved_proj_ver = proj_version or detect_library_version(proj_lib, "9.5.0")

    base_runtime_id = (
        runtime_id
        or f"native-gis-gdal{resolved_gdal_ver}-proj{resolved_proj_ver}-{resolved_os}-{resolved_arch}"
    )
    if not production_admitted and not base_runtime_id.endswith("-unverified"):
        base_runtime_id = f"{base_runtime_id}-unverified"

    closure, gdal_staged_name, proj_staged_name = collect_unix_library_closure(
        gdal_lib=gdal_lib,
        proj_lib=proj_lib,
        os_tag=resolved_os,
    )

    if output_root.exists():
        shutil.rmtree(output_root)
    lib_out = output_root / "lib"
    proj_out = output_root / "share" / "proj"
    gdal_out = output_root / "share" / "gdal"
    lib_out.mkdir(parents=True, exist_ok=True)

    for staged_name, real_src in sorted(closure.items()):
        dest_file = lib_out / staged_name
        shutil.copyfile(real_src, dest_file)
        os.chmod(dest_file, 0o755)

    if resolved_os == "macos":
        rewrite_macos_loader_paths(lib_out, set(closure.keys()))
    elif resolved_os == "linux":
        rewrite_linux_rpaths(lib_out, set(closure.keys()))

    copy_filtered_data_tree(proj_data, proj_out, exclude_vertical_grids=True)
    copy_filtered_data_tree(gdal_data, gdal_out, exclude_vertical_grids=False)

    if not (proj_out / "proj.db").is_file():
        raise FileNotFoundError(f"Staged PROJ data directory is missing proj.db: {proj_out}")

    if vertical_grid_root is not None:
        build_vertical_grid_manifest(vertical_grid_root, egm96_source, egm2008_source)

    manifest_files = []
    for file_path in sorted(output_root.rglob("*")):
        if file_path.is_dir():
            continue
        rel = file_path.relative_to(output_root).as_posix()
        if rel == "manifest.json" or rel.startswith("vertical-grids/"):
            continue
        manifest_files.append(
            {
                "relative_path": rel,
                "bytes": file_path.stat().st_size,
                "sha256": sha256_file(file_path),
            }
        )
    manifest_files.sort(key=lambda item: item["relative_path"])

    manifest = {
        "schema_version": 1,
        "runtime_id": base_runtime_id,
        "gdal_version": resolved_gdal_ver,
        "proj_version": resolved_proj_ver,
        "gdal_library": f"lib/{gdal_staged_name}",
        "proj_library": f"lib/{proj_staged_name}",
        "proj_data_dir": "share/proj",
        "gdal_data_dir": "share/gdal",
        "proj_network": "disabled",
        "drivers": ["GTiff", "ENVI"],
        "files": manifest_files,
    }
    manifest_path = output_root / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest


def auto_discover_unix_inputs(
    source_root: Path | None,
    os_tag: str,
) -> tuple[Path, Path, Path, Path]:
    ext = ".dylib" if os_tag == "macos" else ".so"
    candidate_roots: list[Path] = []
    if source_root:
        candidate_roots.append(source_root)
    if os_tag == "macos":
        for prefix in ("/opt/homebrew", "/usr/local"):
            p = Path(prefix)
            if p.is_dir() and p not in candidate_roots:
                candidate_roots.append(p)
    else:
        for prefix in ("/usr", "/usr/local"):
            p = Path(prefix)
            if p.is_dir() and p not in candidate_roots:
                candidate_roots.append(p)

    lib_dirs: list[Path] = []
    for root in candidate_roots:
        for sub in (
            "lib",
            "lib64",
            "lib/x86_64-linux-gnu",
            "lib/aarch64-linux-gnu",
            "opt/gdal/lib",
            "opt/proj/lib",
        ):
            d = root / sub
            if d.is_dir() and d not in lib_dirs:
                lib_dirs.append(d)

    def find_lib(stem: str) -> Path:
        for d in lib_dirs:
            exact = d / f"{stem}{ext}"
            if exact.exists():
                return exact
            matches = sorted(d.glob(f"{stem}*{ext}*"))
            for m in matches:
                if m.is_file():
                    return m
        raise FileNotFoundError(f"Could not locate {stem}{ext} in {lib_dirs}")

    def find_share(subdir: str, marker: str) -> Path:
        for root in candidate_roots:
            for candidate in (
                root / "share" / subdir,
                root / "opt" / subdir / "share" / subdir,
            ):
                if (candidate / marker).is_file():
                    return candidate
        raise FileNotFoundError(f"Could not locate share/{subdir}/{marker} in {candidate_roots}")

    return (
        find_lib("libgdal"),
        find_lib("libproj"),
        find_share("proj", "proj.db"),
        find_share("gdal", "gdalvrt.xsd"),
    )


def main(argv: list[str] | None = None) -> int:
    repo_root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(
        description="Stage and verify Unix (macOS/Linux) native GIS runtime."
    )
    parser.add_argument("--source-root", type=Path, default=None)
    parser.add_argument("--gdal-lib", type=Path, default=None)
    parser.add_argument("--proj-lib", type=Path, default=None)
    parser.add_argument("--proj-data", type=Path, default=None)
    parser.add_argument("--gdal-data", type=Path, default=None)
    parser.add_argument("--egm96-grid", type=Path, default=None)
    parser.add_argument("--egm2008-grid", type=Path, default=None)
    parser.add_argument("--output-root", type=Path, default=repo_root / "runtime" / "gis")
    parser.add_argument(
        "--vertical-grid-root",
        type=Path,
        default=repo_root / "runtime" / "vertical-grids",
    )
    parser.add_argument("--os-tag", type=str, default=None, choices=["macos", "linux"])
    parser.add_argument("--arch-tag", type=str, default=None)
    parser.add_argument("--gdal-version", type=str, default=None)
    parser.add_argument("--proj-version", type=str, default=None)
    parser.add_argument("--runtime-id", type=str, default=None)
    parser.add_argument("--production-admitted", action="store_true")
    parser.add_argument("--skip-contract-verifier", action="store_true")
    args = parser.parse_args(argv)

    os_tag = args.os_tag or normalize_os_tag()
    if args.gdal_lib and args.proj_lib and args.proj_data and args.gdal_data:
        gdal_lib = args.gdal_lib
        proj_lib = args.proj_lib
        proj_data = args.proj_data
        gdal_data = args.gdal_data
    else:
        disc_gdal, disc_proj, disc_proj_data, disc_gdal_data = auto_discover_unix_inputs(
            args.source_root, os_tag
        )
        gdal_lib = args.gdal_lib or disc_gdal
        proj_lib = args.proj_lib or disc_proj
        proj_data = args.proj_data or disc_proj_data
        gdal_data = args.gdal_data or disc_gdal_data

    manifest = stage_unix_native_gis_runtime(
        gdal_lib=gdal_lib,
        proj_lib=proj_lib,
        proj_data=proj_data,
        gdal_data=gdal_data,
        output_root=args.output_root,
        vertical_grid_root=args.vertical_grid_root,
        egm96_source=args.egm96_grid,
        egm2008_source=args.egm2008_grid,
        os_tag=os_tag,
        arch_tag=args.arch_tag,
        gdal_version=args.gdal_version,
        proj_version=args.proj_version,
        runtime_id=args.runtime_id,
        production_admitted=args.production_admitted,
    )

    if not args.skip_contract_verifier:
        node_bin = shutil.which("node")
        verifier = repo_root / "scripts" / "native_gis_package_contract.mjs"
        if node_bin and verifier.is_file():
            cmd = [node_bin, str(verifier), "--runtime-root", str(args.output_root)]
            if (
                args.vertical_grid_root
                and (args.vertical_grid_root / "manifest.json").is_file()
            ):
                cmd.extend(["--vertical-grid-root", str(args.vertical_grid_root)])
            subprocess.run(cmd, check=True)

    print(
        json.dumps(
            {
                "ok": True,
                "runtime_id": manifest["runtime_id"],
                "output_root": str(args.output_root),
                "file_count": len(manifest["files"]),
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
