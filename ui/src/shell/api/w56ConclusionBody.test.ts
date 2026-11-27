import test from "node:test";
import assert from "node:assert/strict";
import * as conclusionBodyLogic from "./conclusionBodyLogic.ts";
import { splitConclusionJsonBlock } from "./conclusionBodyLogic.ts";

test("W5-6 R5 (A): splitConclusionJsonBlock 纯逻辑测试", async (t) => {
  await t.test("0. 模块仅导出 splitConclusionJsonBlock", () => {
    assert.deepStrictEqual(
      Object.keys(conclusionBodyLogic),
      ["splitConclusionJsonBlock"],
      "conclusionBodyLogic.ts 必须仅导出 splitConclusionJsonBlock"
    );
  });

  await t.test("1. 空串与无块文本原样返回，jsonBlocks 为空数组", () => {
    assert.deepStrictEqual(splitConclusionJsonBlock(""), {
      body: "",
      jsonBlocks: [],
    });
    const plain = "已完成 Sentinel-1 检索，共匹配 4 景数据。";
    assert.deepStrictEqual(splitConclusionJsonBlock(plain), {
      body: plain,
      jsonBlocks: [],
    });
  });

  await t.test("2. 单块尾随（L4 C4 真机实例形状）：拆出正文与完整围栏块", () => {
    const rawBlock = '```json\n{"text":"已完成检索","claims":[]}\n```';
    const input = `已完成检索，共找到 2 景产品。\n\n${rawBlock}`;
    const res = splitConclusionJsonBlock(input);
    assert.strictEqual(res.body, "已完成检索，共找到 2 景产品。");
    assert.deepStrictEqual(res.jsonBlocks, [rawBlock]);
  });

  await t.test("3. 块在中间：去掉 json 块后前后文本按原顺序拼接并 trim", () => {
    const block = '```json\n{"step":1}\n```';
    const input = `第一段结论\n${block}\n第二段结论`;
    const res = splitConclusionJsonBlock(input);
    assert.ok(res.body.includes("第一段结论"));
    assert.ok(res.body.includes("第二段结论"));
    assert.ok(!res.body.includes("```json"));
    assert.deepStrictEqual(res.jsonBlocks, [block]);
  });

  await t.test("4. 多块与大小写不敏感（```JSON / 带首尾空白）：按出现顺序提取", () => {
    const b1 = '```JSON\n{"a":1}\n```';
    const b2 = '```Json\n{"b":2,"claims":[]}\n```';
    const input = `结论摘要\n  ${b1}  \n补充说明\n${b2}`;
    const res = splitConclusionJsonBlock(input);
    assert.ok(res.body.includes("结论摘要"));
    assert.ok(res.body.includes("补充说明"));
    assert.ok(!res.body.toLowerCase().includes("```json"));
    assert.strictEqual(res.jsonBlocks.length, 2);
    assert.ok(res.jsonBlocks[0].includes('{"a":1}'));
    assert.ok(res.jsonBlocks[1].includes('{"b":2,"claims":[]}'));
  });

  await t.test("5. 无 json info string 的普通 ``` 围栏块留在 body，不计入 jsonBlocks", () => {
    const plainFence = "文本说明\n```\nplain text block\n```";
    const res = splitConclusionJsonBlock(plainFence);
    assert.strictEqual(res.body, plainFence);
    assert.deepStrictEqual(res.jsonBlocks, []);
  });
});
