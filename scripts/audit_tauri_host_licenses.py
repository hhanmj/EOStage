"""Offline license-metadata audit for the Tauri desktop host dependency closure.

This tool intentionally covers the Tauri/Rust/npm host only. The PyInstaller
Compatibility Worker has a different artifact closure and must be audited from
its actual PyInstaller analysis rather than from the whole development venv.

The audit makes two narrow claims:

* every observed dependency has machine-readable license metadata (or a Cargo
  license_file);
* explicitly configured license expressions are surfaced as review-required.

It does not decide that any other license is legally approved for release.

``--mode l3`` / ``--mode distribution`` run the GPLv3 compatibility policy
for the GPL-3.0-or-later project (``scripts/license_policy.py``) over the
whole Windows-target Cargo resolve graph and the npm production dependencies.
Owner-granted, dated exceptions in ``contracts/license-exceptions.v1.json`` are
honored (as warnings) only in ``l3`` mode; ``distribution`` mode ignores them
and fails.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from collections import Counter, deque
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

import license_policy as policy

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_WINDOWS_TARGET = "x86_64-pc-windows-msvc"
REVIEW_REQUIRED_LICENSES = frozenset({"Hippocratic-2.1"})
DEFAULT_EXCEPTIONS_PATH = ROOT / "contracts" / "license-exceptions.v1.json"
POLICY_SCOPE = "gplv3-compatibility-license-policy-v1"
POLICY_MODES = ("l3", "distribution")
POLICY_EXIT_VIOLATION = 3


class LicenseAuditError(RuntimeError):
    """The audit input or local metadata command is invalid."""


@dataclass(frozen=True)
class DependencyLicense:
    ecosystem: str
    name: str
    version: str
    license: str
    license_file: str | None
    status: str
    license_file_sha256: str | None = None


def _license_status(license_expression: str, license_file: str | None) -> str:
    value = license_expression.strip()
    if not value and not license_file:
        return "missing_metadata"
    folded = value.casefold()
    if any(marker.casefold() in folded for marker in REVIEW_REQUIRED_LICENSES):
        return "review_required"
    return "observed"


def audit_npm_lock(document: dict[str, Any]) -> list[DependencyLicense]:
    packages = document.get("packages")
    if not isinstance(packages, dict):
        raise LicenseAuditError("npm package-lock is missing the packages object")

    records: list[DependencyLicense] = []
    for package_path, raw in sorted(packages.items()):
        if not isinstance(package_path, str) or not package_path.startswith("node_modules/"):
            continue
        if not isinstance(raw, dict) or raw.get("dev") is True:
            continue
        version = raw.get("version")
        if not isinstance(version, str) or not version.strip():
            raise LicenseAuditError(f"npm package has no version metadata: {package_path}")
        name = package_path.rsplit("node_modules/", 1)[-1]
        license_expression = raw.get("license")
        if not isinstance(license_expression, str):
            license_expression = ""
        records.append(
            DependencyLicense(
                ecosystem="npm",
                name=name,
                version=version,
                license=license_expression.strip(),
                license_file=None,
                status=_license_status(license_expression, None),
            )
        )
    return records


def _normal_dependency_ids(node: dict[str, Any]) -> list[str]:
    dependency_ids: list[str] = []
    for dependency in node.get("deps", []):
        if not isinstance(dependency, dict):
            continue
        dep_kinds = dependency.get("dep_kinds")
        if not isinstance(dep_kinds, list):
            continue
        if any(isinstance(kind, dict) and kind.get("kind") is None for kind in dep_kinds):
            package_id = dependency.get("pkg")
            if isinstance(package_id, str):
                dependency_ids.append(package_id)
    return dependency_ids


def audit_cargo_metadata(
    document: dict[str, Any],
    *,
    root_package_name: str = "eostage-desktop",
    extra_root_package_names: tuple[str, ...] = (),
) -> list[DependencyLicense]:
    raw_packages = document.get("packages")
    resolve = document.get("resolve")
    if not isinstance(raw_packages, list) or not isinstance(resolve, dict):
        raise LicenseAuditError("Cargo metadata is missing packages or resolve")
    raw_nodes = resolve.get("nodes")
    if not isinstance(raw_nodes, list):
        raise LicenseAuditError("Cargo metadata resolve graph is missing nodes")

    packages = {
        package["id"]: package
        for package in raw_packages
        if isinstance(package, dict) and isinstance(package.get("id"), str)
    }
    nodes = {
        node["id"]: node
        for node in raw_nodes
        if isinstance(node, dict) and isinstance(node.get("id"), str)
    }
    roots = [
        package
        for package in packages.values()
        if package.get("name") == root_package_name
    ]
    if len(roots) != 1:
        raise LicenseAuditError(
            f"expected exactly one Cargo root package named {root_package_name!r}, found {len(roots)}"
        )

    extra_roots = [package for package in packages.values() if package.get("name") in extra_root_package_names]
    if len(extra_roots) != len(extra_root_package_names):
        raise LicenseAuditError("missing or duplicated extra Cargo package root")

    reachable: set[str] = set()
    pending: deque[str] = deque([roots[0]["id"], *(root["id"] for root in extra_roots)])
    while pending:
        package_id = pending.popleft()
        if package_id in reachable:
            continue
        reachable.add(package_id)
        node = nodes.get(package_id)
        if node is None:
            raise LicenseAuditError(f"Cargo resolve graph is missing package node: {package_id}")
        pending.extend(_normal_dependency_ids(node))

    records: list[DependencyLicense] = []
    for package_id in sorted(reachable):
        package = packages.get(package_id)
        if package is None or package.get("source") is None:
            continue
        records.append(_cargo_record(package))
    return records


def _cargo_record(package: dict[str, Any], *, hash_license_file: bool = False) -> DependencyLicense:
    name = package.get("name")
    version = package.get("version")
    if not isinstance(name, str) or not isinstance(version, str):
        raise LicenseAuditError("Cargo dependency is missing name/version metadata")
    license_expression = package.get("license")
    if not isinstance(license_expression, str):
        license_expression = ""
    raw_license_file = package.get("license_file")
    license_file = raw_license_file if isinstance(raw_license_file, str) else None
    digest = None
    manifest_path = package.get("manifest_path")
    if hash_license_file and license_file and isinstance(manifest_path, str):
        digest = _sha256_file(Path(manifest_path).parent / license_file)
    return DependencyLicense(
        ecosystem="cargo",
        name=name,
        version=version,
        license=license_expression.strip(),
        license_file=license_file,
        status=_license_status(license_expression, license_file),
        license_file_sha256=digest,
    )


def run_cargo_metadata(
    repository_root: Path,
    *,
    target: str = DEFAULT_WINDOWS_TARGET,
) -> dict[str, Any]:
    command = [
        "cargo",
        "metadata",
        "--locked",
        "--offline",
        "--format-version",
        "1",
        "--filter-platform",
        target,
    ]
    process = subprocess.run(
        command,
        cwd=repository_root,
        capture_output=True,
        check=False,
    )
    if process.returncode != 0:
        stderr = process.stderr.decode("utf-8", errors="replace").strip()
        detail = stderr.splitlines()[-1] if stderr else "no diagnostic"
        raise LicenseAuditError(
            f"Cargo metadata failed with exit code {process.returncode}: {detail}"
        )
    try:
        return json.loads(process.stdout.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise LicenseAuditError("Cargo metadata did not return valid UTF-8 JSON") from exc


def build_report(
    npm_records: list[DependencyLicense],
    cargo_records: list[DependencyLicense],
    *,
    target: str = DEFAULT_WINDOWS_TARGET,
) -> dict[str, Any]:
    records = sorted(
        [*npm_records, *cargo_records],
        key=lambda record: (record.ecosystem, record.name.casefold(), record.version),
    )
    missing = [record for record in records if record.status == "missing_metadata"]
    review = [record for record in records if record.status == "review_required"]
    counts = Counter(record.ecosystem for record in records)
    return {
        "ok": not missing,
        "status": "invalid" if missing else ("review_required" if review else "metadata_complete"),
        "scope": "tauri-host-and-agent-license-metadata-v2",
        "target": target,
        "dependency_count": len(records),
        "ecosystem_counts": dict(sorted(counts.items())),
        "missing_metadata_count": len(missing),
        "review_required_count": len(review),
        "review_required_licenses": sorted(REVIEW_REQUIRED_LICENSES),
        "review_required": [asdict(record) for record in review],
        "missing_metadata": [asdict(record) for record in missing],
        "worker_coverage": {
            "covered": False,
            "reason": (
                "Compatibility Worker must be audited from the actual PyInstaller "
                "artifact closure, not the development virtual environment."
            ),
        },
        "approval_claimed": False,
        "dependencies": [asdict(record) for record in records],
    }


def report_exit_code(report: dict[str, Any], *, fail_on_review_required: bool) -> int:
    if report.get("missing_metadata_count", 0):
        return 1
    if fail_on_review_required and report.get("review_required_count", 0):
        return 2
    return 0


def _sha256_file(path: Path) -> str | None:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError:
        return None


def cargo_resolve_records(document: dict[str, Any]) -> list[DependencyLicense]:
    """Every non-workspace package in the Cargo resolve graph (all dependency kinds)."""
    raw_packages = document.get("packages")
    resolve = document.get("resolve")
    if not isinstance(raw_packages, list) or not isinstance(resolve, dict):
        raise LicenseAuditError("Cargo metadata is missing packages or resolve")
    raw_nodes = resolve.get("nodes")
    if not isinstance(raw_nodes, list):
        raise LicenseAuditError("Cargo metadata resolve graph is missing nodes")
    packages = {
        package["id"]: package
        for package in raw_packages
        if isinstance(package, dict) and isinstance(package.get("id"), str)
    }
    workspace_members = document.get("workspace_members")
    if not isinstance(workspace_members, list):
        raise LicenseAuditError("Cargo metadata is missing workspace_members")
    members = {member for member in workspace_members if isinstance(member, str)}
    records: list[DependencyLicense] = []
    for node in raw_nodes:
        package_id = node.get("id") if isinstance(node, dict) else None
        package = packages.get(package_id) if isinstance(package_id, str) else None
        if package is None:
            raise LicenseAuditError(f"Cargo resolve node has no package metadata: {package_id}")
        # Only EOStage's own workspace crates are skipped: vendored path crates and
        # [patch] path overrides have no source either but are third-party code.
        if package_id in members:
            continue
        records.append(_cargo_record(package, hash_license_file=True))
    return sorted(records, key=lambda record: (record.name.casefold(), record.version))


_EXCEPTION_FIELDS = (
    "name",
    "ecosystem",
    "license",
    "reason",
    "granted_by",
    "granted_on",
    "must_resolve_before",
    "issue",
)
_REVIEW_FIELDS = (
    "name",
    "ecosystem",
    "version",
    "license_file",
    "license_file_sha256",
    "classified_as",
    "reason",
    "reviewed_by",
    "reviewed_on",
)
_ISO_DATE = re.compile(r"^\d{4}-\d{2}-\d{2}$")


def _require_entries(
    document: dict[str, Any], key: str, fields: tuple[str, ...], date_fields: tuple[str, ...]
) -> list[dict[str, Any]]:
    entries = document.get(key, [])
    if not isinstance(entries, list):
        raise LicenseAuditError(f"license exceptions: {key} must be a list")
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict):
            raise LicenseAuditError(f"license exceptions: {key}[{index}] must be an object")
        for field in fields:
            value = entry.get(field)
            if not isinstance(value, str) or not value.strip():
                raise LicenseAuditError(
                    f"license exceptions: {key}[{index}] needs a non-empty {field!r}"
                )
        for field in date_fields:
            if not _ISO_DATE.match(entry[field]):
                raise LicenseAuditError(
                    f"license exceptions: {key}[{index}] {field} is not YYYY-MM-DD"
                )
        if entry["ecosystem"] not in {"npm", "cargo"}:
            raise LicenseAuditError(f"license exceptions: {key}[{index}] has unknown ecosystem")
    return entries


def validate_exceptions(document: Any) -> dict[str, Any]:
    if not isinstance(document, dict) or document.get("contract_id") != "license-exceptions.v1":
        raise LicenseAuditError("license exceptions file is not contract license-exceptions.v1")
    version = document.get("version", 1)
    if not isinstance(version, int) or isinstance(version, bool):
        raise LicenseAuditError("license exceptions: version must be an integer")
    exceptions = _require_entries(document, "exceptions", _EXCEPTION_FIELDS, ("granted_on",))
    for index, entry in enumerate(exceptions):
        pinned = entry.get("version")
        if pinned is not None and (not isinstance(pinned, str) or not pinned.strip()):
            raise LicenseAuditError(
                f"license exceptions: exceptions[{index}] version must be a non-empty string"
            )
    keys = [(entry["ecosystem"], entry["name"]) for entry in exceptions]
    if len(keys) != len(set(keys)):
        raise LicenseAuditError("license exceptions: duplicate ecosystem/name exception")
    reviews = _require_entries(document, "license_file_reviews", _REVIEW_FIELDS, ("reviewed_on",))
    allowlist = document.get("license_ref_allowlist", [])
    if not isinstance(allowlist, list) or not all(isinstance(item, str) for item in allowlist):
        raise LicenseAuditError(
            "license exceptions: license_ref_allowlist must be a list of strings"
        )
    return {
        "exceptions": exceptions,
        "license_file_reviews": reviews,
        "license_ref_allowlist": allowlist,
    }


def load_exceptions(path: Path) -> dict[str, Any]:
    return validate_exceptions(json.loads(path.read_text(encoding="utf-8")))


def _matching_review(
    record: DependencyLicense, reviews: list[dict[str, Any]]
) -> dict[str, Any] | None:
    for review in reviews:
        if (
            review["ecosystem"] == record.ecosystem
            and review["name"] == record.name
            and review["version"] == record.version
            and review["license_file"] == record.license_file
            and review["license_file_sha256"] == record.license_file_sha256
        ):
            return review
    return None


def evaluate_policy(
    records: list[DependencyLicense],
    exceptions_document: dict[str, Any],
    *,
    mode: str,
    target: str = DEFAULT_WINDOWS_TARGET,
) -> dict[str, Any]:
    """Classify every record against the GPLv3 compatibility policy and apply exceptions per mode."""
    if mode not in POLICY_MODES:
        raise LicenseAuditError(f"unknown policy mode: {mode!r}")
    honor_exceptions = mode == "l3"
    exceptions = exceptions_document["exceptions"]
    reviews = exceptions_document["license_file_reviews"]
    allowlist = exceptions_document["license_ref_allowlist"]
    used_exceptions: set[tuple[str, str]] = set()

    classified: list[dict[str, Any]] = []
    ordered = sorted(records, key=lambda item: (item.ecosystem, item.name.casefold(), item.version))
    for record in ordered:
        entry = asdict(record)
        basis = "license"
        if record.license:
            verdict = policy.evaluate_expression(record.license, license_ref_allowlist=allowlist)
        else:
            review = _matching_review(record, reviews)
            if review is not None:
                basis = "license_file_review"
                verdict = policy.evaluate_expression(
                    review["classified_as"], license_ref_allowlist=allowlist
                )
                entry["classified_as"] = review["classified_as"]
            else:
                verdict = policy.NEEDS_REVIEW
        entry["verdict"] = verdict
        entry["basis"] = basis
        if verdict != policy.COMPATIBLE:
            for exception in exceptions:
                if (
                    exception["ecosystem"] == record.ecosystem
                    and exception["name"] == record.name
                    and exception["license"] == record.license
                    and exception.get("version") in (None, record.version)
                ):
                    entry["exception"] = exception
                    used_exceptions.add((exception["ecosystem"], exception["name"]))
                    break
        entry["outcome"] = "excepted" if "exception" in entry and honor_exceptions else verdict
        classified.append(entry)

    def _bucket(outcome: str) -> list[dict[str, Any]]:
        return [entry for entry in classified if entry["outcome"] == outcome]

    excepted = _bucket("excepted")
    incompatible = _bucket(policy.INCOMPATIBLE)
    needs_review = _bucket(policy.NEEDS_REVIEW)
    ok = not incompatible and not needs_review
    counts = Counter(record.ecosystem for record in records)
    return {
        "ok": ok,
        "status": "pass" if ok else "fail",
        "mode": mode,
        "exceptions_honored": honor_exceptions,
        "scope": POLICY_SCOPE,
        "project_license": "GPL-3.0-or-later",
        "target": target,
        "cargo_scope": (
            "full resolve graph for the target: normal, build and dev dependencies, "
            "including non-workspace path crates; only workspace members are skipped"
        ),
        "npm_scope": (
            "package-lock production dependencies; dev: true is excluded because Vite "
            "bundles only modules the app imports. Known case: tailwindcss is a dev "
            "dependency whose generated CSS ships (MIT today) but is not scanned"
        ),
        "scope_asymmetry": (
            "Rust is scanned over every dependency kind (stricter, cheap); npm only over "
            "production packages, since dev tooling is not part of the distributed work"
        ),
        "worker_coverage": {
            "covered": False,
            "reason": (
                "Not covered: the PyInstaller Compatibility Worker (audit its actual "
                "PyInstaller artifact closure) and the native GIS runtime (fail-closed "
                "manifest in the GIS runtime staging)."
            ),
        },
        "dependency_count": len(records),
        "ecosystem_counts": dict(sorted(counts.items())),
        "counts": {
            "compatible": len(_bucket(policy.COMPATIBLE)),
            "excepted": len(excepted),
            "incompatible": len(incompatible),
            "needs_review": len(needs_review),
        },
        "excepted": excepted,
        "incompatible": incompatible,
        "needs_review": needs_review,
        "license_file_reviews_applied": [
            entry for entry in classified if entry["basis"] == "license_file_review"
        ],
        "unused_exceptions": [
            exception
            for exception in exceptions
            if (exception["ecosystem"], exception["name"]) not in used_exceptions
        ],
        "approval_claimed": False,
        "dependencies": classified,
    }


def run_policy_scan(
    repository_root: Path = ROOT,
    *,
    mode: str,
    npm_lock: Path | None = None,
    exceptions_path: Path | None = None,
    target: str = DEFAULT_WINDOWS_TARGET,
) -> dict[str, Any]:
    lock_path = npm_lock or repository_root / "ui" / "package-lock.json"
    npm_records = audit_npm_lock(json.loads(lock_path.read_text(encoding="utf-8")))
    cargo_records = cargo_resolve_records(run_cargo_metadata(repository_root, target=target))
    exceptions_document = load_exceptions(exceptions_path or DEFAULT_EXCEPTIONS_PATH)
    return evaluate_policy(
        [*npm_records, *cargo_records], exceptions_document, mode=mode, target=target
    )


def _summary_line(entry: dict[str, Any]) -> str:
    license_text = entry["license"] or f"<no license field; license_file={entry['license_file']}>"
    line = f"  {entry['ecosystem']} {entry['name']} {entry['version']}: {license_text}"
    exception = entry.get("exception")
    if exception:
        honored = "" if entry.get("outcome") == "excepted" else "NOT honored in this mode: "
        line += (
            f" [{honored}exception {exception['issue']}, granted by {exception['granted_by']} "
            f"{exception['granted_on']}, must resolve before {exception['must_resolve_before']}]"
        )
    return line


def format_policy_summary(report: dict[str, Any]) -> str:
    counts = report["counts"]
    ecosystems = ", ".join(f"{name} {count}" for name, count in report["ecosystem_counts"].items())
    lines = [
        f"license policy {report['project_license']} mode={report['mode']} "
        f"status={report['status']}",
        f"dependencies: {report['dependency_count']} ({ecosystems})",
        "compatible={compatible} excepted={excepted} incompatible={incompatible} "
        "needs_review={needs_review}".format(**counts),
    ]
    sections = (
        ("excepted (warning; honored in l3 mode only)", report["excepted"]),
        ("incompatible", report["incompatible"]),
        ("needs_review", report["needs_review"]),
    )
    for title, entries in sections:
        lines.append(f"{title}: {len(entries)}")
        lines.extend(_summary_line(entry) for entry in entries)
    reviews = report["license_file_reviews_applied"]
    lines.append(f"license_file reviews applied: {len(reviews)}")
    lines.extend(
        f"  {entry['ecosystem']} {entry['name']} {entry['version']}: "
        f"{entry['license_file']} -> {entry['classified_as']}"
        for entry in reviews
    )
    unused = report["unused_exceptions"]
    if unused:
        lines.append(f"unused exceptions (warning): {len(unused)}")
        lines.extend(f"  {item['ecosystem']} {item['name']}: {item['license']}" for item in unused)
    return "\n".join(lines)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--npm-lock",
        type=Path,
        default=ROOT / "ui" / "package-lock.json",
        help="npm lock file to audit (default: ui/package-lock.json)",
    )
    parser.add_argument(
        "--target",
        default=DEFAULT_WINDOWS_TARGET,
        help=f"Cargo target used to resolve host dependencies (default: {DEFAULT_WINDOWS_TARGET})",
    )
    parser.add_argument(
        "--fail-on-review-required",
        action="store_true",
        help="return exit code 2 when a configured review-required license is present",
    )
    parser.add_argument(
        "--mode",
        choices=POLICY_MODES,
        help=(
            "run the GPLv3 compatibility policy: 'l3' honors the dated exceptions "
            "(warnings), 'distribution' ignores them; exit code 3 on any violation"
        ),
    )
    parser.add_argument(
        "--exceptions",
        type=Path,
        default=DEFAULT_EXCEPTIONS_PATH,
        help="exception contract for --mode (default: contracts/license-exceptions.v1.json)",
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="with --mode: write the full JSON report to this path",
    )
    return parser.parse_args(argv)


def run_policy_main(args: argparse.Namespace) -> int:
    try:
        report = run_policy_scan(
            ROOT,
            mode=args.mode,
            npm_lock=args.npm_lock,
            exceptions_path=args.exceptions,
            target=args.target,
        )
    except (OSError, json.JSONDecodeError, LicenseAuditError) as exc:
        print(f"license policy {args.mode}: audit_error: {exc}")
        return 1
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(
            json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    print(format_policy_summary(report))
    if args.output is not None:
        print(f"report: {args.output}")
    return 0 if report["ok"] else POLICY_EXIT_VIOLATION


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.mode is not None:
        return run_policy_main(args)
    try:
        lock_document = json.loads(args.npm_lock.read_text(encoding="utf-8"))
        npm_records = audit_npm_lock(lock_document)
        cargo_document = run_cargo_metadata(ROOT, target=args.target)
        cargo_records = audit_cargo_metadata(
            cargo_document, extra_root_package_names=("insar-agent-model", "insar-agent-mcp")
        )
        report = build_report(npm_records, cargo_records, target=args.target)
    except (OSError, json.JSONDecodeError, LicenseAuditError) as exc:
        print(
            json.dumps(
                {
                    "ok": False,
                    "status": "audit_error",
                    "scope": "tauri-host-and-agent-license-metadata-v2",
                    "error": str(exc),
                    "approval_claimed": False,
                },
                ensure_ascii=False,
                sort_keys=True,
            )
        )
        return 1

    print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    return report_exit_code(report, fail_on_review_required=args.fail_on_review_required)


if __name__ == "__main__":
    sys.exit(main())
