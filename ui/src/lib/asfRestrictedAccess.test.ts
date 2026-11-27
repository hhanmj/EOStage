import { strict as assert } from "node:assert";
import test from "node:test";

import {
  ASF_DATASET_ACCESS_RESTRICTED,
  asfRestrictedAccessPresentation,
  asfRestrictedAccessNoticeKey,
} from "./asfRestrictedAccess.ts";

test("JERS transfer-time access rejection projects the translated ASF support response", () => {
  assert.deepEqual(
    asfRestrictedAccessPresentation("JERS-1", ASF_DATASET_ACCESS_RESTRICTED),
    {
      title: "禁止访问",
      badge: "🔒 访问受限",
      message: "JERS-1 是受限数据集。有关如何访问这些数据的信息，请联系 ASF 用户支持。",
      supportLabel: "ASF 用户支持",
      supportUrl: "https://asf.alaska.edu/asf/contact/",
    },
  );
});

test("restricted notice key de-duplicates polling but permits a new task revision", () => {
  const first = asfRestrictedAccessNoticeKey(
    "jers-task",
    3,
    ASF_DATASET_ACCESS_RESTRICTED,
  );
  assert.equal(
    first,
    asfRestrictedAccessNoticeKey("jers-task", 3, ASF_DATASET_ACCESS_RESTRICTED),
  );
  assert.notEqual(
    first,
    asfRestrictedAccessNoticeKey("jers-task", 4, ASF_DATASET_ACCESS_RESTRICTED),
  );
});

test("ordinary credential failures and non-JERS datasets keep their normal failure presentation", () => {
  assert.equal(asfRestrictedAccessPresentation("JERS-1", "DL004"), null);
  assert.equal(
    asfRestrictedAccessPresentation("ERS", ASF_DATASET_ACCESS_RESTRICTED),
    null,
  );
  assert.equal(asfRestrictedAccessPresentation(null, ASF_DATASET_ACCESS_RESTRICTED), null);
});
