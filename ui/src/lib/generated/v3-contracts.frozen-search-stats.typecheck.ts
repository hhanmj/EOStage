import type { FrozenSearchStats } from "./v3-contracts";

const allowlisted: FrozenSearchStats = {
  matched_count: 4,
  fetched_record_count: 4,
  mapped_record_count: 4,
  provider_returned_count: 4,
  frozen_candidate_count: 4,
  truncated: false,
  source: "ASF_BATCH",
};
void allowlisted;

const hostile = {
  ...allowlisted,
  // @ts-expect-error Frozen search statistics retain only canonical route labels.
  source: "https://example.invalid/search",
} satisfies FrozenSearchStats;
void hostile;
