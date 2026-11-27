#!/usr/bin/env python3
"""Comprehensive public-repository hygiene gate for EOStage."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SELF_REL = "scripts/check_public_hygiene.py"

FORBIDDEN_PATTERNS: list[tuple[str, re.Pattern[str], bool]] = [
    # (label, pattern, prod_only_for_secrets)
    (
        "competitor/reference brand",
        re.compile(
            r"(?:(?<![A-Za-z0-9])(?:GeoD(?:\s+Agent)?|geod-global|GeoDownloader|ClawsGO|ZCode|Cherry\s*Studio|interfaces\.show|Antigravity|SARGet|ZGHHGZ|Kubai|Astryx|Devin)(?![A-Za-z0-9])|(?:ZCODE|CLAWSGO|DEVIN)_)",
            re.I,
        ),
        False,
    ),
    ("legacy competitor CSS/DOM prefix", re.compile(r"\b(?:dv|devin)-[a-z0-9_-]+"), False),
    ("provenance-scrubbing or reverse-engineering term", re.compile(r"(?:原教程|公众号|反编译|逆向参考)"), False),
    (
        "developer macOS home path",
        re.compile(r"(?<![A-Za-z0-9._/-])/Users/(?!demo\b|alice\b|bob\b|fake\b|me\b|owner\b|someone\b|user\b|you\b|x\b|Zhang\b|zhang\b|<user>)[A-Za-z0-9._-]+|/Volumes/[A-Za-z0-9._-]+"),
        False,
    ),
    (
        "developer Linux home path",
        re.compile(r"(?<![A-Za-z0-9._/-])/home/(?!demo\b|alice\b|bob\b|fake\b|me\b|owner\b|someone\b|user\b|you\b|x\b|Zhang\b|zhang\b|runner\b|builder\b|\.|<user>)[A-Za-z0-9._-]+"),
        False,
    ),
    (
        "developer Windows path",
        re.compile(
            r"(?i)(?:C:\\+Users\\+(?!demo\b|you\b|user\b|alice\b|bob\b|fake\b|me\b|owner\b|someone\b|x\b|zhang\b|private\b|example\b|fixture(?:-secret)?\b|<user>)[A-Za-z0-9._-]+|[EF]:\\+(?:InSAR_Studio|ScenePort|A_Master))"
        ),
        False,
    ),
    ("brand-swap misattribution", re.compile(r"(?:对齐\s*EOStage|Exact\s+EOStage|EOStage-EOStage|原\s*EOStage|eostage/(?:tools|core)/[A-Za-z0-9_]+\.py)"), False),
    ("private VPS/SSH leak", re.compile(r"\beostage_vps\b|\beostage-server\b|BatchMode=yes\s+root@|ssh\s+-i\s+[^\s]+", re.I), False),
    ("personal email leak", re.compile(r"[A-Za-z0-9._%+-]+@(?:gmail|foxmail|qq)\.com\b", re.I), False),
    ("private key material", re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"), True),
]

MD_LINK_RE = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
DELETED_INTERNAL_PATH_RE = re.compile(
    r"(?:docs/superpowers/|docs/tasks/|docs/design/|docs/reports/|docs/worklog/|docs/agent-handoff\.md|memories/[A-Za-z0-9_.-]+|\.agents/(?:rules|skills)/|scripts/tests/|insar-studio-ui-audit|\bAGENTS\.md\b)"
)
FAKE_SKILL_HOST_RE = re.compile(
    r"(?:EOStage\s+cloud|cloud\s+host|compute\s+host|eostage-ssh|eostage-figure|/opt/refdata|conda-run|weather-model-data-fetching|sceneport-core)",
    re.I,
)
LEGACY_BRAND_PUBLIC_RE = re.compile(r"(?:ScenePort|InSAR[\s_-]Studio)", re.I)


def tracked_files() -> list[str]:
    return subprocess.check_output(["git", "-C", str(ROOT), "ls-files"], text=True).splitlines()


def is_text_bytes(raw: bytes) -> bool:
    if b"\x00" in raw[:8192]:
        return False
    return True


def check_markdown_links(rel: str, text: str) -> list[str]:
    errs: list[str] = []
    base = (ROOT / rel).parent
    in_fence = False
    for line_no, line in enumerate(text.splitlines(), start=1):
        stripped = line.strip()
        if stripped.startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        for match in MD_LINK_RE.finditer(line):
            target = match.group(1).strip()
            if (
                not target
                or target.startswith("#")
                or target.startswith("http://")
                or target.startswith("https://")
                or target.startswith("mailto:")
                or target.startswith("file://")
            ):
                continue
            target_path = target.split("#", 1)[0].split("?", 1)[0]
            if not target_path:
                continue
            resolved = (base / target_path).resolve()
            if not resolved.exists():
                errs.append(f"{rel}:{line_no}: broken relative markdown link {target!r}")
        if DELETED_INTERNAL_PATH_RE.search(line):
            errs.append(f"{rel}:{line_no}: reference to deleted internal path: {line.strip()[:120]}")
    return errs


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check-history",
        action="store_true",
        help="Also verify git history contains zero private IPs or emails (for final orphan public branch).",
    )
    args = parser.parse_args()

    errors: list[str] = []
    files = tracked_files()

    # 1. Scan ALL tracked text files (no extension blind spots, no sync script exemption)
    for rel in files:
        if rel == SELF_REL:
            continue
        p = ROOT / rel
        if not p.is_file() or p.stat().st_size > 2_500_000:
            continue
        raw = p.read_bytes()
        if not is_text_bytes(raw):
            continue
        text = raw.decode("utf-8", errors="ignore")
        is_test_file = ".test." in rel or "/tests/" in rel or rel.startswith("tests/")
        prod_text = text.split("#[cfg(test)]", 1)[0] if rel.endswith(".rs") else text
        for label, pat, prod_only in FORBIDDEN_PATTERNS:
            if prod_only and is_test_file:
                continue
            scan_text = prod_text if prod_only else text
            for line_no, line in enumerate(scan_text.splitlines(), start=1):
                if pat.search(line):
                    errors.append(f"{rel}:{line_no}: {label}: {line.strip()[:120]}")
                    break
        for line_no, line in enumerate(text.splitlines(), start=1):
            if DELETED_INTERNAL_PATH_RE.search(line):
                errors.append(f"{rel}:{line_no}: reference to deleted internal path: {line.strip()[:120]}")
                break
        if rel.startswith("ui/public/skills/"):
            for line_no, line in enumerate(text.splitlines(), start=1):
                if FAKE_SKILL_HOST_RE.search(line):
                    errors.append(f"{rel}:{line_no}: nonexistent cloud-host/skill ref in skill: {line.strip()[:120]}")
                    break
        check_legacy_brand = (
            rel in {"README.md", "PRODUCT.md", "DESIGN.md", "GLOSSARY.md", "THIRD-PARTY-NOTICES.md"}
            or (rel.startswith("docs/") and not rel.startswith("docs/fixtures/optional-gis-runtime-bundle/"))
            or rel.startswith(("packaging/", "contracts/", "ui/public/skills/"))
            or (rel.startswith("scripts/") and rel != "scripts/sync_builtin_skills_catalog.py")
        )
        if check_legacy_brand:
            for line_no, line in enumerate(text.splitlines(), start=1):
                if LEGACY_BRAND_PUBLIC_RE.search(line):
                    errors.append(f"{rel}:{line_no}: legacy brand in public/contract/script file: {line.strip()[:120]}")
                    break
        if rel.endswith(".md") and not rel.startswith("ui/public/skills/"):
            errors.extend(check_markdown_links(rel, text))

    # 2. Verify built-in skills license, copyright vs manifest consistency, and THIRD-PARTY-NOTICES coverage
    notices_text = (ROOT / "THIRD-PARTY-NOTICES.md").read_text(encoding="utf-8")
    manifest_path = ROOT / "ui/public/skills/manifest.json"
    if manifest_path.is_file():
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        for skill in manifest.get("skills", []):
            sid = str(skill.get("id", ""))
            author = str(skill.get("author", "")).strip()
            lic = str(skill.get("license", "")).strip()
            if not lic or "未声明" in lic or "no license" in lic.lower():
                errors.append(f"ui/public/skills/manifest.json: skill {sid!r} lacks a valid license ({lic!r})")
            if author != "EOStage":
                if sid not in notices_text:
                    errors.append(f"THIRD-PARTY-NOTICES.md: missing third-party built-in skill entry {sid!r} ({author})")
                skill_dir = ROOT / "ui/public/skills" / sid
                lic_files = list(skill_dir.glob("LICENSE*"))
                if not lic_files:
                    errors.append(f"ui/public/skills/{sid}: third-party skill missing LICENSE file")
                else:
                    combined_lic = "\n".join(lf.read_text(encoding="utf-8", errors="ignore") for lf in lic_files)
                    if "EOStage Contributors" in combined_lic:
                        errors.append(f"ui/public/skills/{sid}: third-party LICENSE wrongly names EOStage Contributors")

    # 2b. Verify every bundledFileContents entry in geoscienceSkillsCatalog.ts exists on disk (fail-closed)
    catalog_ts_path = ROOT / "ui/src/lib/geoscienceSkillsCatalog.ts"
    if not catalog_ts_path.is_file():
        errors.append("ui/src/lib/geoscienceSkillsCatalog.ts: missing generated skills catalog")
    else:
        ts_raw = catalog_ts_path.read_text(encoding="utf-8")
        ts_match = re.search(
            r"export\s+const\s+GEOSCIENCE_SKILL_PACKAGES\s*:\s*readonly\s+GeoscienceSkillPackage\[\]\s*=\s*(\[\s*\{[\s\S]*\n\]);",
            ts_raw,
        )
        if not ts_match:
            errors.append(
                "ui/src/lib/geoscienceSkillsCatalog.ts: failed to parse GEOSCIENCE_SKILL_PACKAGES array"
            )
        else:
            ts_packages = json.loads(ts_match.group(1))
            if not ts_packages:
                errors.append("ui/src/lib/geoscienceSkillsCatalog.ts: GEOSCIENCE_SKILL_PACKAGES is empty")
            for pkg in ts_packages:
                sid = str(pkg.get("id", ""))
                for rel_file in pkg.get("bundledFileContents", {}).keys():
                    if not (ROOT / "ui/public/skills" / sid / rel_file).is_file():
                        errors.append(
                            f"ui/src/lib/geoscienceSkillsCatalog.ts: skill {sid!r} bundles deleted file {rel_file!r}"
                        )
                if sid == "radar-satellite-analysis" and "天气雷达" in json.dumps(pkg, ensure_ascii=False):
                    errors.append(
                        "ui/src/lib/geoscienceSkillsCatalog.ts: radar-satellite-analysis still contains weather-radar metadata"
                    )

    # 2c. Verify eoAgentSuite.ts comments referencing Python scripts attribute GeoAgent or cite an existing file
    eo_suite_path = ROOT / "ui/src/lib/eoAgentSuite.ts"
    if eo_suite_path.is_file():
        all_py_basenames = {Path(f).name for f in files if f.endswith(".py")}
        for line_no, line in enumerate(eo_suite_path.read_text(encoding="utf-8").splitlines(), start=1):
            if "/*" in line or line.lstrip().startswith("*"):
                for py_name in re.findall(r"\b([A-Za-z0-9_]+\.py)\b", line):
                    if py_name not in all_py_basenames and "GeoAgent" not in line:
                        errors.append(
                            f"ui/src/lib/eoAgentSuite.ts:{line_no}: external Python ref {py_name!r} without opengeos/GeoAgent attribution"
                        )

    # 3. Verify Taiwan admin boundary policy SHA-256 integrity
    tw_path = ROOT / "contracts/taiwan-admin/taiwan-admin-identities.v1.json"
    if tw_path.is_file():
        tw = json.loads(tw_path.read_text(encoding="utf-8"))
        ev_rel = tw.get("verification", {}).get("evidence_path", "")
        ev_sha = tw.get("verification", {}).get("evidence_sha256", "")
        ev_file = ROOT / ev_rel
        if not ev_file.is_file():
            errors.append(f"contracts/taiwan-admin: missing evidence_path {ev_rel!r}")
        else:
            actual_sha = hashlib.sha256(ev_file.read_bytes()).hexdigest()
            if actual_sha != ev_sha:
                errors.append(f"contracts/taiwan-admin: sha256 mismatch ({ev_sha} != {actual_sha})")

    # 4. Verify license consistency across Cargo.toml, ui/package.json, CITATION.cff, LICENSE
    cargo_toml = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    if 'license = "GPL-3.0-or-later"' not in cargo_toml:
        errors.append("Cargo.toml: workspace license must be GPL-3.0-or-later")
    ui_pkg = json.loads((ROOT / "ui/package.json").read_text(encoding="utf-8"))
    if ui_pkg.get("license") != "GPL-3.0-or-later":
        errors.append("ui/package.json: license must be GPL-3.0-or-later")
    citation = (ROOT / "CITATION.cff").read_text(encoding="utf-8")
    if "license: GPL-3.0-or-later" not in citation:
        errors.append("CITATION.cff: license must be GPL-3.0-or-later")

    # 5. Verify all runtime plugins & nested SKILL.md have consistent EOStage attribution & GPL-3.0-or-later license
    for pj in sorted((ROOT / "runtime/plugins").glob("eostage-*/plugin.json")):
        data = json.loads(pj.read_text(encoding="utf-8"))
        if data.get("author") != "EOStage":
            errors.append(f"{pj.relative_to(ROOT)}: unexpected plugin author {data.get('author')!r}")
        if data.get("license") != "GPL-3.0-or-later":
            errors.append(f"{pj.relative_to(ROOT)}: plugin license must be GPL-3.0-or-later")
    for sm in sorted((ROOT / "runtime/plugins").rglob("SKILL.md")):
        sm_text = sm.read_text(encoding="utf-8")
        if "OSGeo / Pangeo" in sm_text:
            errors.append(f"{sm.relative_to(ROOT)}: invalid third-party author attribution")
        if not re.search(r"^author:[ \t]+EOStage\s*$", sm_text, flags=re.M):
            errors.append(f"{sm.relative_to(ROOT)}: nested plugin SKILL.md must declare author: EOStage")
        if not re.search(r"^license:[ \t]+GPL-3\.0-or-later\s*$", sm_text, flags=re.M):
            errors.append(f"{sm.relative_to(ROOT)}: nested plugin SKILL.md must declare license: GPL-3.0-or-later")

    # 6. Verify local CLI credential scanner is absent from src-tauri
    oauth_text = (ROOT / "src-tauri/src/oauth_login.rs").read_text(encoding="utf-8")
    for forbidden_sym in ("detect_local_cli_credentials", "import_local_cli_credential", "gh/hosts.yml", "application_default_credentials.json"):
        if forbidden_sym in oauth_text:
            errors.append(f"src-tauri/src/oauth_login.rs: forbidden local CLI scanner symbol {forbidden_sym!r}")

    # 7. Optional full git history scan (used on final orphan public branch)
    if args.check_history:
        commit_count = int(
            subprocess.check_output(["git", "-C", str(ROOT), "rev-list", "--count", "--all"], text=True).strip()
        )
        if commit_count != 1:
            errors.append(
                f"git-history: expected single-commit orphan history for public export, found {commit_count} commits"
            )
        log_out = subprocess.check_output(
            ["git", "-C", str(ROOT), "log", "--all", "--format=%H%x09%ae%x09%ce%x09%s%x09%b"], text=True
        )
        for line in log_out.splitlines():
            if not line.strip():
                continue
            parts = line.split("\t", 4)
            if len(parts) >= 3:
                ae, ce = parts[1].strip().lower(), parts[2].strip().lower()
                if not ae.endswith("@users.noreply.github.com") or not ce.endswith("@users.noreply.github.com"):
                    errors.append(
                        f"git-history: commit author/committer email must use @users.noreply.github.com: {ae!r} / {ce!r}"
                    )
            msg = "\t".join(parts[3:]) if len(parts) > 3 else line
            if re.search(
                r"\b(?:ZCode|ClawsGO|Devin|GeoD|eostage_vps|eostage-server)\b|@(?:gmail|foxmail|qq)\.com",
                msg,
                re.I,
            ):
                errors.append(f"git-history: sensitive metadata in commit message: {line[:120]}")
        blob_hits = subprocess.check_output(
            [
                "git",
                "-C",
                str(ROOT),
                "log",
                "--all",
                "--oneline",
                "-E",
                "-G",
                r"(eostage_vps|eostage-server|@(gmail|foxmail|qq)\.com|BatchMode=yes\s+root@)",
                "--",
                ".",
                f":(exclude){SELF_REL}",
            ],
            text=True,
        ).strip()
        if blob_hits:
            errors.append(f"git-history: historical file contents still contain sensitive strings:\n{blob_hits}")
        self_text = (ROOT / SELF_REL).read_text(encoding="utf-8")
        if re.search(r"\b\d{1,3}(?:\.\d{1,3}){3}\b", self_text) or re.search(
            r"[A-Za-z0-9._%+-]+@(?:gmail|foxmail|qq)\.com", self_text, re.I
        ):
            errors.append(f"{SELF_REL}: script itself must not embed literal IPv4 or personal email addresses")

    if errors:
        for err in errors:
            print(f"ERROR: {err}", file=sys.stderr)
        return 1
    print(f"Public repository hygiene check passed ({len(files)} tracked files scanned).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
