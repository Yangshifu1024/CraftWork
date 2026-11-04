// 添加提供商页纯逻辑测试：目录行「需填写端点」判定与手动创建表单内联校验。
// 函数位于渲染层 lib（无 React 依赖），供添加页组件消费（票 6 / spec §3.2 §7）。
import { describe, expect, it } from "vitest";
import { manualModelContextK, needsEndpoint, validateManualForm } from "../src/renderer/src/lib/provider-add";

describe("目录提供商「需填写端点」判定", () => {
  it("端点为空 → 需填写（true）", () => {
    expect(needsEndpoint({ endpoint: "" })).toBe(true);
  });

  it("端点只有空白 → 需填写（true）", () => {
    expect(needsEndpoint({ endpoint: "   " })).toBe(true);
  });

  it("端点非空 → 不需要（false）", () => {
    expect(needsEndpoint({ endpoint: "https://api.deepseek.com/v1" })).toBe(false);
  });
});

describe("手动创建表单内联校验", () => {
  it("全部必填项为空 → 名称 / 端点 / 模型 ID 三处报错", () => {
    const errs = validateManualForm({ name: "", endpoint: "", modelId: "", contextK: Number.NaN });
    expect(errs.name).toBe("提供商名称不能为空");
    expect(errs.endpoint).toBe("端点不能为空");
    expect(errs.modelId).toBe("模型 ID 不能为空");
    expect(errs.contextK).toBeUndefined();
  });

  it("合法填写 → 无错误", () => {
    const errs = validateManualForm({ name: "本地网关", endpoint: "http://127.0.0.1:11434/v1", modelId: "llama3", contextK: 128 });
    expect(errs).toEqual({});
  });

  it("模型 ID 含空白 → 报错（对齐服务端 buildModel 规则）", () => {
    const errs = validateManualForm({ name: "x", endpoint: "https://a.com/v1", modelId: "gpt 4o", contextK: 32 });
    expect(errs.modelId).toBe("模型 ID 不能包含空白");
  });

  it("端点非合法 URL → 报错", () => {
    const errs = validateManualForm({ name: "x", endpoint: "not-a-url", modelId: "m", contextK: 32 });
    expect(errs.endpoint).toBe("端点不是合法的 URL");
  });

  it("端点非 http(s) 协议 → 报错", () => {
    const errs = validateManualForm({ name: "x", endpoint: "ftp://a.com/v1", modelId: "m", contextK: 32 });
    expect(errs.endpoint).toBe("端点不是合法的 URL");
  });

  it("上下文窗口超上限（>100000）→ 报错（对齐服务端 buildModel 上限）", () => {
    const errs = validateManualForm({ name: "x", endpoint: "https://a.com/v1", modelId: "m", contextK: 100_001 });
    expect(errs.contextK).toContain("100000");
  });

  it("上下文窗口为 0 / NaN → 不报错（提交时归默认 32）", () => {
    expect(validateManualForm({ name: "x", endpoint: "https://a.com/v1", modelId: "m", contextK: 0 }).contextK).toBeUndefined();
    expect(validateManualForm({ name: "x", endpoint: "https://a.com/v1", modelId: "m", contextK: Number.NaN }).contextK).toBeUndefined();
  });
});

describe("上下文窗口提交值归一", () => {
  it("正数取整", () => {
    expect(manualModelContextK(128)).toBe(128);
    expect(manualModelContextK(32.9)).toBe(32);
  });

  it("非正数 / NaN 归默认 32", () => {
    expect(manualModelContextK(0)).toBe(32);
    expect(manualModelContextK(-5)).toBe(32);
    expect(manualModelContextK(Number.NaN)).toBe(32);
  });
});
