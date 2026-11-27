import { describe, it } from "node:test";
import assert from "node:assert/strict";

import {
  DEFAULT_GLOBAL_PROXY_MODE,
  getDefaultNetworkProxySettings,
  validateProxyUrl,
} from "./networkSettingsLogic.ts";
import { fetchChannelModelsWithTimeout } from "./modelChannelLogic.ts";
import {
  getAccountStateLabel,
  filterRenderedAccounts,
  getTaskRunningLabel,
} from "./accountLogic.ts";
import {
  INPLACE_LOGIN_HINT_KEY,
  INPLACE_LOGIN_SUCCESS_CONTINUE_KEY,
} from "./inplaceLoginText.ts";
import { translate } from "../../i18n/index.ts";

const INPLACE_LOGIN_HINT = translate("zh-CN", INPLACE_LOGIN_HINT_KEY);
const INPLACE_LOGIN_SUCCESS_CONTINUE = translate("zh-CN", INPLACE_LOGIN_SUCCESS_CONTINUE_KEY);
import type { AccountId } from "./types.ts";

describe("工作台调试 核心业务行为契约与自检", () => {
  it("网络默认模式必须为直连，且 NISAR S 默认例外覆盖为系统代理", () => {
    assert.equal(DEFAULT_GLOBAL_PROXY_MODE, "direct", "全局网络默认必须为直连 direct");
    const defaults = getDefaultNetworkProxySettings();
    assert.equal(defaults.global_mode, "direct");
    assert.equal(
      defaults.source_overrides["bhoonidhi"],
      "system",
      "NISAR S 波段 (Bhoonidhi) 默认必须为系统代理例外"
    );
  });

  it("代理 URL 客户端校验：拒绝内嵌凭据 user:pass@ 并要求显式端口", () => {
    const credUrlRes = validateProxyUrl("http://admin:secret123@127.0.0.1:7890");
    assert.equal(credUrlRes.ok, false, "内嵌凭据的代理 URL 必须被拒绝");
    assert.match(credUrlRes.error_zh || "", /凭据|密码|安全/, "拒绝提示必须提及凭据/密码/安全原因");

    const noPortRes = validateProxyUrl("http://127.0.0.1");
    assert.equal(noPortRes.ok, false, "手动代理缺少端口必须被拒绝");
    assert.match(noPortRes.error_zh || "", /端口/, "拒绝提示必须提及端口");

    const validRes = validateProxyUrl("http://127.0.0.1:7890");
    assert.equal(validRes.ok, true, "合法的主机与端口必须校验通过");
  });

  it("读取模型列表超时 8 秒后必须安全回退为手动输入模式", async () => {
    // 模拟超过 8 秒未响应的请求
    const hangingFetcher = () =>
      new Promise<string[]>((resolve) => {
        setTimeout(() => resolve(["gpt-4o"]), 20000);
      });

    // 传入自定义 100ms 超时用于快速单元测试
    const result = await fetchChannelModelsWithTimeout(hangingFetcher, 100);
    assert.equal(result.fallbackToManual, true, "超时后必须回退为手动输入");
    assert.equal(result.models.length, 0, "回退后可用模型列表为空或需要手动输入");

    // 正常响应情况
    const quickFetcher = async () => ["model-alpha", "model-beta"];
    const quickResult = await fetchChannelModelsWithTimeout(quickFetcher, 100);
    assert.equal(quickResult.fallbackToManual, false);
    assert.deepEqual(quickResult.models, ["model-alpha", "model-beta"]);
  });

  it("账号状态五态映射必须准确，且界面绝不渲染 GACOS 账户卡片", () => {
    assert.equal(getAccountStateLabel("logged_in"), "账号有效");
    assert.equal(getAccountStateLabel("login_required"), "未验证");
    assert.equal(getAccountStateLabel("credentials_missing"), "未配置");
    assert.equal(getAccountStateLabel("service_unavailable"), "服务不可达");
    assert.equal(getAccountStateLabel("not_required"), "无需认证");

    const allAccounts: AccountId[] = [
      "cdse",
      "earthdata",
      "usgs_m2m",
      "bhoonidhi",
      "mpc",
      "opentopography",
      "gacos",
    ];
    const rendered = filterRenderedAccounts(allAccounts);
    assert.ok(!rendered.includes("gacos"), "账号页界面列表绝对不得包含 gacos");
    assert.ok(rendered.includes("cdse"));
    assert.ok(rendered.includes("mpc"));
  });

  it("就地登录卡文案必须包含「登录成功后自动继续」，且绝不包含禁词", () => {
    assert.ok(
      INPLACE_LOGIN_SUCCESS_CONTINUE.includes("登录成功后自动继续"),
      "就地登录说明必须包含「登录成功后自动继续」"
    );

    const fullText = `${INPLACE_LOGIN_HINT} ${INPLACE_LOGIN_SUCCESS_CONTINUE}`;
    const forbiddenWords = [
      "存储",
      "凭据管理器",
      "netrc",
      "重新点击",
      "加入下载队列",
    ];
    for (const word of forbiddenWords) {
      assert.ok(
        !fullText.includes(word),
        `就地登录文案绝对不能包含禁词「${word}」，当前内容为: ${fullText}`
      );
    }
  });

  it("DEM 转换任务运行态必须显示「转换中」而非「下载中」", () => {
    assert.equal(getTaskRunningLabel("dem_convert"), "转换中");
    assert.equal(getTaskRunningLabel("scene_download"), "下载中");
    assert.equal(getTaskRunningLabel(), "下载中");
  });
});
