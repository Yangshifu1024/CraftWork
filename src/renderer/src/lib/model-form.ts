// 模型表单共享校验：添加提供商（首个模型）与模型增改弹窗两处共用的字段校验与提交值归一。
// 无 React / DOM 依赖，供渲染层表单组件消费并由 vitest 直测（tests/model-form.test.ts）。
// 校验规则与服务端 ProviderService.buildModel 对齐，最终以服务端为准。

/** 上下文窗口服务端上限（buildModel：>100_000 报错）。 */
export const CONTEXT_K_MAX = 100_000;

/** 最大输出 tokens 服务端边界（buildModel clamp 到 [0, 4_000_000]）。 */
export const MAX_TOKENS_LIMIT = 4_000_000;

/** 模型 ID 内联红字文案：空 → 不能为空；含空白 → 不能包含空白；合法 → undefined。 */
export function validateModelId(rawId: string): string | undefined {
  const id = rawId.trim();
  if (id === "") return "模型 ID 不能为空";
  if (/\s/.test(id)) return "模型 ID 不能包含空白";
  return undefined;
}

/** 上下文窗口内联红字文案（NaN 视为输入中间态，不报错；提交时归默认 32）。 */
export function validateContextK(raw: number): string | undefined {
  if (Number.isFinite(raw) && raw > CONTEXT_K_MAX) {
    return `上下文窗口不能超过 ${CONTEXT_K_MAX}K`;
  }
  return undefined;
}

/** 模型增改弹窗字段 → 内联红字（服务端规则的投影，重复 ID 等只能由服务端判定）。 */
export type ModelFormErrors = Partial<Record<"id" | "contextK", string>>;

/** 模型增改弹窗预校验：字段 → 内联红字；无错误字段不出现。 */
export function validateModelForm(input: { id: string; contextK: number }): ModelFormErrors {
  const errs: ModelFormErrors = {};
  const idError = validateModelId(input.id);
  if (idError) errs.id = idError;
  const contextKError = validateContextK(input.contextK);
  if (contextKError) errs.contextK = contextKError;
  return errs;
}

/** 上下文窗口提交值归一（对齐服务端 buildModel：>0 取整，否则默认 32）。 */
export function normalizeContextK(raw: number): number {
  if (Number.isFinite(raw) && raw > 0) return Math.trunc(raw);
  return 32;
}

/** 最大输出 tokens 提交值归一（对齐服务端 buildModel：clamp 到 [0, 4_000_000]）。 */
export function normalizeMaxTokens(raw: number): number {
  if (!Number.isFinite(raw)) return 0;
  return Math.min(MAX_TOKENS_LIMIT, Math.max(0, Math.trunc(raw)));
}
