// 添加提供商页纯逻辑：目录行「需填写端点」判定、手动创建表单内联校验与上下文归一。
// 无 React / DOM 依赖，供添加页组件消费并由 vitest 直测（tests/provider-add.test.ts）。
// 校验规则与服务端 ProviderService.buildProvider / buildModel 对齐，最终以服务端为准。

/** 目录提供商「需填写端点」判定（spec §3.2）：端点为空（trim 后）→ 行上显示 warning 徽标。 */
export function needsEndpoint(p: { endpoint: string }): boolean {
  return p.endpoint.trim() === "";
}

/** 手动创建表单校验入参（contextK 允许 NaN：输入框清空时的中间态，不视为错误）。 */
export interface ManualFormInput {
  name: string;
  endpoint: string;
  modelId: string;
  contextK: number;
}

/** 字段 → 内联红字文案；无错误字段不出现。 */
export type ManualFormErrors = Partial<Record<"name" | "endpoint" | "modelId" | "contextK", string>>;

/** 上下文窗口服务端上限（buildModel：>100_000 报错）。 */
const CONTEXT_K_MAX = 100_000;

/** 手动创建表单必填校验（服务端规则的投影；服务端错误最终同样内联展示）。 */
export function validateManualForm(input: ManualFormInput): ManualFormErrors {
  const errs: ManualFormErrors = {};
  if (input.name.trim() === "") errs.name = "提供商名称不能为空";
  if (input.endpoint.trim() === "") {
    errs.endpoint = "端点不能为空";
  } else if (!isHttpUrl(input.endpoint.trim())) {
    errs.endpoint = "端点不是合法的 URL";
  }
  const modelId = input.modelId.trim();
  if (modelId === "") errs.modelId = "模型 ID 不能为空";
  else if (/\s/.test(modelId)) errs.modelId = "模型 ID 不能包含空白";
  if (Number.isFinite(input.contextK) && input.contextK > CONTEXT_K_MAX) {
    errs.contextK = `上下文窗口不能超过 ${CONTEXT_K_MAX}K`;
  }
  return errs;
}

/** 上下文窗口提交值归一（对齐服务端：>0 取整，否则默认 32）。 */
export function manualModelContextK(raw: number): number {
  if (Number.isFinite(raw) && raw > 0) return Math.trunc(raw);
  return 32;
}

function isHttpUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:";
  } catch {
    return false;
  }
}
