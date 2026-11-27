import assert from "node:assert/strict";
import test from "node:test";

import {
  CANONICAL_CHINA_PROVINCE_ORDER,
  stableAdminOptionOrder,
} from "./adminOptionOrdering.ts";

test("DataV options have one exact-adcode order regardless of response order", () => {
  const options = [
    { name: "黄石市", provider_object_id: "420200" },
    { name: "全部", provider_object_id: null },
    { name: "恩施土家族苗族自治州", provider_object_id: "422800" },
    { name: "武汉市", provider_object_id: "420100" },
  ];
  const reversed = [...options].reverse();
  const expected = [null, "420100", "420200", "422800"];

  for (const input of [options, reversed]) {
    assert.deepEqual(
      stableAdminOptionOrder(input).map((option) => option.provider_object_id),
      expected,
    );
  }
});

test("fallback names keep their source order and never acquire a fabricated adcode", () => {
  const fallback = [
    { name: "全部", provider_object_id: null },
    { name: "甲县", provider_object_id: null },
    { name: "乙县", provider_object_id: null },
  ];
  assert.deepEqual(stableAdminOptionOrder(fallback), fallback);
});

test("province names use one product-owned order with or without provider identities", () => {
  const shuffled = [
    { name: "台湾省", provider_object_id: null },
    { name: "湖北省", provider_object_id: "420000" },
    { name: "北京市", provider_object_id: null },
    { name: "上海市", provider_object_id: "310000" },
    { name: "全部", provider_object_id: null },
  ];
  const expectedNames = ["全部", "北京市", "上海市", "湖北省", "台湾省"];

  for (const input of [shuffled, [...shuffled].reverse()]) {
    const ordered = stableAdminOptionOrder(input);
    assert.deepEqual(ordered.map((option) => option.name), expectedNames);
    assert.equal(ordered.find((option) => option.name === "台湾省")?.provider_object_id, null);
  }
  assert.equal(new Set(CANONICAL_CHINA_PROVINCE_ORDER).size, 34);
});

test("identified and unidentified province reloads have byte-for-byte equivalent name order", () => {
  const provider = CANONICAL_CHINA_PROVINCE_ORDER
    .map((name, index) => ({ name, provider_object_id: `${index + 11}0000`.slice(0, 6) }))
    .reverse();
  const fallback = CANONICAL_CHINA_PROVINCE_ORDER
    .map((name) => ({ name, provider_object_id: null }))
    .reverse();

  assert.equal(
    stableAdminOptionOrder(provider).map((option) => option.name).join("\n"),
    stableAdminOptionOrder(fallback).map((option) => option.name).join("\n"),
  );
});
