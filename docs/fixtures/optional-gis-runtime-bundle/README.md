# Optional GIS Runtime Bundle offline fixtures

These files are deterministic structural vectors for the optional GIS runtime
bundle verifier and activation transaction contracts.

They contain no GDAL binary, Python runtime, credential, private signing key,
Provider response or personal path. The `archives/` directory contains five
deterministic, stored ZIPs: exactly one archive per component. They contain only
synthetic, non-executable bytes and must never be installed by a production
component manager. Each archive has a companion ComponentManifest fixture, so
both valid BundleSets can be checked end-to-end without inventing member
digests. A BundleSet is an activation combination, never a combined archive.

Files containing `fixture` and `mutation` are deterministic policy-level
mutation vectors, not standalone signed manifests. A future verifier must keep
signature-invalid cases separate from expiry, replay, platform, checksum,
grid, and self-test policy assertions; it must not treat a mutated unsigned
object as a cryptographic success.

`active-before.json` models the retained prior pointer during an installation
transaction; its base-only selection is intentional. `active-after.json` is the
complete base-plus-extension registry whose digests resolve to
`bundle-set-valid.json` and `bundle-set-extension-valid.json`.

The Ed25519 public key in `trust-root-fixture.json` and the signatures in the
valid manifest vectors are real cryptographic test values generated from a
deterministic fixture seed. The private key is not included and the key is not a
release trust root. Future verifier tests must not contact OSS, COS, CDN or
GitHub. The `.part` vector represents an interrupted download with the matching
`part-state.json` resume metadata. Replay-ledger, activation-journal and lease
vectors model crash recovery outside SQLite.

Archive entries are sorted by unsigned UTF-8 path bytes, case-sensitive and
without locale/case folding. ZIP bytes use a fixed DOS timestamp, stored
compression, no comments and no extra fields. The fixture extractor limits are
100000 entries, 4294967296 bytes per file, 17179869184 total bytes and a 1000
compression ratio; only strict greater-than exceeds a limit. The journal phase
vectors include every recovery phase plus separate pointer-pending match and
mismatch observations.

The post-commit backup scenario points to the candidate ledger, not the prior
transaction snapshot; restoring it therefore preserves the last accepted
generation and release sequences. The prior ledger is used only to roll back an
incomplete activation.

The `startup-no-journal-*.json` envelopes exercise the complete startup matrix:
equal ledgers, either corrupt or missing copy, a structurally valid stale copy,
same-generation divergence, invalid/missing active state, both-ledger failure,
empty first install and a committed authorized rollback. They are state-machine
vectors, not production persistence. A valid-but-different ledger is never
selected merely because its generation or timestamp looks newer.

`bundle-set-base-previous-valid.json` and
`rollback-authorization-valid.json` use the existing synthetic fixture key.
The former is a signed older base-set combination; the latter is a signed,
single-use, exact-target rollback authorization. No new key or private key is
stored. Rollback scenario envelopes cover ordinary rejection, expiry, target
mismatch, authorization reuse, every crash phase and an active lease. The
candidate rollback ledger increases generation while preserving every accepted
stream floor; the committed receipt, not `active.json`, explains why the lower
target may be active. Exact RollbackRequest, receipt, journal and RecoveryRecord
fixtures keep user intent, one-time consumption, crash state and audit evidence
separate.

Startup and rollback scenario envelopes may reference injected validation
outcomes and exact fixture digests. They do not claim that a production parser,
ledger manager, authorization verifier, journal recovery path or Worker exists.
