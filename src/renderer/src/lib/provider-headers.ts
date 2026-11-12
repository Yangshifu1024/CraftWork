// 自定义请求头编辑纯逻辑：表单行整理与客户端预校验。
// 无 React / DOM 依赖，供设置页请求头编辑器消费；规则与服务端
// craft_model::types::validate_headers 对齐（最终以服务端校验为准，此处为即时反馈）。
import type { HeaderPair } from "../../../shared/types";

/** 头数量上限（与服务端 MAX_CUSTOM_HEADERS 一致）。 */
export const MAX_CUSTOM_HEADERS = 16;

/** 单个头值的字节上限（与服务端 MAX_HEADER_VALUE_BYTES 一致）。 */
export const MAX_HEADER_VALUE_BYTES = 4096;

/** 头名称合法字符（RFC 7230 token：字母数字与 !#$%&'*+-.^_`|~）。 */
function isHeaderNameChar(c: string): boolean {
  return /^[a-zA-Z0-9!#$%&'*+.^_`|~-]$/.test(c);
}

/** 表单行 → 待保存列表：剔除名称与值皆空的行，其余 trim（保存时以 JSON 提交）。 */
export function compactHeaderRows(rows: HeaderPair[]): HeaderPair[] {
  return rows
    .map((h) => ({ name: h.name.trim(), value: h.value.trim() }))
    .filter((h) => h.name !== "" || h.value !== "");
}

/** 客户端预校验：返回错误文案；空串 = 通过。规则与服务端 validate_headers 对齐。 */
export function validateHeaderPairs(headers: HeaderPair[]): string {
  if (headers.length > MAX_CUSTOM_HEADERS) return `自定义请求头最多 ${MAX_CUSTOM_HEADERS} 条`;
  const seen = new Set<string>();
  for (const h of headers) {
    const name = h.name.trim();
    const value = h.value.trim();
    if (name === "" && value === "") continue;
    if (name === "") return "请求头名称不能为空";
    for (const c of name) {
      if (!isHeaderNameChar(c)) return `请求头名称含非法字符：${h.name}`;
    }
    if (name.toLowerCase() === "authorization" || name.toLowerCase() === "x-api-key") {
      return `请求头 ${name} 与密钥字段冲突，请使用密钥设置`;
    }
    if (/[\r\n]/.test(value)) return `请求头 ${name} 的值不能包含换行`;
    if (seen.has(name.toLowerCase())) return `请求头 ${name} 重复`;
    seen.add(name.toLowerCase());
    if (value !== "" && new TextEncoder().encode(value).length > MAX_HEADER_VALUE_BYTES) {
      return `请求头 ${name} 的值过长（上限 4KB）`;
    }
  }
  return "";
}
