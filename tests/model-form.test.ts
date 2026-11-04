// 模型表单共享校验测试（review 修复 #8）：provider-add 与 provider-model-dialog 两份重复
// 校验收敛到 lib/model-form 后的统一行为（对齐服务端 ProviderService.buildModel 规则）。
import { describe, expect, it } from "vitest";
import {
  normalizeContextK,
  normalizeMaxTokens,
  validateContextK,
  validateModelForm,
  validateModelId,
} from "../src/renderer/src/lib/model-form";

describe("模型 ID 校验帮手（对齐服务端 buildModel）", () => {
  it("空 / 纯空白 → 报「模型 ID 不能为空」", () => {
    expect(validateModelId("")).toBe("模型 ID 不能为空");
    expect(validateModelId("   ")).toBe("模型 ID 不能为空");
  });

  it("含空白 → 报「模型 ID 不能包含空白」", () => {
    expect(validateModelId("gpt 4o")).toBe("模型 ID 不能包含空白");
    expect(validateModelId("a\tb")).toBe("模型 ID 不能包含空白");
  });

  it("合法 ID → 无错误", () => {
    expect(validateModelId("deepseek-chat")).toBeUndefined();
  });
});

describe("上下文窗口校验帮手（对齐服务端上限 100000K）", () => {
  it("超上限 → 报错文案含上限值", () => {
    expect(validateContextK(100_001)).toContain("100000");
  });

  it("上限内 / 0 / NaN → 不报错（0 与 NaN 提交时归默认）", () => {
    expect(validateContextK(100_000)).toBeUndefined();
    expect(validateContextK(0)).toBeUndefined();
    expect(validateContextK(Number.NaN)).toBeUndefined();
  });
});

describe("模型表单整体校验（validateModelForm）", () => {
  it("ID 为空且上下文超限 → 两处同时报错", () => {
    const errs = validateModelForm({ id: "", contextK: 200_000 });
    expect(errs.id).toBe("模型 ID 不能为空");
    expect(errs.contextK).toContain("100000");
  });

  it("合法填写 → 无错误", () => {
    expect(validateModelForm({ id: "glm-5", contextK: 128 })).toEqual({});
  });
});

describe("上下文窗口提交值归一（对齐服务端：>0 取整，否则默认 32）", () => {
  it("正数取整", () => {
    expect(normalizeContextK(128)).toBe(128);
    expect(normalizeContextK(32.9)).toBe(32);
  });

  it("非正数 / NaN 归默认 32", () => {
    expect(normalizeContextK(0)).toBe(32);
    expect(normalizeContextK(-5)).toBe(32);
    expect(normalizeContextK(Number.NaN)).toBe(32);
  });
});

describe("最大输出 tokens 提交值归一（对齐服务端：clamp 到 [0, 4_000_000]）", () => {
  it("范围内取整", () => {
    expect(normalizeMaxTokens(4096)).toBe(4096);
    expect(normalizeMaxTokens(4096.9)).toBe(4096);
  });

  it("负数 / NaN 归 0；超大值 clamp 到 4_000_000", () => {
    expect(normalizeMaxTokens(-5)).toBe(0);
    expect(normalizeMaxTokens(Number.NaN)).toBe(0);
    expect(normalizeMaxTokens(9_999_999)).toBe(4_000_000);
  });
});
