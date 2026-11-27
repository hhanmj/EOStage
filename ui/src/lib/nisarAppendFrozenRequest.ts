import type {
  DesktopDownloadLayout,
  DesktopNisarAppendSubsetBatchTaskV2Request,
  DesktopNisarSuccessorTaskV2Request,
} from "./desktopShellBridge";

/**
 * Valid download layout values supported by the desktop bridge and Rust serde contract.
 */
export const VALID_NISAR_DOWNLOAD_LAYOUTS: readonly DesktopDownloadLayout[] = Object.freeze([
  "direct",
  "sentinel1_slc",
  "sentinel_orbit_aux_poeorb",
  "sar_dataset_processing",
]);

/**
 * Maximum UTF-8 byte length for a task ID in new-request admission
 * (`validate_nisar_append_task_id` in `nisar_subset.rs`).
 */
export const MAX_NISAR_TASK_ID_BYTES = 96;

/**
 * Maximum Unicode scalar value (code point) count for opaque catalogue/event IDs
 * (`valid_catalogue_opaque_id` in `lib.rs`).
 */
export const MAX_NISAR_OPAQUE_ID_CHARS = 128;

/**
 * Maximum UTF-8 byte length for each NISAR source item ID
 * (`MAX_NISAR_SUBSET_SOURCE_ITEM_ID_BYTES` in `nisar_subset.rs` and `MAX_KEY_BYTES` in `source_gate.rs`).
 */
export const MAX_NISAR_SOURCE_ITEM_ID_BYTES = 1024;

/**
 * Maximum UTF-8 byte length for stored identity key/timestamp strings in persisted gate records
 * (`MAX_KEY_BYTES` in `source_gate.rs`).
 */
export const MAX_NISAR_STORED_KEY_BYTES = 1024;

/**
 * Storage safety upper bound in UTF-8 bytes for `output_root`.
 */
export const MAX_NISAR_OUTPUT_ROOT_BYTES = 32768;

/**
 * Storage safety upper bound in UTF-8 bytes for a serialized frozen request record
 * (`MAX_REQUEST_BYTES` in `source_gate.rs`).
 */
export const MAX_NISAR_FROZEN_REQUEST_BYTES = 16 * 1024 * 1024;

/**
 * Minimum and maximum concurrency allowed by the backend service on new-request admission
 * (`create_nisar_subset_successor_task_v2` in `nisar_subset.rs`).
 */
export const MIN_NISAR_CONCURRENCY = 1;
export const MAX_NISAR_CONCURRENCY = 8;

/**
 * Validation mode:
 * - `"admission"`: Strict backend new-request admission rules (`ApplicationService`).
 * - `"persisted"`: Persisted record & receipt-query identity integrity rules
 *   (enforces exact structure, duplicate/empty source-item checks, UTF-8 validity, and storage
 *   safety limits without discarding stored records over admission-only constraints).
 */
export type NisarFrozenRequestValidationMode = "admission" | "persisted";

/**
 * Deeply readonly representation of `DesktopNisarAppendSubsetBatchTaskV2Request`.
 */
export interface ReadonlyNisarAppendRequest
  extends Omit<Readonly<DesktopNisarAppendSubsetBatchTaskV2Request>, "source_item_ids"> {
  readonly source_item_ids: readonly string[];
}

/**
 * Deeply readonly representation of `DesktopNisarSuccessorTaskV2Request`.
 */
export interface ReadonlyNisarSuccessorRequest
  extends Omit<Readonly<DesktopNisarSuccessorTaskV2Request>, "source_item_ids"> {
  readonly source_item_ids: readonly string[];
}

/**
 * Discriminated frozen wrapper for an exact NISAR append-subset-batch request.
 */
export interface FrozenNisarAppendRequest {
  readonly kind: "append";
  readonly request: ReadonlyNisarAppendRequest;
}

/**
 * Discriminated frozen wrapper for an exact NISAR create-subset-successor request.
 */
export interface FrozenNisarSuccessorRequest {
  readonly kind: "successor";
  readonly request: ReadonlyNisarSuccessorRequest;
}

/**
 * Discriminated union of frozen NISAR source mutation requests.
 */
export type FrozenNisarRequest = FrozenNisarAppendRequest | FrozenNisarSuccessorRequest;

/**
 * Error codes returned by frozen request validation.
 * Error messages intentionally omit all request values, paths, and identifiers.
 */
export type NisarFrozenRequestErrorCode =
  | "INVALID_RECORD_OBJECT"
  | "UNKNOWN_RECORD_KEYS"
  | "INVALID_KIND"
  | "INVALID_REQUEST_OBJECT"
  | "UNKNOWN_REQUEST_KEYS"
  | "MISSING_REQUIRED_KEY"
  | "REQUEST_TOO_LARGE"
  | "INVALID_TASK_ID"
  | "INVALID_SOURCE_TASK_ID"
  | "INVALID_NEW_TASK_ID"
  | "DUPLICATE_TASK_IDS"
  | "INVALID_EVENT_ID"
  | "INVALID_EXPECTED_TASK_REVISION"
  | "INVALID_SOURCE_ITEM_IDS_ARRAY"
  | "INVALID_SOURCE_ITEM_ID"
  | "DUPLICATE_SOURCE_ITEM_ID"
  | "INVALID_OCCURRED_AT"
  | "INVALID_CREATED_AT"
  | "INVALID_OUTPUT_ROOT"
  | "INVALID_LAYOUT"
  | "INVALID_CONCURRENCY"
  | "INVALID_RECREATE_ORIGINAL_ITEMS";

/**
 * Sanitized error thrown when a NISAR request or persisted envelope fails validation.
 * Never contains user identifiers, filesystem paths, or raw field values.
 */
export class NisarFrozenRequestValidationError extends Error {
  public readonly code: NisarFrozenRequestErrorCode;

  public constructor(code: NisarFrozenRequestErrorCode, message: string) {
    super(message);
    this.name = "NisarFrozenRequestValidationError";
    this.code = code;
    Object.setPrototypeOf(this, NisarFrozenRequestValidationError.prototype);
  }
}

/**
 * Discriminated result type for non-throwing validation callers.
 */
export type NisarFrozenRequestValidationResult<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly error: NisarFrozenRequestValidationError };

const APPEND_ALLOWED_KEYS = new Set<string>([
  "task_id",
  "event_id",
  "expected_task_revision",
  "source_item_ids",
  "occurred_at",
]);

const APPEND_REQUIRED_KEYS: readonly string[] = [
  "task_id",
  "event_id",
  "expected_task_revision",
  "source_item_ids",
  "occurred_at",
];

const SUCCESSOR_ALLOWED_KEYS = new Set<string>([
  "source_task_id",
  "new_task_id",
  "expected_task_revision",
  "source_item_ids",
  "output_root",
  "layout",
  "concurrency",
  "created_at",
  "recreate_original_items",
]);

const SUCCESSOR_REQUIRED_KEYS: readonly string[] = [
  "source_task_id",
  "new_task_id",
  "expected_task_revision",
  "source_item_ids",
  "output_root",
  "layout",
  "concurrency",
  "created_at",
];

const RECORD_ALLOWED_KEYS = new Set<string>(["kind", "request"]);

const UTF8_ENCODER = new TextEncoder();

// Matches any lone UTF-16 high or low surrogate (invalid Unicode scalar / ill-formed UTF-8).
const LONE_SURROGATE_REGEX =
  /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/;

// Matches ASCII control characters: 0x00..=0x1F and 0x7F (`byte.is_ascii_control()` in Rust).
// eslint-disable-next-line no-control-regex
const ASCII_CONTROL_CHAR_REGEX = /[\x00-\x1F\x7F]/;

// Matches Unicode General_Category=Control (`char::is_control` in Rust: 0x00..=0x1F, 0x7F..=0x9F).
// eslint-disable-next-line no-control-regex
const UNICODE_CONTROL_CHAR_REGEX = /[\x00-\x1F\x7F-\x9F]/;

// Matches NUL characters forbidden in OS paths and SQLite keys.
// eslint-disable-next-line no-control-regex
const NUL_CHAR_REGEX = /\x00/;

// Matches canonical RFC3339 UTC timestamp format produced by chrono `SecondsFormat::AutoSi, true`.
const NORMALIZED_UTC_REGEX =
  /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d{3}|\d{6}|\d{9}))?Z$/;

const WINDOWS_DRIVE_ABS_REGEX = /^[a-zA-Z]:[/\\]/;
const WINDOWS_UNC_ABS_REGEX = /^\\\\[^\\/?*:|"<>]+\\[^\\/?*:|"<>]+/;
const POSIX_ABS_REGEX = /^\//;

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const proto = Object.getPrototypeOf(value);
  return proto === Object.prototype || proto === null;
}

/**
 * Checks that a JS string contains no lone UTF-16 surrogates, ensuring lossless UTF-8 round-tripping
 * across the Tauri/Rust JSON boundary.
 */
export function isWellFormedUtf8String(value: string): boolean {
  return !LONE_SURROGATE_REGEX.test(value);
}

/**
 * Returns the exact UTF-8 byte length of a string.
 */
export function utf8ByteLength(value: string): number {
  return UTF8_ENCODER.encode(value).byteLength;
}

/**
 * Counts Unicode scalar values (code points), matching Rust `str::chars().count()`.
 */
function countUnicodeScalarValues(value: string): number {
  let count = 0;
  for (let i = 0; i < value.length; i += 1) {
    const code = value.charCodeAt(i);
    if (code >= 0xd800 && code <= 0xdbff && i + 1 < value.length) {
      const next = value.charCodeAt(i + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        i += 1;
      }
    }
    count += 1;
  }
  return count;
}

/**
 * Exact match for Rust `valid_exact_text(value)` in `crates/insar-application-service/src/lib.rs`:
 * `!value.trim().is_empty() && value == value.trim() && !value.chars().any(char::is_control)`
 */
export function isValidExactText(value: unknown): value is string {
  if (typeof value !== "string" || value.length === 0) {
    return false;
  }
  if (!isWellFormedUtf8String(value)) {
    return false;
  }
  if (value.trim().length === 0 || value !== value.trim()) {
    return false;
  }
  if (UNICODE_CONTROL_CHAR_REGEX.test(value)) {
    return false;
  }
  return true;
}

/**
 * Exact match for Rust `valid_catalogue_opaque_id(value)` in `crates/insar-application-service/src/lib.rs`:
 * `valid_opaque_id(value, 128)`:
 * `!value.trim().is_empty() && value.chars().count() <= 128 && value == value.trim() && !value.chars().any(char::is_control)`
 */
export function isValidCatalogueOpaqueId(value: unknown): value is string {
  if (!isValidExactText(value)) {
    return false;
  }
  return countUnicodeScalarValues(value) <= MAX_NISAR_OPAQUE_ID_CHARS;
}

/**
 * Exact match for Rust `validate_nisar_append_task_id(value)` in
 * `crates/insar-application-service/src/nisar_subset.rs`:
 * `valid_catalogue_opaque_id(value) && value.len() <= 96`
 */
export function isValidNisarTaskId(value: unknown): value is string {
  if (!isValidCatalogueOpaqueId(value)) {
    return false;
  }
  return utf8ByteLength(value) <= MAX_NISAR_TASK_ID_BYTES;
}

/**
 * Exact match for Rust `valid_nisar_subset_source_item_id(value)` in
 * `crates/insar-application-service/src/nisar_subset.rs`:
 * `!value.trim().is_empty() && value.len() <= 1024 && !value.bytes().any(|byte| byte.is_ascii_control())`
 *
 * Important contract note:
 * - Leading/trailing or internal non-control whitespace is permitted by the backend as long as
 *   `value.trim()` is non-empty, and the raw string MUST be preserved verbatim without trimming.
 * - Length limit is 1024 UTF-8 bytes (not JS UTF-16 code units).
 */
export function isValidNisarSubsetSourceItemId(value: unknown): value is string {
  if (typeof value !== "string" || value.length === 0) {
    return false;
  }
  if (!isWellFormedUtf8String(value)) {
    return false;
  }
  if (value.trim().length === 0) {
    return false;
  }
  if (ASCII_CONTROL_CHAR_REGEX.test(value)) {
    return false;
  }
  return utf8ByteLength(value) <= MAX_NISAR_SOURCE_ITEM_ID_BYTES;
}

function isLeapYear(year: number): boolean {
  return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
}

function daysInGregorianMonth(year: number, month: number): number {
  switch (month) {
    case 1:
    case 3:
    case 5:
    case 7:
    case 8:
    case 10:
    case 12:
      return 31;
    case 4:
    case 6:
    case 9:
    case 11:
      return 30;
    case 2:
      return isLeapYear(year) ? 29 : 28;
    default:
      return 0;
  }
}

/**
 * Exact match for Rust `parse_normalized_utc(value, field)` in
 * `crates/insar-application-service/src/lib.rs`:
 * 1. `valid_exact_text(value)`
 * 2. `DateTime::parse_from_rfc3339(value)` succeeds with `offset == 0`
 * 3. `parsed.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::AutoSi, true) == value`
 *
 * Under `SecondsFormat::AutoSi` with `use_z = true`:
 * - Offset must be uppercase `Z`.
 * - Date/time components must be calendar-valid (`month` 01..12, `day` 01..daysInMonth,
 *   `hour` 00..23, `minute` 00..59, `second` 00..60).
 * - Fractional seconds, if present, must have 3, 6, or 9 digits and MUST NOT end with `"000"`
 *   (because `AutoSi` truncates trailing 3-digit zero groups and omits `.` entirely when nanos == 0).
 *   For leap second `:60`, chrono folds +1_000_000_000 ns into subseconds; when fractional digits
 *   are present on `:60`, `AutoSi` always formats 9 fractional digits.
 */
export function isValidNormalizedUtcTimestamp(value: unknown): value is string {
  if (!isValidExactText(value)) {
    return false;
  }
  const match = NORMALIZED_UTC_REGEX.exec(value);
  if (!match) {
    return false;
  }
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const hour = Number(match[4]);
  const minute = Number(match[5]);
  const second = Number(match[6]);
  const fraction = match[7];

  const maxDay = daysInGregorianMonth(year, month);
  if (maxDay === 0 || day < 1 || day > maxDay) {
    return false;
  }
  if (hour > 23 || minute > 59 || second > 60) {
    return false;
  }
  if (fraction !== undefined) {
    if (fraction.endsWith("000")) {
      return false;
    }
    if (second === 60 && fraction.length !== 9) {
      return false;
    }
  }
  return true;
}

/**
 * Validates a persisted key/timestamp string under storage safety constraints
 * (`source_gate.rs` `valid_text` + `MAX_KEY_BYTES` = 1024):
 * - Well-formed UTF-8 string
 * - `value.trim().length > 0` (raw string preserved verbatim without trimming)
 * - No NUL characters
 * - UTF-8 byte length <= 1024
 */
function isValidPersistedKeyString(value: unknown): value is string {
  if (typeof value !== "string" || value.length === 0) {
    return false;
  }
  if (!isWellFormedUtf8String(value)) {
    return false;
  }
  if (value.trim().length === 0) {
    return false;
  }
  if (NUL_CHAR_REGEX.test(value)) {
    return false;
  }
  return utf8ByteLength(value) <= MAX_NISAR_STORED_KEY_BYTES;
}

function isValidOutputRoot(
  value: unknown,
  mode: NisarFrozenRequestValidationMode,
): value is string {
  if (typeof value !== "string" || value.length === 0) {
    return false;
  }
  if (!isWellFormedUtf8String(value)) {
    return false;
  }
  if (NUL_CHAR_REGEX.test(value)) {
    return false;
  }
  if (utf8ByteLength(value) > MAX_NISAR_OUTPUT_ROOT_BYTES) {
    return false;
  }
  if (mode === "persisted") {
    // Persisted mode (`source_gate.rs` `valid_text` / `source_receipts.rs`):
    // require non-empty trimmed text without discarding OS-specific path formats or trailing spaces.
    return value.trim().length > 0;
  }
  // Admission mode (`nisar_subset.rs`: `!output_root.as_os_str().is_empty() && output_root.is_absolute()`):
  // reject ASCII control characters and require an absolute Windows drive, UNC, or POSIX root path.
  if (ASCII_CONTROL_CHAR_REGEX.test(value)) {
    return false;
  }
  return (
    WINDOWS_DRIVE_ABS_REGEX.test(value) ||
    WINDOWS_UNC_ABS_REGEX.test(value) ||
    POSIX_ABS_REGEX.test(value)
  );
}

function isValidNonNegativeSafeInt(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isValidConcurrency(
  value: unknown,
  mode: NisarFrozenRequestValidationMode,
): value is number {
  if (typeof value !== "number" || !Number.isSafeInteger(value)) {
    return false;
  }
  if (mode === "persisted") {
    // Rust `usize` in `NisarCreateSubsetSuccessorTaskV2Request` / `StoreNisarSuccessorIdentity`.
    // In persisted mode, allow any positive safe integer so stored records are never lost if
    // admission concurrency bounds evolve.
    return value >= 1;
  }
  return value >= MIN_NISAR_CONCURRENCY && value <= MAX_NISAR_CONCURRENCY;
}

function isValidDownloadLayout(value: unknown): value is DesktopDownloadLayout {
  return (
    typeof value === "string" &&
    (VALID_NISAR_DOWNLOAD_LAYOUTS as readonly string[]).includes(value)
  );
}

function validateAndCloneSourceItemIds(rawIds: unknown): {
  readonly ids: readonly string[];
  readonly totalBytes: number;
} {
  if (!Array.isArray(rawIds) || rawIds.length === 0) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_SOURCE_ITEM_IDS_ARRAY",
      "source_item_ids must be a non-empty array.",
    );
  }

  // Enforce `Object.keys` check on the array to reject sparse arrays or custom enumerable properties
  if (Object.keys(rawIds).length !== rawIds.length) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_SOURCE_ITEM_IDS_ARRAY",
      "source_item_ids must be a dense array without extra properties.",
    );
  }

  const seen = new Set<string>();
  const cloned: string[] = new Array<string>(rawIds.length);
  let totalBytes = 2; // '[' and ']'

  for (let i = 0; i < rawIds.length; i += 1) {
    const item = rawIds[i];
    if (!isValidNisarSubsetSourceItemId(item)) {
      throw new NisarFrozenRequestValidationError(
        "INVALID_SOURCE_ITEM_ID",
        "Each source_item_id must be a well-formed UTF-8 string with non-empty trimmed content, no ASCII control characters, and at most 1024 UTF-8 bytes.",
      );
    }
    if (seen.has(item)) {
      throw new NisarFrozenRequestValidationError(
        "DUPLICATE_SOURCE_ITEM_ID",
        "source_item_ids must not contain duplicate entries.",
      );
    }
    seen.add(item);
    cloned[i] = item;
    totalBytes += utf8ByteLength(item) + 3; // quotes + comma/delimiter estimate
    if (totalBytes > MAX_NISAR_FROZEN_REQUEST_BYTES) {
      throw new NisarFrozenRequestValidationError(
        "REQUEST_TOO_LARGE",
        "Request exceeds the maximum allowed storage size.",
      );
    }
  }

  return {
    ids: Object.freeze(cloned),
    totalBytes,
  };
}

function assertExactObjectKeys(
  obj: Record<string, unknown>,
  allowedKeys: ReadonlySet<string>,
  requiredKeys: readonly string[],
  unknownKeyCode: NisarFrozenRequestErrorCode,
): void {
  if (Object.getOwnPropertySymbols(obj).length > 0) {
    throw new NisarFrozenRequestValidationError(
      unknownKeyCode,
      "Object contains unexpected symbol keys.",
    );
  }

  const keys = Object.keys(obj);
  for (let i = 0; i < keys.length; i += 1) {
    if (!allowedKeys.has(keys[i])) {
      throw new NisarFrozenRequestValidationError(
        unknownKeyCode,
        "Object contains unknown or unsupported properties.",
      );
    }
  }

  for (let i = 0; i < requiredKeys.length; i += 1) {
    const reqKey = requiredKeys[i];
    if (!Object.prototype.hasOwnProperty.call(obj, reqKey)) {
      throw new NisarFrozenRequestValidationError(
        "MISSING_REQUIRED_KEY",
        "Object is missing a required property.",
      );
    }
  }
}

function validateAndFreezeAppendPayloadInternal(
  raw: unknown,
  mode: NisarFrozenRequestValidationMode,
): ReadonlyNisarAppendRequest {
  if (!isPlainRecord(raw)) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_REQUEST_OBJECT",
      "Append request payload must be a plain object.",
    );
  }

  assertExactObjectKeys(
    raw,
    APPEND_ALLOWED_KEYS,
    APPEND_REQUIRED_KEYS,
    "UNKNOWN_REQUEST_KEYS",
  );

  const taskId = raw.task_id;
  const eventId = raw.event_id;
  const expectedTaskRevision = raw.expected_task_revision;
  const sourceItemIds = raw.source_item_ids;
  const occurredAt = raw.occurred_at;

  const isTaskIdValid =
    mode === "admission"
      ? isValidNisarTaskId(taskId)
      : isValidPersistedKeyString(taskId);
  if (typeof taskId !== "string" || !isTaskIdValid) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_TASK_ID",
      "task_id is invalid.",
    );
  }

  const isEventIdValid =
    mode === "admission"
      ? isValidCatalogueOpaqueId(eventId)
      : isValidPersistedKeyString(eventId);
  if (typeof eventId !== "string" || !isEventIdValid) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_EVENT_ID",
      "event_id is invalid.",
    );
  }

  if (!isValidNonNegativeSafeInt(expectedTaskRevision)) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_EXPECTED_TASK_REVISION",
      "expected_task_revision must be a non-negative safe integer.",
    );
  }

  const { ids: frozenIds, totalBytes: idsBytes } =
    validateAndCloneSourceItemIds(sourceItemIds);

  const isOccurredAtValid =
    mode === "admission"
      ? isValidNormalizedUtcTimestamp(occurredAt)
      : isValidPersistedKeyString(occurredAt);
  if (typeof occurredAt !== "string" || !isOccurredAtValid) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_OCCURRED_AT",
      "occurred_at is invalid.",
    );
  }

  const estimatedBytes =
    idsBytes +
    utf8ByteLength(taskId) +
    utf8ByteLength(eventId) +
    utf8ByteLength(occurredAt) +
    256;
  if (estimatedBytes > MAX_NISAR_FROZEN_REQUEST_BYTES) {
    throw new NisarFrozenRequestValidationError(
      "REQUEST_TOO_LARGE",
      "Append request exceeds the maximum allowed storage size.",
    );
  }

  return Object.freeze({
    task_id: taskId,
    event_id: eventId,
    expected_task_revision: expectedTaskRevision,
    source_item_ids: frozenIds,
    occurred_at: occurredAt,
  });
}

function validateAndFreezeSuccessorPayloadInternal(
  raw: unknown,
  mode: NisarFrozenRequestValidationMode,
): ReadonlyNisarSuccessorRequest {
  if (!isPlainRecord(raw)) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_REQUEST_OBJECT",
      "Successor request payload must be a plain object.",
    );
  }

  assertExactObjectKeys(
    raw,
    SUCCESSOR_ALLOWED_KEYS,
    SUCCESSOR_REQUIRED_KEYS,
    "UNKNOWN_REQUEST_KEYS",
  );

  const sourceTaskId = raw.source_task_id;
  const newTaskId = raw.new_task_id;
  const expectedTaskRevision = raw.expected_task_revision;
  const sourceItemIds = raw.source_item_ids;
  const outputRoot = raw.output_root;
  const layout = raw.layout;
  const concurrency = raw.concurrency;
  const createdAt = raw.created_at;

  const isSourceTaskIdValid =
    mode === "admission"
      ? isValidNisarTaskId(sourceTaskId)
      : isValidPersistedKeyString(sourceTaskId);
  if (typeof sourceTaskId !== "string" || !isSourceTaskIdValid) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_SOURCE_TASK_ID",
      "source_task_id is invalid.",
    );
  }

  const isNewTaskIdValid =
    mode === "admission"
      ? isValidNisarTaskId(newTaskId)
      : isValidPersistedKeyString(newTaskId);
  if (typeof newTaskId !== "string" || !isNewTaskIdValid) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_NEW_TASK_ID",
      "new_task_id is invalid.",
    );
  }

  if (sourceTaskId === newTaskId) {
    throw new NisarFrozenRequestValidationError(
      "DUPLICATE_TASK_IDS",
      "source_task_id and new_task_id must be distinct.",
    );
  }

  if (!isValidNonNegativeSafeInt(expectedTaskRevision)) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_EXPECTED_TASK_REVISION",
      "expected_task_revision must be a non-negative safe integer.",
    );
  }

  const { ids: frozenIds, totalBytes: idsBytes } =
    validateAndCloneSourceItemIds(sourceItemIds);

  if (!isValidOutputRoot(outputRoot, mode)) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_OUTPUT_ROOT",
      "output_root is invalid.",
    );
  }

  if (!isValidDownloadLayout(layout)) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_LAYOUT",
      "layout must be a supported DesktopDownloadLayout.",
    );
  }

  if (!isValidConcurrency(concurrency, mode)) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_CONCURRENCY",
      "concurrency is invalid.",
    );
  }

  const isCreatedAtValid =
    mode === "admission"
      ? isValidNormalizedUtcTimestamp(createdAt)
      : isValidPersistedKeyString(createdAt);
  if (typeof createdAt !== "string" || !isCreatedAtValid) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_CREATED_AT",
      "created_at is invalid.",
    );
  }

  const hasRecreateKey = Object.prototype.hasOwnProperty.call(
    raw,
    "recreate_original_items",
  );
  const recreateOriginalItems = raw.recreate_original_items;
  if (
    hasRecreateKey &&
    recreateOriginalItems !== undefined &&
    typeof recreateOriginalItems !== "boolean"
  ) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_RECREATE_ORIGINAL_ITEMS",
      "recreate_original_items must be a boolean when provided.",
    );
  }

  const estimatedBytes =
    idsBytes +
    utf8ByteLength(sourceTaskId) +
    utf8ByteLength(newTaskId) +
    utf8ByteLength(outputRoot) +
    utf8ByteLength(layout) +
    utf8ByteLength(createdAt) +
    256;
  if (estimatedBytes > MAX_NISAR_FROZEN_REQUEST_BYTES) {
    throw new NisarFrozenRequestValidationError(
      "REQUEST_TOO_LARGE",
      "Successor request exceeds the maximum allowed storage size.",
    );
  }

  const cloned: DesktopNisarSuccessorTaskV2Request =
    hasRecreateKey && typeof recreateOriginalItems === "boolean"
      ? {
          source_task_id: sourceTaskId,
          new_task_id: newTaskId,
          expected_task_revision: expectedTaskRevision,
          source_item_ids: frozenIds as unknown as string[],
          output_root: outputRoot,
          layout,
          concurrency,
          created_at: createdAt,
          recreate_original_items: recreateOriginalItems,
        }
      : {
          source_task_id: sourceTaskId,
          new_task_id: newTaskId,
          expected_task_revision: expectedTaskRevision,
          source_item_ids: frozenIds as unknown as string[],
          output_root: outputRoot,
          layout,
          concurrency,
          created_at: createdAt,
        };

  return Object.freeze(cloned) as ReadonlyNisarSuccessorRequest;
}

function validateFrozenEnvelopeInternal(
  raw: unknown,
  mode: NisarFrozenRequestValidationMode,
): FrozenNisarRequest {
  if (!isPlainRecord(raw)) {
    throw new NisarFrozenRequestValidationError(
      "INVALID_RECORD_OBJECT",
      "Frozen request record must be a plain object.",
    );
  }

  assertExactObjectKeys(
    raw,
    RECORD_ALLOWED_KEYS,
    ["kind", "request"],
    "UNKNOWN_RECORD_KEYS",
  );

  const kind = raw.kind;
  if (kind === "append") {
    return Object.freeze({
      kind: "append",
      request: validateAndFreezeAppendPayloadInternal(raw.request, mode),
    });
  }
  if (kind === "successor") {
    return Object.freeze({
      kind: "successor",
      request: validateAndFreezeSuccessorPayloadInternal(raw.request, mode),
    });
  }

  throw new NisarFrozenRequestValidationError(
    "INVALID_KIND",
    'Record kind must be "append" or "successor".',
  );
}

/**
 * Validates and deeply freezes an exact `DesktopNisarAppendSubsetBatchTaskV2Request` payload
 * under strict new-request admission rules (`ApplicationService::append_nisar_subset_batch_task_v2`).
 * Does not mutate the input object or normalize any field values.
 */
export function validateAndFreezeNisarAppendPayload(
  raw: unknown,
): ReadonlyNisarAppendRequest {
  return validateAndFreezeAppendPayloadInternal(raw, "admission");
}

/**
 * Validates and deeply freezes an exact `DesktopNisarAppendSubsetBatchTaskV2Request` payload
 * from persisted storage or for receipt-status querying.
 * Preserves all fields and array order verbatim without applying stricter new-request admission rules.
 */
export function validateAndFreezePersistedNisarAppendPayload(
  raw: unknown,
): ReadonlyNisarAppendRequest {
  return validateAndFreezeAppendPayloadInternal(raw, "persisted");
}

/**
 * Validates and deeply freezes an exact `DesktopNisarSuccessorTaskV2Request` payload
 * under strict new-request admission rules (`ApplicationService::create_nisar_subset_successor_task_v2`).
 * Does not mutate the input object or normalize any field values.
 */
export function validateAndFreezeNisarSuccessorPayload(
  raw: unknown,
): ReadonlyNisarSuccessorRequest {
  return validateAndFreezeSuccessorPayloadInternal(raw, "admission");
}

/**
 * Validates and deeply freezes an exact `DesktopNisarSuccessorTaskV2Request` payload
 * from persisted storage or for receipt-status querying.
 * Preserves all fields and array order verbatim without applying stricter new-request admission rules.
 */
export function validateAndFreezePersistedNisarSuccessorPayload(
  raw: unknown,
): ReadonlyNisarSuccessorRequest {
  return validateAndFreezeSuccessorPayloadInternal(raw, "persisted");
}

/**
 * Creates a discriminated `FrozenNisarAppendRequest` under new-request admission rules.
 */
export function freezeNisarAppendRequest(
  payload: DesktopNisarAppendSubsetBatchTaskV2Request | ReadonlyNisarAppendRequest,
): FrozenNisarAppendRequest {
  const frozenPayload = validateAndFreezeNisarAppendPayload(payload);
  return Object.freeze({
    kind: "append",
    request: frozenPayload,
  });
}

/**
 * Restores a discriminated `FrozenNisarAppendRequest` under persisted identity integrity rules.
 */
export function restoreFrozenNisarAppendRequest(
  payload: unknown,
): FrozenNisarAppendRequest {
  const frozenPayload = validateAndFreezePersistedNisarAppendPayload(payload);
  return Object.freeze({
    kind: "append",
    request: frozenPayload,
  });
}

/**
 * Creates a discriminated `FrozenNisarSuccessorRequest` under new-request admission rules.
 */
export function freezeNisarSuccessorRequest(
  payload: DesktopNisarSuccessorTaskV2Request | ReadonlyNisarSuccessorRequest,
): FrozenNisarSuccessorRequest {
  const frozenPayload = validateAndFreezeNisarSuccessorPayload(payload);
  return Object.freeze({
    kind: "successor",
    request: frozenPayload,
  });
}

/**
 * Restores a discriminated `FrozenNisarSuccessorRequest` under persisted identity integrity rules.
 */
export function restoreFrozenNisarSuccessorRequest(
  payload: unknown,
): FrozenNisarSuccessorRequest {
  const frozenPayload = validateAndFreezePersistedNisarSuccessorPayload(payload);
  return Object.freeze({
    kind: "successor",
    request: frozenPayload,
  });
}

/**
 * Validates an untrusted discriminated record (`{ kind, request }`) under new-request admission rules
 * and returns a fresh deeply frozen `FrozenNisarRequest`.
 */
export function validateFrozenNisarRequest(raw: unknown): FrozenNisarRequest {
  return validateFrozenEnvelopeInternal(raw, "admission");
}

/**
 * Restores and validates an untrusted discriminated record (`{ kind, request }`) from persisted
 * storage under identity integrity rules without discarding records that only fail stricter
 * new-request admission checks.
 */
export function restoreFrozenNisarRequest(raw: unknown): FrozenNisarRequest {
  return validateFrozenEnvelopeInternal(raw, "persisted");
}

/**
 * Alias for `restoreFrozenNisarRequest` for explicit persisted validation call sites.
 */
export const validatePersistedFrozenNisarRequest = restoreFrozenNisarRequest;

/**
 * Non-throwing wrapper around `validateFrozenNisarRequest` (new-request admission mode).
 */
export function tryValidateFrozenNisarRequest(
  raw: unknown,
): NisarFrozenRequestValidationResult<FrozenNisarRequest> {
  try {
    return { ok: true, value: validateFrozenNisarRequest(raw) };
  } catch (err) {
    if (err instanceof NisarFrozenRequestValidationError) {
      return { ok: false, error: err };
    }
    return {
      ok: false,
      error: new NisarFrozenRequestValidationError(
        "INVALID_RECORD_OBJECT",
        "Unexpected validation failure.",
      ),
    };
  }
}

/**
 * Non-throwing wrapper around `restoreFrozenNisarRequest` (persisted identity integrity mode).
 */
export function tryRestoreFrozenNisarRequest(
  raw: unknown,
): NisarFrozenRequestValidationResult<FrozenNisarRequest> {
  try {
    return { ok: true, value: restoreFrozenNisarRequest(raw) };
  } catch (err) {
    if (err instanceof NisarFrozenRequestValidationError) {
      return { ok: false, error: err };
    }
    return {
      ok: false,
      error: new NisarFrozenRequestValidationError(
        "INVALID_RECORD_OBJECT",
        "Unexpected validation failure.",
      ),
    };
  }
}

/**
 * Type guard checking whether `value` is a valid `FrozenNisarAppendRequest` under admission rules.
 */
export function isFrozenNisarAppendRequest(
  value: unknown,
): value is FrozenNisarAppendRequest {
  if (!isPlainRecord(value) || value.kind !== "append") {
    return false;
  }
  try {
    validateFrozenEnvelopeInternal(value, "admission");
    return true;
  } catch {
    return false;
  }
}

/**
 * Type guard checking whether `value` is a valid `FrozenNisarAppendRequest` under persisted identity rules.
 */
export function isPersistedFrozenNisarAppendRequest(
  value: unknown,
): value is FrozenNisarAppendRequest {
  if (!isPlainRecord(value) || value.kind !== "append") {
    return false;
  }
  try {
    validateFrozenEnvelopeInternal(value, "persisted");
    return true;
  } catch {
    return false;
  }
}

/**
 * Type guard checking whether `value` is a valid `FrozenNisarSuccessorRequest` under admission rules.
 */
export function isFrozenNisarSuccessorRequest(
  value: unknown,
): value is FrozenNisarSuccessorRequest {
  if (!isPlainRecord(value) || value.kind !== "successor") {
    return false;
  }
  try {
    validateFrozenEnvelopeInternal(value, "admission");
    return true;
  } catch {
    return false;
  }
}

/**
 * Type guard checking whether `value` is a valid `FrozenNisarSuccessorRequest` under persisted identity rules.
 */
export function isPersistedFrozenNisarSuccessorRequest(
  value: unknown,
): value is FrozenNisarSuccessorRequest {
  if (!isPlainRecord(value) || value.kind !== "successor") {
    return false;
  }
  try {
    validateFrozenEnvelopeInternal(value, "persisted");
    return true;
  } catch {
    return false;
  }
}

/**
 * Type guard checking whether `value` is a valid `FrozenNisarRequest` under admission rules.
 */
export function isFrozenNisarRequest(
  value: unknown,
): value is FrozenNisarRequest {
  const result = tryValidateFrozenNisarRequest(value);
  return result.ok;
}

/**
 * Type guard checking whether `value` is a valid `FrozenNisarRequest` under persisted identity rules.
 */
export function isPersistedFrozenNisarRequest(
  value: unknown,
): value is FrozenNisarRequest {
  const result = tryRestoreFrozenNisarRequest(value);
  return result.ok;
}

/**
 * Produces a fresh mutable clone of a `DesktopNisarAppendSubsetBatchTaskV2Request`
 * suitable for passing to the desktop bridge without sharing array or object references.
 * Preserves persisted records verbatim by validating with `"persisted"` identity integrity rules
 * unless `mode` is explicitly set to `"admission"`.
 */
export function cloneNisarAppendRequestPayload(
  source: ReadonlyNisarAppendRequest | DesktopNisarAppendSubsetBatchTaskV2Request,
  mode: NisarFrozenRequestValidationMode = "persisted",
): DesktopNisarAppendSubsetBatchTaskV2Request {
  const validated = validateAndFreezeAppendPayloadInternal(source, mode);
  return {
    task_id: validated.task_id,
    event_id: validated.event_id,
    expected_task_revision: validated.expected_task_revision,
    source_item_ids: [...validated.source_item_ids],
    occurred_at: validated.occurred_at,
  };
}

/**
 * Produces a fresh mutable clone of a `DesktopNisarSuccessorTaskV2Request`
 * suitable for passing to the desktop bridge without sharing array or object references.
 * Preserves persisted records verbatim by validating with `"persisted"` identity integrity rules
 * unless `mode` is explicitly set to `"admission"`.
 */
export function cloneNisarSuccessorRequestPayload(
  source: ReadonlyNisarSuccessorRequest | DesktopNisarSuccessorTaskV2Request,
  mode: NisarFrozenRequestValidationMode = "persisted",
): DesktopNisarSuccessorTaskV2Request {
  const validated = validateAndFreezeSuccessorPayloadInternal(source, mode);
  const base: DesktopNisarSuccessorTaskV2Request = {
    source_task_id: validated.source_task_id,
    new_task_id: validated.new_task_id,
    expected_task_revision: validated.expected_task_revision,
    source_item_ids: [...validated.source_item_ids],
    output_root: validated.output_root,
    layout: validated.layout,
    concurrency: validated.concurrency,
    created_at: validated.created_at,
  };
  if (validated.recreate_original_items !== undefined) {
    base.recreate_original_items = validated.recreate_original_items;
  }
  return base;
}

/**
 * Produces a fresh deeply frozen clone of a `FrozenNisarAppendRequest`.
 * Defaults to `"persisted"` mode so already-stored records are never rejected or altered during cloning.
 */
export function cloneFrozenNisarAppendRequest(
  record: FrozenNisarAppendRequest,
  mode: NisarFrozenRequestValidationMode = "persisted",
): FrozenNisarAppendRequest {
  if (!isPlainRecord(record) || record.kind !== "append") {
    throw new NisarFrozenRequestValidationError(
      "INVALID_KIND",
      'Expected a frozen record with kind "append".',
    );
  }
  assertExactObjectKeys(
    record,
    RECORD_ALLOWED_KEYS,
    ["kind", "request"],
    "UNKNOWN_RECORD_KEYS",
  );
  return Object.freeze({
    kind: "append",
    request: validateAndFreezeAppendPayloadInternal(record.request, mode),
  });
}

/**
 * Produces a fresh deeply frozen clone of a `FrozenNisarSuccessorRequest`.
 * Defaults to `"persisted"` mode so already-stored records are never rejected or altered during cloning.
 */
export function cloneFrozenNisarSuccessorRequest(
  record: FrozenNisarSuccessorRequest,
  mode: NisarFrozenRequestValidationMode = "persisted",
): FrozenNisarSuccessorRequest {
  if (!isPlainRecord(record) || record.kind !== "successor") {
    throw new NisarFrozenRequestValidationError(
      "INVALID_KIND",
      'Expected a frozen record with kind "successor".',
    );
  }
  assertExactObjectKeys(
    record,
    RECORD_ALLOWED_KEYS,
    ["kind", "request"],
    "UNKNOWN_RECORD_KEYS",
  );
  return Object.freeze({
    kind: "successor",
    request: validateAndFreezeSuccessorPayloadInternal(record.request, mode),
  });
}

/**
 * Produces a fresh deeply frozen clone of any `FrozenNisarRequest`.
 * Defaults to `"persisted"` mode so already-stored records are never rejected or altered during cloning.
 */
export function cloneFrozenNisarRequest(
  record: FrozenNisarRequest,
  mode: NisarFrozenRequestValidationMode = "persisted",
): FrozenNisarRequest {
  return validateFrozenEnvelopeInternal(record, mode);
}

function areStringArraysStrictlyEqual(
  a: readonly string[],
  b: readonly string[],
): boolean {
  if (a.length !== b.length) {
    return false;
  }
  for (let i = 0; i < a.length; i += 1) {
    if (a[i] !== b[i]) {
      return false;
    }
  }
  return true;
}

/**
 * Exact field-by-field and array-order equality comparison for two append request payloads.
 */
export function areNisarAppendPayloadsEqual(
  a: ReadonlyNisarAppendRequest | DesktopNisarAppendSubsetBatchTaskV2Request,
  b: ReadonlyNisarAppendRequest | DesktopNisarAppendSubsetBatchTaskV2Request,
): boolean {
  return (
    a.task_id === b.task_id &&
    a.event_id === b.event_id &&
    a.expected_task_revision === b.expected_task_revision &&
    a.occurred_at === b.occurred_at &&
    areStringArraysStrictlyEqual(a.source_item_ids, b.source_item_ids)
  );
}

/**
 * Exact field-by-field and array-order equality comparison for two successor request payloads.
 * Note: `recreate_original_items` is compared strictly (`===`) so omitted/`undefined` vs `false`
 * remains distinguishable on the TypeScript side before serde serialization.
 */
export function areNisarSuccessorPayloadsEqual(
  a: ReadonlyNisarSuccessorRequest | DesktopNisarSuccessorTaskV2Request,
  b: ReadonlyNisarSuccessorRequest | DesktopNisarSuccessorTaskV2Request,
): boolean {
  return (
    a.source_task_id === b.source_task_id &&
    a.new_task_id === b.new_task_id &&
    a.expected_task_revision === b.expected_task_revision &&
    a.output_root === b.output_root &&
    a.layout === b.layout &&
    a.concurrency === b.concurrency &&
    a.created_at === b.created_at &&
    a.recreate_original_items === b.recreate_original_items &&
    areStringArraysStrictlyEqual(a.source_item_ids, b.source_item_ids)
  );
}

/**
 * Exact equality check for two `FrozenNisarAppendRequest` records.
 */
export function areFrozenNisarAppendRequestsEqual(
  a: FrozenNisarAppendRequest,
  b: FrozenNisarAppendRequest,
): boolean {
  if (a.kind !== "append" || b.kind !== "append") {
    return false;
  }
  return areNisarAppendPayloadsEqual(a.request, b.request);
}

/**
 * Exact equality check for two `FrozenNisarSuccessorRequest` records.
 */
export function areFrozenNisarSuccessorRequestsEqual(
  a: FrozenNisarSuccessorRequest,
  b: FrozenNisarSuccessorRequest,
): boolean {
  if (a.kind !== "successor" || b.kind !== "successor") {
    return false;
  }
  return areNisarSuccessorPayloadsEqual(a.request, b.request);
}

/**
 * Exact equality check for any two `FrozenNisarRequest` records.
 */
export function areFrozenNisarRequestsEqual(
  a: FrozenNisarRequest,
  b: FrozenNisarRequest,
): boolean {
  if (a.kind === "append" && b.kind === "append") {
    return areNisarAppendPayloadsEqual(a.request, b.request);
  }
  if (a.kind === "successor" && b.kind === "successor") {
    return areNisarSuccessorPayloadsEqual(a.request, b.request);
  }
  return false;
}
