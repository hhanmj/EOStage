# M1 B2 Store snapshot codec fixtures

These offline fixtures drive the pure Store codec, its V2-only compatibility
boundary, and the focused row-envelope checker. They contain no database
credentials, network input, runtime task state, or production V3 writer data.

Each JSON fixture selects a deterministic in-test base:

- `legacy_v2` and `compact_v2` are V2-only snapshots;
- `v3_single` and `v3_shared` reuse the B1B accepted graph payloads;
- `snapshot_json` provides a raw malformed payload when JSON parsing itself is
  the case under test.

Optional mutations are applied before the codec runs. Expected results assert
the version-aware codec outcome, the V2-only compatibility outcome, safe V3
validation subtype where applicable, and row-envelope mismatch issue. The
corpus has 21 JSON vectors plus one raw malformed JSON fragment.

The row-envelope vectors use a rollback-only SQLite test. They verify only
`task_snapshots` metadata versus decoded snapshot content and never create a
V3 runtime, view, event, or production task row.