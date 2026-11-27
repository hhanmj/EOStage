import type { ModelProtocol } from "./types.ts";

export const MODEL_PROTOCOLS: { value: ModelProtocol; label_zh: string }[] = [
  { value: "openai_chat_completions", label_zh: "OpenAI Chat Completions" },
  { value: "openai_responses", label_zh: "OpenAI Responses" },
  { value: "anthropic_messages", label_zh: "Anthropic Messages" },
];

export interface FetchModelsResult {
  models: string[];
  fallbackToManual: boolean;
  message_zh?: string;
}

export async function fetchChannelModelsWithTimeout(
  fetcher: () => Promise<string[]>,
  timeoutMs: number = 8000
): Promise<FetchModelsResult> {
  let timer: any;
  const timeoutPromise = new Promise<FetchModelsResult>((resolve) => {
    timer = setTimeout(() => {
      resolve({
        models: [],
        fallbackToManual: true,
        message_zh: "读取模型列表超时（已等待 8 秒），已切换为手动输入模型标识",
      });
    }, timeoutMs);
  });

  const executionPromise = fetcher()
    .then((models) => {
      clearTimeout(timer);
      return {
        models,
        fallbackToManual: false,
      };
    })
    .catch((err) => {
      clearTimeout(timer);
      return {
        models: [],
        fallbackToManual: true,
        message_zh: `读取模型列表失败: ${err?.message || "网络异常"}，请手动输入模型标识`,
      };
    });

  return Promise.race([executionPromise, timeoutPromise]);
}
