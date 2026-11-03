// 标题清洗与附件加载测试。
import { describe, expect, it } from "vitest";
import { normalizeTitle, shortTitle, sanitizeModelTitle, deriveTitle } from "../src/main/agent/title";
import { isSensitiveAttachment } from "../src/main/policy/attachment";

describe("标题清洗", () => {
  it("短标题直用", () => {
    expect(shortTitle("修复登录页面的报错")).toBe("修复登录页面的报错");
  });

  it("剥前缀装饰", () => {
    expect(normalizeTitle("# - 标题：内容")).toBe("标题：内容");
    expect(normalizeTitle("「你好，世界」")).toBe("你好，世界");
  });

  it("模型输出清洗：剥「标题：」前缀", () => {
    expect(sanitizeModelTitle("标题：修复登录")).toBe("修复登录");
    expect(sanitizeModelTitle("Title: Fix login")).toBe("Fix login");
  });

  it("超长截断加省略号", () => {
    const long = "这是一个特别特别特别长的标题已经超过了二十个字的限制需要截断";
    const result = sanitizeModelTitle(long);
    expect([...result].length).toBeLessThanOrEqual(21); // 20 + 省略号
    expect(result.endsWith("…")).toBe(true);
  });

  it("兜底：纯附件用文件名", () => {
    expect(deriveTitle("", ["/tmp/报表.xlsx"])).toBe("报表.xlsx");
  });

  it("敏感名仍被策略层拦截（文件名单独判定）", () => {
    expect(isSensitiveAttachment(".env.production")).toBe(true);
  });
});
