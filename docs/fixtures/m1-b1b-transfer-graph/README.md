# M1 B1B frozen task graph fixtures

These offline fixtures drive only
`validate_frozen_task_snapshot_v3_graph` against generated-structure-valid
`FrozenTaskSnapshotV3` values. They contain no credentials, live URLs, Store
rows, or runtime task state.

The two accepted fixtures carry complete V3 payloads. Each rejected fixture is
an independent vector that declares a valid base payload (`single` or
`shared`), deterministic JSON mutations, and the complete expected typed
graph error. The fixture harness applies the mutations before deserializing,
runs generated structural validation, and then compares the entire error value.

Rejected `expected.error` values record only safe fields: error variant,
identity/reference/issue/subject, collection location/index, and count. They
never record source or transfer ID values, paths, URLs, credentials, or
snapshot JSON.

The requested-orbit-type vector is the one documented exception to the graph-only
precondition: generated structural validation already requires \`POEORB\`. Its
fixture records that structural result while separately asserting the graph-only
validator's fixed typed orbit issue.

The corpus covers 2 accepted cases and 25 rejected graph invariants, including
duplicate identities, missing references, POEORB matching, transfer and
association coverage, and asset roles.
