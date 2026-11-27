import assert from "node:assert/strict";
import test from "node:test";
import { setCurrentLocale } from "../../i18n/index.ts";
import { approvedExternalUrl } from "../../lib/externalNavigation.ts";
import {
  ACCOUNT_PORTAL_LINKS,
  resolveAccountPortalLink,
  type AccountPortalTarget,
} from "./accountPortalLinks.ts";

test("all account portal targets expose approved HTTP/HTTPS URLs and localize dynamically across zh-CN, en, and ja", () => {
  const expectedTargets: AccountPortalTarget[] = [
    "cdse",
    "cdse_register",
    "earthdata",
    "earthdata_register",
    "earthdata_token",
    "bhoonidhi",
    "bhoonidhi_register",
    "usgs_m2m",
    "usgs_register",
    "usgs_token",
    "gacos",
    "mpc",
    "opentopography",
    "opentopography_key",
    "gee",
    "gee_project_id",
    "gee_service_account",
    "gee_oauth_token",
  ];

  assert.deepEqual(Object.keys(ACCOUNT_PORTAL_LINKS).sort(), [...expectedTargets].sort());
  assert.equal(resolveAccountPortalLink("unknown_provider"), null);

  try {
    for (const target of expectedTargets) {
      const spec = resolveAccountPortalLink(target);
      assert.ok(spec, `Expected spec for ${target}`);
      assert.ok(
        approvedExternalUrl(spec.url),
        `Expected approved HTTP/HTTPS URL for ${target}, got ${spec.url}`,
      );

      setCurrentLocale("zh-CN");
      const zhLabel = spec.label;
      assert.ok(zhLabel.trim().length > 0);

      setCurrentLocale("en");
      const enLabel = spec.label;
      assert.ok(enLabel.trim().length > 0);
      assert.ok(!/[\u4e00-\u9fff]/.test(enLabel), `Expected English label for ${target}, got ${enLabel}`);

      setCurrentLocale("ja");
      const jaLabel = spec.label;
      assert.ok(jaLabel.trim().length > 0);
      assert.notEqual(jaLabel, enLabel);
    }
  } finally {
    setCurrentLocale("zh-CN");
  }
});
