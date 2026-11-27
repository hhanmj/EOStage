# GACOS Request Workflow V1

This document describes the offline-first request planner and the opt-in,
user-confirmed GACOS form submission path. After the single confirmation, the
application submits batches automatically with the required pacing; the result
link is still obtained by the user from the configured mailbox. It does not add mailbox access,
OAuth, IMAP, automatic result-link discovery, automatic result downloading, or
NISAR processing.

## Sources and scope

- The online source is a selected-only copy of the current Sentinel-1 scene
  snapshot. It never broadens an unchecked selection or starts a second ASF
  search.
- Local sources reuse the ASF scene/cart parser and support ASF Python/text
  exports, CSV, GeoJSON/JSON, SAFE directories, ZIP archives, scene names, and
  supported directory scans. Python files are read as text; they are never
  imported or executed.
- The Processing AOI remains the geometry owner. The plan displays the source
  bbox, buffer, and final request bbox and rejects invalid coordinates or a
  request span above the GACOS limit before submission.

## Time, deduplication, and batches

Complete acquisition datetimes are normalized to UTC. GACOS groups by UTC
`HH:MM`, records discarded seconds, and never combines different UTC times.
Date-only evidence is `DATE_ONLY` until the user supplies a UTC hour and minute;
it is never silently changed to `00:00`. Conflicting observations for one
stable scene identity are `CONFLICT` and block submission.

Stable scene duplicates merge provenance before request-semantic deduplication.
Dates are sorted and split within each UTC time group, with at most 20 dates per
batch. Batch and plan fingerprints are deterministic SHA-256 values over the
normalized request and an irreversible keyring-email identity hash; source paths
and random batch IDs do not affect them.

## Credentials and durable state

The request preview may be built without an email. A real submission resolves a
valid email from the Settings keyring immediately before POST. UI, plan,
fingerprint, ledger, JSON/TXT output, logs, and errors contain only a mask or
identity hash, never the raw address. The existing paste-link download/import
workflow remains the result path.

The shared download archive is the desktop ledger owner; the CLI uses its
atomic JSON ledger. Durable statuses include `READY`, `SUBMITTED`, `FAILED`,
`SUBMISSION_UNCERTAIN`, `SKIPPED_DUPLICATE`, and `CANCELLED`. Submitted,
uncertain, and in-flight requests block duplicate submission. A failed request
can continue only after explicit user intent; force reapply creates a new
attempt and preserves prior records. Restart converts an in-flight POST to
uncertain and never retries it automatically.

## Submission safety

One final confirmation starts the queue. The first batch is immediate; adjacent
batches are separated by at least 20 seconds using an injectable monotonic
clock/sleeper in tests. Waiting is cancellable. An uncertain result stops the
queue; a clear failure stops by default and requires explicit continuation.

All normal tests are offline and use fake clients, keyrings, and clocks. A real
GACOS POST is not part of the default verification gate.
