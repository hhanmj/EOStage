# EOStage Product Overview & Scope

**Document Type:** Contributor Product Specification
**Maintainers:** EOStage contributors

## Authority And Scope

This file governs product users, jobs, purpose, positioning, trust, privacy, accessibility, and product principles. It does not redefine scientific algorithms, provider contracts, task-state transitions, visual implementation, or agent workflow.

Verified scientific and behavioral contracts remain authoritative for runtime truth. Cross-domain conflicts must be reported before implementation.

## Product Purpose

EOStage is a desktop workspace for discovering, inspecting, obtaining, verifying,
and organizing multi-source geospatial and remote-sensing data. SAR, optical imagery,
DEM, orbit and atmospheric supporting products share an area of interest and a
coherent catalogue-to-delivery workflow. InSAR preparation remains an important
user job, not the definition of the whole product. A general InSAR processing suite,
cloud platform, or plugin marketplace is not required to deliver this purpose.

Geospatial capability serves getting data ready for processing: defining the area,
judging coverage and acquisition geometry, orbits, DEM preparation, and showing what
has been acquired, what is planned, and what is already held locally. General-purpose
spatial analysis (buffering, overlay, zonal statistics and similar operations) is
outside the product scope; users take prepared data to GIS and processing tools for
that work.

The product should reduce operational friction without concealing scientific meaning. It should make the correct workflow easier while leaving professional parameters inspectable and controllable.

Current release scope and verified capabilities are documented in [CHANGELOG.md](CHANGELOG.md) and [docs/README.md](docs/README.md).

## Users And Jobs

Primary users include:

- students and researchers learning InSAR data preparation;
- experienced remote-sensing practitioners managing repeated searches and large downloads;
- teams preparing reproducible input data for downstream scientific tools.

The desktop supports Simplified Chinese (zh-CN), English, and Japanese through an
explicit language preference. zh-CN remains the fixed default; the product does not
infer a user's language from the operating system. Language choice changes the
translated interface without restarting and is stored locally, without telemetry.
The classic `pages/` workbench remains Chinese-only. Scientific identifiers and
user-authored data retain their original identities. Japanese begins as a
machine-translated first pass requiring terminology and semantic review.

Core jobs are:

- define a scientifically valid area and search configuration;
- understand candidate products and metadata before selection;
- download multiple independent batches without cross-task contamination;
- pause, resume, retry, and recover long-running work;
- know what completed, what failed, why it failed, and where outputs were written;
- reuse verified products without downloading duplicates;
- preserve enough context to audit or continue work later.

## Product Success

Success means users can complete data preparation accurately, recoverably, and with less manual bookkeeping. Product quality is measured by behavioral truth, scientific clarity, recoverability, task isolation, useful diagnostics, and efficient repeated operation, not by decorative novelty.

Evaluate a change across the whole user job: define AOI and product requirements,
discover and compare candidates, inspect relevant assets, select the required
delivery, obtain a verifiable output, and recover after interruption. Record
time to useful results/visible preview, actual transferred bytes, peak memory,
recovery behavior, and avoidable user steps on stated fixtures and hardware.
Set performance acceptance against measured baselines before selecting an
implementation; test counts and the number of adapters are not product outcomes.
Measure locally on approved benchmark inputs. This does not authorize user
analytics, installation identifiers, background usage collection or uploads.

## Data Access And Delivery

- The user selects data, scientific parameters and delivery content; EOStage
  selects qualified sources. Source provenance remains inspectable, not a routine
  website-selection burden. Costs, account use and material output changes remain
  explicit user decisions.
- A logical product, a provider/collection record, and a physical asset are
  distinct identities. A SAFE archive, a complete asset set, selected bands, a
  scientific AOI subset, and a rendered picture are different deliveries.
- Search, preview, asset discovery, asset download, archive download and subset
  access have independent capability and readiness evidence. A missing archive
  route must not become a permanent prohibition on an otherwise implementable
  asset route. Conversely, a visible STAC asset never authorizes execution alone.
- Prefer useful, bounded access before compulsory whole-scene transfer where
  the format, service and verified implementation support it. COG is an access
  opportunity, not proof of correct subsetting or universal Range support.
- Unknown metadata must not be invented. Missing optional preview/size metadata
  should affect that operation rather than become a blanket product rejection;
  changes to current strict admission still require scoped implementation/tests.

## Reference-Informed Evolution

Evaluate compatible maintained open-source components to reduce repeated protocol, raster, and download work. Prefer a compatible maintained
component or thin adapter when evidence supports it; retaining self-owned code
needs a concrete correctness, compatibility or lifecycle reason, not merely that
it exists. References inform mechanisms and user outcomes, not automatic dependency
installation, scientific equivalence, permission to copy code or deployment.

One primary state owner allows multiple linked views and replaceable components.
It does not require one giant page or prevent standards-aligned internal models.

## Scientific Trust

- Displayed metadata must preserve its scientific meaning and provenance.
- Unknown, not applicable, not fetched, unavailable, failed, paused, cancelled, and successful are different states.
- Counts, progress, paths, frames, orbit direction, sizes, and completion status must not be inferred merely to fill a UI gap.
- A successful-looking state must be backed by durable runtime state or verified output.
- Search results and task snapshots must remain isolated according to their lifecycle.
- Errors must identify the affected operation and provide a practical next action when one exists.

## Beginner And Professional Balance

Beginner support uses explanation, safe defaults, validation, legal-combination filtering, progressive disclosure, and recovery guidance. It must not remove professional parameters or merge scientifically distinct states.

Advanced controls should be available where they materially change search, selection, download, or processing preparation. Defaults should be explicit and reversible.

## Workflow Continuity

- Long-running work must acknowledge the user's command immediately.
- Search, download, and auxiliary-data tasks keep durable identity and context.
- Closing or restarting the desktop must not silently erase recoverable tasks.
- Pause, resume, retry, cancel, and delete are distinct actions with distinct outcomes.
- Completed, active, and historical task collections must not claim the same task simultaneously unless a documented transition requires it.
- A new search must not inherit another task's transient download state.

## Privacy

The desktop does not add usage telemetry by default. Do not introduce an installation identifier, device fingerprint, startup event, usage analytics, online heartbeat, or live-user counter without an explicit product decision, a clear user benefit, disclosure, consent where required, and a separate privacy review.

Release-download counts may be collected by hosting platforms without adding tracking code to the application.

Credentials and tokens are user secrets. They must not be logged, committed, transmitted to unrelated services, or exposed in screenshots and diagnostics.

## Accessibility And Inclusion

- Core workflows must remain keyboard reachable.
- Controls need stable, meaningful accessible names.
- Status cannot rely on color alone.
- Text and controls must survive Simplified Chinese, English, and Japanese labels, scientific identifiers, long filenames, and long error messages.
- Motion must not be required to understand state and must respect reduced-motion preferences when motion exists.
- Dense scientific information should remain scannable without being hidden from users who need it.

## Product Character

EOStage should feel precise, calm, technical, and work-focused. It is a desktop instrument, not a marketing landing page. The interface should prioritize repeated operation, comparison, traceability, and scientific confidence.

## Product Principles

1. Scientific truth before visual simplification.
2. One business object, one primary state source.
3. Preserve lifecycle boundaries between search state and task snapshots.
4. Immediate acknowledgement for expensive operations.
5. Durable context and recoverable long tasks.
6. Explicit defaults and inspectable professional parameters.
7. Useful failure reasons instead of generic failure labels.
8. Minimal data collection and no hidden user tracking.

## Extension, Skill, Plugin, MCP & Runtime Component Principles

1. **Five distinct runtime layers (never conflated):**
   - **System Rules**: Host-enforced safety invariants (tool permissions, two-step confirmation gates, path containment, credential isolation, and evidence rules). Ship with the host, always apply to the relevant Agent mode, are never exposed as a toggleable user skill, and never contain sensor recipes or scientific thresholds.
   - **Skill**: A standalone or plugin-owned scientific or operational playbook loaded on demand via progressive disclosure. Canonical LLM instructions (`SKILL.md`) MUST be written in structured English covering trigger scope, inputs and units, decision rules, failure/stop conditions, deliverable, and verification protocol; user-facing card metadata (title, summary, prerequisites, example prompts) is localized (`zh-CN`, `en`, `ja`). Reject 4-line filler prompts, persona fluff, and duplicated textbook excerpts; index general reference material separately from executable procedures.
   - **Plugin**: An independently installable, disableable, and removable capability extension bundle from an identified publisher for co-creation. A Plugin MUST contribute at least one genuinely executable capability verified through an ordinary EOStage invocation path (real MCP server, bounded script runner/adapter, or verified host extension) plus a minimal passing fixture; methodology-only bundles MUST publish as Skills, never disguised as Plugins.
   - **MCP Connector**: A real local stdio or remote Streamable HTTP server instance that completes initialization handshake and tool discovery before reporting connected. Built-in host operations use native adapters/channels rather than wrapping `tauri://` commands as fake MCP servers.
   - **Runtime Component**: A shared, host-managed local dependency (such as GDAL/PROJ, EGM vertical datum grids, or the bounded compatibility worker) with platform metadata, whole-package integrity verification, and readiness probes. Plugins and Skills declare component dependencies by ID; uninstalling a plugin never deletes shared components needed by the host or other extensions.
2. **Package ownership and whole-package integrity:**
   - The Rust host is the single authoritative owner of installed packages, enablement state, and effective skills/connectors (`publisher/package/item` isolation).
   - Installing a plugin MUST NOT copy plugin-owned skills or connectors into user-authored custom stores (`skills_custom` / `connectors_custom`); disabling a parent plugin MUST suppress all of its bundled skills and connectors at both discovery and invocation; uninstalling a plugin MUST remove only artifacts owned by that plugin without deleting user-authored entries.
   - Integrity digests MUST hash the complete package contents deterministically (manifest, `SKILL.md`, references, scripts, `mcp.json`, excluding mutable `.disabled` state), never `plugin.json` alone.
3. **Intent-oriented Extension UX:**
   - Organize top-level discovery and management around user intent (**Extensions → Discover** and **Extensions → Installed**, with deep links to **Settings → Connections** and **Settings → Runtime Components**), keeping retired installed packages manageable and cleanly uninstallable.

## Anti-References

Do not turn core workflows into:

- a landing page or feature tour;
- a decorative card gallery;
- a second workbench for the same business object;
- a simplified wizard that hides scientific choices or state truth;
- an interface that reports activity, progress, or success without runtime evidence;
- a telemetry surface introduced merely to count active users.

## Governance Update Triggers

Review this file when the project:

- adds a major data family, processing domain, or user group;
- changes the product's primary workflow or positioning;
- introduces telemetry, accounts, cloud synchronization, or collaboration;
- adds another desktop platform or a materially different distribution model;
- changes the meaning or ownership of search results, tasks, or snapshots;
- discovers that a product principle conflicts with verified scientific behavior.

## Product Review Checklist

- Does the change preserve scientific distinctions and provenance?
- Is there one primary state source for each business object?
- Can the user recover long-running work and understand failures?
- Are beginner aids additive rather than destructive to professional control?
- Does the change avoid hidden tracking and credential exposure?
- Is the scope necessary for the stated user job?
