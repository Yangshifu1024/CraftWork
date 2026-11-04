// 提供商列表纯逻辑测试：状态点三色判定、左栏按来源分组、模型选择器过滤（票 #7）。
// 函数位于渲染层 lib（无 React 依赖），供设置页与模型选择器消费。
import { describe, expect, it } from "vitest";
import { groupProviders, providerStatus, selectableModels } from "../src/renderer/src/lib/provider-list";
import type { ModelView, ProviderView } from "../src/shared/types";

function makeModel(overrides?: Partial<ModelView>): ModelView {
  return {
    id: "model",
    ref: "prov/model",
    name: "测试模型",
    provider: "prov",
    providerName: "测试提供商",
    contextK: 8,
    maxTokens: 1024,
    caps: { stream: true, tools: false, vision: false, reasoning: false, structured: false },
    status: "",
    canonicalId: "",
    releaseDate: "",
    source: "manual",
    enabled: true,
    available: true,
    ...overrides,
  };
}

function makeProviderView(overrides?: Partial<ProviderView>): ProviderView {
  return {
    id: "prov",
    name: "测试提供商",
    kind: "openai-compatible",
    endpoint: "https://example.invalid/v1",
    keyRequired: false,
    keySource: "",
    keyMasked: "",
    available: true,
    enabled: true,
    docUrl: "",
    catalogId: "",
    source: "manual",
    models: [makeModel()],
    ...overrides,
  };
}

describe("模型选择器过滤（spec §9）", () => {
  it("只列启用提供商 × 启用模型，分组保留提供商信息", () => {
    const groups = selectableModels([
      makeProviderView({
        models: [makeModel({ id: "a", ref: "prov/a", name: "甲" }), makeModel({ id: "b", ref: "prov/b", name: "乙", enabled: false })],
      }),
    ]);
    expect(groups.length).toBe(1);
    expect(groups[0]?.provider.id).toBe("prov");
    expect(groups[0]?.models.map((m) => m.id)).toEqual(["a"]);
  });

  it("禁用提供商的全部模型被过滤", () => {
    const groups = selectableModels([
      makeProviderView({ id: "on", name: "开", enabled: true }),
      makeProviderView({ id: "off", name: "关", enabled: false }),
    ]);
    expect(groups.map((g) => g.provider.id)).toEqual(["on"]);
  });

  it("全部被禁时返回空列表", () => {
    expect(selectableModels([makeProviderView({ enabled: false })])).toEqual([]);
    expect(selectableModels([])).toEqual([]);
  });

  it("无可用模型的提供商不产生空分组", () => {
    const groups = selectableModels([
      makeProviderView({ id: "has" }),
      makeProviderView({ id: "empty", models: [] }),
      makeProviderView({ id: "allOff", models: [makeModel({ enabled: false })] }),
    ]);
    expect(groups.map((g) => g.provider.id)).toEqual(["has"]);
  });
});

describe("状态点三色判定", () => {
  it("禁用（enabled === false）为灰，优先级最高", () => {
    const r = providerStatus({ enabled: false, keyMasked: "sk-···abcd", keyRequired: true, enabledModelCount: 3 });
    expect(r.tone).toBe("off");
    expect(r.label).toBe("已停用");
  });

  it("启用且密钥就绪且有启用模型为绿", () => {
    const r = providerStatus({ keyMasked: "sk-···abcd", keyRequired: true, enabledModelCount: 3 });
    expect(r.tone).toBe("ok");
    expect(r.label).toBe("可用");
  });

  it("启用但无需密钥且有启用模型为绿", () => {
    const r = providerStatus({ keyMasked: "", keyRequired: false, enabledModelCount: 1 });
    expect(r.tone).toBe("ok");
  });

  it("启用但缺密钥为橙", () => {
    const r = providerStatus({ keyMasked: "", keyRequired: true, enabledModelCount: 3 });
    expect(r.tone).toBe("warn");
    expect(r.label).toBe("未配置密钥");
  });

  it("启用但无启用模型为橙", () => {
    const r = providerStatus({ keyMasked: "sk-···abcd", keyRequired: true, enabledModelCount: 0 });
    expect(r.tone).toBe("warn");
    expect(r.label).toBe("暂无模型");
  });

  it("模型全部停用（enabledModelCount 0）为橙，不因模型总数显绿", () => {
    // 调用点以 models.filter(m => m.enabled !== false).length 传入；模型在但全停用同样无可用
    const r = providerStatus({ keyMasked: "sk-···abcd", keyRequired: false, enabledModelCount: 0 });
    expect(r.tone).toBe("warn");
    expect(r.label).toBe("暂无模型");
  });

  it("enabled 缺省视为启用", () => {
    expect(providerStatus({ keyMasked: "", keyRequired: false, enabledModelCount: 2 }).tone).toBe("ok");
    expect(providerStatus({ keyMasked: "", keyRequired: true, enabledModelCount: 0 }).tone).toBe("warn");
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
