---
name: spatial-sql
description: "Write, review, debug or optimize spatial SQL in PostGIS or DuckDB Spatial: bbox gates and exact predicates, spatial joins, GiST and expression indexes, EXPLAIN plans, geography vs geometry, SRID/CRS labels, axis order, metric and geodesic correctness. Non-spatial SQL over data files: duckdb."
---

# Spatial SQL

Solve the query in the engine the user has chosen. Read
`references/spatial-sql.md` for the relevant engine and query patterns, and
`references/formats-and-crs.md` when coordinate or format semantics matter.

## Query workflow

1. Establish the engine/version, available spatial functions, geometry versus
   geography types, CRS/SRID, input grain, expected result and scale. Inspect
   the existing query and plan when available. Do not assume different engines
   share function signatures, boundary semantics or index behavior.
2. Express the intended spatial relationship precisely. Resolve intersects
   versus contains/covers, boundary inclusion, null/empty/invalid geometries,
   and whether several spatial matches should duplicate an entity or produce
   a single membership result.
3. Make distance, area and buffers use the intended units and method. A
   geographic geometry in EPSG:4326 does not acquire metre units from a numeric
   threshold. Use the engine's explicitly supported geodesic/geography
   operation, or a projected CRS whose area of use contains the data's known
   extent. For distance and within-distance tests on an unknown or wide
   extent, prefer an operation the engine documents as geodesic for the
   geometry types involved; do not guess a UTM zone or national grid. A
   geography type does not make every operation geodesic: check buffers and
   other constructions per operation. Assigning an SRID is not coordinate
   transformation; Web Mercator is not a general metric-analysis CRS.
4. Preserve analytical correctness while improving execution. Consider spatial
   indexes or candidate filters, predicate pushdown, partition pruning and
   join cardinality. An expression or cast can prevent use of an existing
   index; verify the relevant plan instead of promising index use by syntax.
5. Run a small meaningful control when an engine is available: qualifying and
   non-qualifying features, a boundary, and duplicate matches where relevant.
   Report actual execution evidence separately from a reasoned SQL review.
   An unavailable engine is `not_testable`, not a successful query run.
   If PostGIS or DuckDB Spatial is available in your local environment, run the control query against it (for DuckDB, run `INSTALL spatial; LOAD spatial;` first).

Return corrected SQL, consequential assumptions and a short explanation of
correctness and performance. Do not create a full GIS project for a bounded
query review. Preserve source data; an optimization must not silently change
the metric, feature population or authoritative geometry.
