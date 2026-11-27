## Summary

<!-- Briefly describe what this PR changes and why. -->

## Type of Change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor / cleanup
- [ ] Documentation

## Verification

- [ ] `cd ui && npm run check && npm test && npm run build:tauri`
- [ ] `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace -- --test-threads=1`
- [ ] `python3 scripts/check_public_hygiene.py`
- [ ] `python3 scripts/audit_tauri_host_licenses.py --mode distribution`

## Checklist

- [ ] No credentials, personal paths, or unlicensed third-party code/assets were added.
- [ ] Any third-party code adaptation is documented in `THIRD-PARTY-NOTICES.md`.
- [ ] Commits are signed off per DCO 1.1 (`git commit -s`).
