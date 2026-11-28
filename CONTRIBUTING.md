# Contributing to EOStage

Thank you for your interest in contributing to **EOStage**.

## Development Setup

EOStage is a Rust + Tauri 2 desktop application with a React + TypeScript frontend (`ui/`).

1. Read [docs/README.md](docs/README.md) for the documentation index and local development guides.
2. Read [docs/code-standards.md](docs/code-standards.md) and [docs/architecture/repository-map.md](docs/architecture/repository-map.md) before making structural changes.

## Before Opening a Pull Request

Please verify your changes locally:

```bash
# Frontend checks
cd ui && npm run check && npm test && npm run build:tauri && cd ..

# Rust workspace checks (optional runtime/gis tests report cleanly as ignored)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -- --test-threads=1

# Optional strict native GIS regression suite (when runtime/gis is installed)
INSAR_GIS_RUNTIME_ROOT=/path/to/runtime/gis bash scripts/run_native_gis_tests.sh

# Public hygiene, license & CDN/CSP consistency checks
python3 scripts/check_public_hygiene.py
python3 scripts/audit_tauri_host_licenses.py --mode distribution
node scripts/check_atlas_cdn_csp.mjs
```

## Optional Stage-2 Onboarding HD Atlas Pack & Desktop CSP

EOStage ships with bundled onboarding globe textures and Stage-1 preview sprites (`ui/public/onboarding-globe/`, `ui/public/onboarding-stage1/`). The optional high-resolution Stage-2 photo archive (`v2/` referenced by `ui/src/components/desktop/onboarding/atlasCatalog.json`) is **not** distributed in the public repository; without it, the welcome dialog displays an explicit built-in fallback card with zero 404 requests.

If you self-host or test an external Stage-2 atlas pack matching `atlasCatalog.json`:

1. **Local dev preview only**: place your `v2/...` files under `atlas-cdn-pack/v2/` at the repository root (git-ignored and never bundled into `dist-tauri`). Vite enables `/atlas-cdn/` in `dev` / `dev:tauri` only when `atlas-cdn-pack/v2/` exists.
2. **Hosted HTTPS CDN (`VITE_EO_ATLAS_CDN_BASE_URL`)**: set `VITE_EO_ATLAS_CDN_BASE_URL=https://your-cdn.example/path` **and** explicitly allowlist that `https://` origin in:
   - `src-tauri/tauri.conf.json` (`app.security.csp.img-src`)
   - `src-tauri/tauri.dev.conf.json` (`app.security.devCsp.img-src`)
   - `src-tauri/tests/security_policy.rs` (`assert_reviewed_image_policy`)
   - `ui/src/shell/api/w53Thumbnails.test.ts` (`CSP_ALLOWED_IMAGE_HOSTS` alignment check)
   Vite builds and `scripts/check_atlas_cdn_csp.mjs` fail closed if `VITE_EO_ATLAS_CDN_BASE_URL` points to an origin missing from the Tauri CSP allowlist.
3. **Self-hosted licensing**: maintainers hosting an external pack are responsible for supplying appropriate imagery rights and attributions for their hosted assets.

## Contributing Rules

- **Minimal, Focused Changes:** Keep each PR scoped to one clear bug fix or feature with matching unit/interaction tests.
- **Scientific & Data Integrity:** Never fabricate metadata, bypass credential redaction, or silently substitute distinct scientific products.
- **Third-Party Code & Licensing:** All contributions must be compatible with **GPL-3.0-or-later**. Do not copy unlicensed third-party code or assets into the repository; any properly licensed external adaptation must be recorded in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
- **Developer Certificate of Origin (DCO 1.1):** External contributions are accepted under DCO 1.1. Please sign off commits with `git commit -s` (`Signed-off-by: Name <email>`).

## Contributing Skills & Plugins

EOStage enforces strict five-layer boundaries (see [PRODUCT.md](PRODUCT.md#extension-skill-plugin-mcp--runtime-component-principles)):

- **Skills (`SKILL.md`):** Canonical instructions MUST be structured English specifying trigger conditions, inputs/units, decision rules, failure/stop conditions, deliverable, and verification protocol, accompanied by localized card metadata (`zh-CN`, `en`, `ja`) and worked + adverse test examples. Do not submit short persona prompts or pure textbook excerpts as standalone skills.
- **Plugins:** Must bundle at least one genuinely executable capability verified end-to-end through EOStage (real MCP server, bounded adapter, or reviewed script runner) plus a minimal fixture and complete package integrity coverage; methodology-only submissions belong under **Skills**, never **Plugins**.
- **MCP Connectors & Runtime Components:** Only declare real MCP stdio/Streamable HTTP endpoints (not `tauri://` internal wrappers) and declare shared heavy runtime dependencies (GDAL/PROJ, vertical grids) by component ID without duplicating binaries.
