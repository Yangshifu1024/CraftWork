// 提供商列表纯逻辑测试：状态点三色判定与左栏按来源分组。
// 函数位于渲染层 lib（无 React 依赖），供设置页左栏消费。
import { describe, expect, it } from "vitest";
import { groupProviders, providerStatus } from "../src/renderer/src/lib/provider-list";

describe("状态点三色判定", () => {
  it("禁用（enabled === false）为灰，优先级最高", () => {
    const r = providerStatus({ enabled: false, keyMasked: "sk-···abcd", keyRequired: true, modelCount: 3 });
    expect(r.tone).toBe("off");
    expect(r.label).toBe("已停用");
  });

  it("启用且密钥就绪且有模型为绿", () => {
    const r = providerStatus({ keyMasked: "sk-···abcd", keyRequired: true, modelCount: 3 });
    expect(r.tone).toBe("ok");
    expect(r.label).toBe("可用");
  });

  it("启用但无需密钥且有模型为绿", () => {
    const r = providerStatus({ keyMasked: "", keyRequired: false, modelCount: 1 });
    expect(r.tone).toBe("ok");
  });

  it("启用但缺密钥为橙", () => {
    const r = providerStatus({ keyMasked: "", keyRequired: true, modelCount: 3 });
    expect(r.tone).toBe("warn");
    expect(r.label).toBe("未配置密钥");
  });

  it("启用但无模型为橙", () => {
    const r = providerStatus({ keyMasked: "sk-···abcd", keyRequired: true, modelCount: 0 });
    expect(r.tone).toBe("warn");
    expect(r.label).toBe("暂无模型");
  });

  it("enabled 缺省视为启用", () => {
    expect(providerStatus({ keyMasked: "", keyRequired: false, modelCount: 2 }).tone).toBe("ok");
    expect(providerStatus({ keyMasked: "", keyRequired: true, modelCount: 0 }).tone).toBe("warn");
  });
});

describe("左栏按来源分组", () => {
  it("catalog 与 manual 各归其组，目录组在前", () => {
    const groups = groupProviders([
      { id: "a", source: "manual" },
      { id: "b", source: "catalog" },
      { id: "c", source: "catalog" },
    ]);
    expect(groups.map((g) => g.source)).toEqual(["catalog", "manual"]);
    expect(groups[0]?.label).toBe("目录导入");
    expect(groups[0]?.providers.map((p) => p.id)).toEqual(["b", "c"]);
    expect(groups[1]?.label).toBe("手动添加");
    expect(groups[1]?.providers.map((p) => p.id)).toEqual(["a"]);
  });

  it("空组隐藏", () => {
    const onlyManual = groupProviders([{ id: "a", source: "manual" }]);
    expect(onlyManual.length).toBe(1);
    expect(onlyManual[0]?.source).toBe("manual");
    expect(groupProviders([])).toEqual([]);
  });

  it("未知或缺失 source 归入手动添加组（与服务端缺省一致）", () => {
    const groups = groupProviders([
      { id: "a", source: "unknown" },
      { id: "b", source: undefined },
    ]);
    expect(groups.length).toBe(1);
    expect(groups[0]?.source).toBe("manual");
    expect(groups[0]?.providers.map((p) => p.id)).toEqual(["a", "b"]);
  });
});
