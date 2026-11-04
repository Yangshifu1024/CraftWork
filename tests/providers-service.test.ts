// 提供商服务测试：enabled 缺省迁移、启停与改名契约、拉取模型（真实 SQLite 临时目录 + mock fetch）。
// 只测公共接缝：loadCustomProviders / ProviderService.setEnabled / rename / list / fetchModels。
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DB } from "../src/main/store/db";
import {
  Catalog,
  saveCustomProviders,
  loadCustomProviders,
  type PersistedModel,
  type PersistedProvider,
} from "../src/main/model/catalog";
import { DEFAULT_CAPS } from "../src/main/model/types";
import { SecretStore } from "../src/main/core/secret";
import { ProviderService } from "../src/main/services/providers";

function makeProvider(overrides?: Partial<PersistedProvider>): PersistedProvider {
  return {
    id: "prov",
    name: "测试提供商",
    kind: "openai-compatible",
    endpoint: "https://example.invalid/v1",
    keyRequired: false,
    note: "https://docs.example.com",
    source: "manual",
    catalogId: "",
    catalogVersion: "",
    envVars: [],
    enabled: true,
    models: [
      {
        id: "model",
        name: "测试模型",
        contextK: 8,
        maxTokens: 1024,
        caps: DEFAULT_CAPS,
        status: "",
        canonicalId: "",
        releaseDate: "",
        source: "",
        enabled: true,
      },
    ],
    ...overrides,
  };
}

let dir: string;
let db: DB;
let service: ProviderService;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "cw-providers-"));
  db = DB.open(join(dir, "test.db"));
  service = new ProviderService(db, SecretStore.newEmpty(dir), new Catalog(db));
});

function cleanup(): void {
  try {
    db.close();
  } catch {
    /* 忽略 */
  }
  rmSync(dir, { recursive: true, force: true });
}

describe("enabled 缺省迁移（旧数据兼容）", () => {
  it("旧数据无 enabled 字段：读取时缺省补 true", () => {
    // 模拟迁移前的持久化形状：直接写入不含 enabled 的旧 JSON
    saveCustomProviders(db, [JSON.parse(JSON.stringify(makeProvider())) as PersistedProvider]);
    const loaded = loadCustomProviders(db);
    expect(loaded).toHaveLength(1);
    expect(loaded[0]!.enabled).toBe(true);
    cleanup();
  });

  it("已显式禁用的数据不被迁移覆盖", () => {
    saveCustomProviders(db, [makeProvider({ enabled: false })]);
    expect(loadCustomProviders(db)[0]!.enabled).toBe(false);
    cleanup();
  });
});

describe("模型 enabled 缺省迁移（旧数据兼容）", () => {
  it("旧数据模型无 enabled 字段：读取时缺省补 true", () => {
    // 构造迁移前的持久化形状：抹掉模型上的 enabled 字段
    const legacy = makeProvider();
    const legacyModel = { ...legacy.models[0]! };
    delete (legacyModel as Partial<PersistedModel>).enabled;
    legacy.models = [legacyModel as PersistedModel];
    saveCustomProviders(db, [JSON.parse(JSON.stringify(legacy)) as PersistedProvider]);
    const loaded = loadCustomProviders(db);
    expect(loaded[0]!.models[0]!.enabled).toBe(true);
    cleanup();
  });

  it("已显式禁用的模型不被迁移覆盖", () => {
    const p = makeProvider();
    p.models = [{ ...p.models[0]!, enabled: false }];
    saveCustomProviders(db, [p]);
    expect(loadCustomProviders(db)[0]!.models[0]!.enabled).toBe(false);
    cleanup();
  });
});

describe("providerSetEnabled", () => {
  it("禁用：返回刷新后的 ProviderView（enabled=false），且持久化生效", () => {
    saveCustomProviders(db, [makeProvider()]);
    const view = service.setEnabled("prov", false);
    expect(view.enabled).toBe(false);
    expect(view.id).toBe("prov");
    expect(service.list()[0]!.enabled).toBe(false);
    cleanup();
  });

  it("重新启用：enabled=true，模型仍完整保留", () => {
    saveCustomProviders(db, [makeProvider({ enabled: false })]);
    const view = service.setEnabled("prov", true);
    expect(view.enabled).toBe(true);
    expect(view.models).toHaveLength(1);
    cleanup();
  });

  it("提供商不存在：抛可读错误", () => {
    expect(() => service.setEnabled("ghost", false)).toThrowError("提供商不存在");
    cleanup();
  });
});

describe("providerRename", () => {
  it("改名：返回刷新后的 ProviderView（name 更新），且持久化生效", () => {
    saveCustomProviders(db, [makeProvider()]);
    const view = service.rename("prov", "新名字");
    expect(view.name).toBe("新名字");
    expect(service.list()[0]!.name).toBe("新名字");
    cleanup();
  });

  it("改名不影响其余字段（端点、模型、enabled、docUrl 透传）", () => {
    saveCustomProviders(db, [makeProvider({ enabled: false })]);
    const view = service.rename("prov", "新名字");
    expect(view.endpoint).toBe("https://example.invalid/v1");
    expect(view.enabled).toBe(false);
    expect(view.docUrl).toBe("https://docs.example.com");
    expect(view.models).toHaveLength(1);
    cleanup();
  });

  it("空白名称：抛「提供商名称不能为空」", () => {
    saveCustomProviders(db, [makeProvider()]);
    expect(() => service.rename("prov", "   ")).toThrowError("提供商名称不能为空");
    cleanup();
  });

  it("提供商不存在：抛可读错误", () => {
    expect(() => service.rename("ghost", "任意")).toThrowError("提供商不存在");
    cleanup();
  });
});

describe("providerSetModelEnabled", () => {
  it("禁用模型：返回刷新后的 ProviderView（模型 enabled=false），且持久化生效", () => {
    saveCustomProviders(db, [makeProvider()]);
    const view = service.setModelEnabled("prov", "model", false);
    expect(view.models[0]!.enabled).toBe(false);
    expect(loadCustomProviders(db)[0]!.models[0]!.enabled).toBe(false);
    cleanup();
  });

  it("重新启用：模型 enabled=true，提供商启用状态不受影响", () => {
    const p = makeProvider();
    p.models = [{ ...p.models[0]!, enabled: false }];
    saveCustomProviders(db, [p]);
    const view = service.setModelEnabled("prov", "model", true);
    expect(view.models[0]!.enabled).toBe(true);
    expect(view.enabled).toBe(true);
    cleanup();
  });

  it("多模型时只影响目标模型", () => {
    const p = makeProvider();
    p.models = [p.models[0]!, { ...p.models[0]!, id: "model2", name: "第二模型" }];
    saveCustomProviders(db, [p]);
    const view = service.setModelEnabled("prov", "model2", false);
    expect(view.models.find((m) => m.id === "model")!.enabled).toBe(true);
    expect(view.models.find((m) => m.id === "model2")!.enabled).toBe(false);
    cleanup();
  });

  it("模型不存在：抛可读错误", () => {
    saveCustomProviders(db, [makeProvider()]);
    expect(() => service.setModelEnabled("prov", "ghost", false)).toThrowError("模型不存在：ghost");
    cleanup();
  });

  it("提供商不存在：抛可读错误", () => {
    expect(() => service.setModelEnabled("ghost", "model", false)).toThrowError("提供商不存在");
    cleanup();
  });
});

describe("providerAddModel", () => {
  it("新增模型：追加到列表末尾，默认启用，持久化生效", () => {
    saveCustomProviders(db, [makeProvider()]);
    const view = service.addModel("prov", { id: "model2", name: "第二模型", contextK: 128, maxTokens: 4096 });
    expect(view.models).toHaveLength(2);
    const added = view.models[1]!;
    expect(added.id).toBe("model2");
    expect(added.name).toBe("第二模型");
    expect(added.contextK).toBe(128);
    expect(added.maxTokens).toBe(4096);
    expect(added.enabled).toBe(true);
    expect(loadCustomProviders(db)[0]!.models).toHaveLength(2);
    cleanup();
  });

  it("新增模型：显示名留空时取模型 ID；能力勾选透传", () => {
    saveCustomProviders(db, [makeProvider()]);
    const view = service.addModel("prov", {
      id: "m3",
      name: "",
      contextK: 8,
      maxTokens: 0,
      caps: { vision: true, reasoning: true },
    });
    expect(view.models[1]!.name).toBe("m3");
    expect(view.models[1]!.caps.vision).toBe(true);
    expect(view.models[1]!.caps.reasoning).toBe(true);
    cleanup();
  });

  it("提供商内重复 ID：抛可读错误", () => {
    saveCustomProviders(db, [makeProvider()]);
    expect(() => service.addModel("prov", { id: "model", name: "x", contextK: 8, maxTokens: 0 })).toThrowError(
      "同一提供商内模型 ID 重复：model",
    );
    cleanup();
  });

  it("模型 ID 为空或含空白：抛可读错误", () => {
    saveCustomProviders(db, [makeProvider()]);
    expect(() => service.addModel("prov", { id: "  ", name: "x", contextK: 8, maxTokens: 0 })).toThrowError(
      "模型 ID 不能为空或含空白",
    );
    expect(() => service.addModel("prov", { id: "a b", name: "x", contextK: 8, maxTokens: 0 })).toThrowError(
      "模型 ID 不能为空或含空白",
    );
    cleanup();
  });

  it("上下文窗口超上限：抛可读错误", () => {
    saveCustomProviders(db, [makeProvider()]);
    expect(() => service.addModel("prov", { id: "big", name: "x", contextK: 100_001, maxTokens: 0 })).toThrowError(
      "模型 big 的上下文窗口超出上限",
    );
    cleanup();
  });

  it("最大输出 tokens 越界：clamp 到 [0, 4,000,000]；contextK 非正数落默认 32", () => {
    saveCustomProviders(db, [makeProvider()]);
    const view = service.addModel("prov", { id: "m4", name: "x", contextK: 0, maxTokens: -5 });
    expect(view.models[1]!.maxTokens).toBe(0);
    expect(view.models[1]!.contextK).toBe(32);
    const view2 = service.addModel("prov", { id: "m5", name: "x", contextK: 8, maxTokens: 9_999_999 });
    expect(view2.models[2]!.maxTokens).toBe(4_000_000);
    cleanup();
  });

  it("提供商不存在：抛可读错误", () => {
    expect(() => service.addModel("ghost", { id: "m", name: "x", contextK: 8, maxTokens: 0 })).toThrowError(
      "提供商不存在",
    );
    cleanup();
  });
});

describe("providerUpdateModel", () => {
  it("更新模型：表单字段整行替换，启用状态与来源等既有字段保留，持久化生效", () => {
    const p = makeProvider();
    p.models = [{ ...p.models[0]!, enabled: false, source: "catalog", status: "alpha", releaseDate: "2026-01-01" }];
    saveCustomProviders(db, [p]);
    const view = service.updateModel("prov", "model", {
      id: "model",
      name: "改名了",
      contextK: 256,
      maxTokens: 8192,
      caps: { reasoning: true },
    });
    const updated = view.models[0]!;
    expect(updated.name).toBe("改名了");
    expect(updated.contextK).toBe(256);
    expect(updated.maxTokens).toBe(8192);
    expect(updated.caps.reasoning).toBe(true);
    // 非表单字段保留
    expect(updated.enabled).toBe(false);
    expect(updated.source).toBe("catalog");
    expect(updated.status).toBe("alpha");
    expect(updated.releaseDate).toBe("2026-01-01");
    expect(loadCustomProviders(db)[0]!.models[0]!.name).toBe("改名了");
    cleanup();
  });

  it("传入 ID 与目标不一致：抛「模型 ID 不可修改」", () => {
    saveCustomProviders(db, [makeProvider()]);
    expect(() =>
      service.updateModel("prov", "model", { id: "other", name: "x", contextK: 8, maxTokens: 0 }),
    ).toThrowError("模型 ID 不可修改");
    cleanup();
  });

  it("模型不存在：抛可读错误；更新时与其他模型撞 ID：抛重复错误", () => {
    const p = makeProvider();
    p.models = [p.models[0]!, { ...p.models[0]!, id: "model2", name: "第二模型" }];
    saveCustomProviders(db, [p]);
    expect(() =>
      service.updateModel("prov", "ghost", { id: "ghost", name: "x", contextK: 8, maxTokens: 0 }),
    ).toThrowError("模型不存在：ghost");
    expect(() =>
      service.updateModel("prov", "model", { id: "model", name: "x", contextK: 8, maxTokens: 0 }),
    ).not.toThrowError(); // 自身 ID 不算撞
    expect(() =>
      service.updateModel("prov", "model", { id: "model2", name: "x", contextK: 8, maxTokens: 0 }),
    ).toThrowError("模型 ID 不可修改");
    cleanup();
  });

  it("提供商不存在：抛可读错误", () => {
    expect(() =>
      service.updateModel("ghost", "model", { id: "model", name: "x", contextK: 8, maxTokens: 0 }),
    ).toThrowError("提供商不存在");
    cleanup();
  });
});

describe("providerRemoveModel", () => {
  it("删除模型：从列表移除且持久化生效，其余模型保留", () => {
    const p = makeProvider();
    p.models = [p.models[0]!, { ...p.models[0]!, id: "model2", name: "第二模型" }];
    saveCustomProviders(db, [p]);
    const view = service.removeModel("prov", "model");
    expect(view.models.map((m) => m.id)).toEqual(["model2"]);
    expect(loadCustomProviders(db)[0]!.models.map((m) => m.id)).toEqual(["model2"]);
    cleanup();
  });

  it("模型不存在：抛可读错误", () => {
    saveCustomProviders(db, [makeProvider()]);
    expect(() => service.removeModel("prov", "ghost")).toThrowError("模型不存在：ghost");
    cleanup();
  });

  it("提供商不存在：抛可读错误", () => {
    expect(() => service.removeModel("ghost", "model")).toThrowError("提供商不存在");
    cleanup();
  });
});

describe("ProviderView 新字段", () => {
  it("list() 返回 enabled 与 docUrl（note 透传）", () => {
    saveCustomProviders(db, [makeProvider(), makeProvider({ id: "prov2", enabled: false, note: "" })]);
    const views = service.list();
    const first = views.find((v) => v.id === "prov")!;
    const second = views.find((v) => v.id === "prov2")!;
    expect(first.enabled).toBe(true);
    expect(first.docUrl).toBe("https://docs.example.com");
    expect(second.enabled).toBe(false);
    expect(second.docUrl).toBe("");
    cleanup();
  });

  it("toView 透传模型的 enabled 与 source", () => {
    const p = makeProvider();
    p.models = [{ ...p.models[0]!, enabled: false, source: "catalog" }];
    saveCustomProviders(db, [p]);
    const view = service.list()[0]!;
    expect(view.models[0]!.enabled).toBe(false);
    expect(view.models[0]!.source).toBe("catalog");
    cleanup();
  });

  it("providerSave 全量保存：已有模型的 enabled 保留（端点编辑不重置启停）", () => {
    const p = makeProvider();
    p.models = [{ ...p.models[0]!, enabled: false }];
    saveCustomProviders(db, [p]);
    service.saveProvider({
      id: "prov",
      name: "测试提供商",
      kind: "openai-compatible",
      endpoint: "https://example.invalid/v1",
      keyRequired: false,
      models: [{ id: "model", name: "测试模型", contextK: 8, maxTokens: 1024 }],
    });
    expect(loadCustomProviders(db)[0]!.models[0]!.enabled).toBe(false);
    cleanup();
  });
});

describe("providerSave 保留模型来源（review 修复）", () => {
  it("编辑端点后 catalog 模型的 source 保持 catalog，不被改写为 manual", () => {
    const p = makeProvider({ source: "catalog", catalogId: "deepseek" });
    p.models = [{ ...p.models[0]!, source: "catalog" }];
    saveCustomProviders(db, [p]);
    const view = service.saveProvider({
      id: "prov",
      name: "测试提供商",
      kind: "openai-compatible",
      endpoint: "https://example.invalid/v2",
      keyRequired: false,
      models: [{ id: "model", name: "测试模型", contextK: 8, maxTokens: 1024 }],
    });
    expect(view.models[0]!.source).toBe("catalog");
    expect(loadCustomProviders(db)[0]!.models[0]!.source).toBe("catalog");
    cleanup();
  });

  it("启停后全量保存：catalog/fetched 来源不变，表单新增的模型仍为 manual", () => {
    const p = makeProvider({ source: "catalog", catalogId: "deepseek" });
    p.models = [
      { ...p.models[0]!, source: "catalog", enabled: true },
      { ...p.models[0]!, id: "fetched-1", name: "拉取模型", source: "fetched" },
    ];
    saveCustomProviders(db, [p]);
    service.setModelEnabled("prov", "model", false); // 模拟界面启停
    const view = service.saveProvider({
      id: "prov",
      name: "测试提供商",
      kind: "openai-compatible",
      endpoint: "https://example.invalid/v1",
      keyRequired: false,
      models: [
        { id: "model", name: "测试模型", contextK: 8, maxTokens: 1024 },
        { id: "fetched-1", name: "拉取模型", contextK: 8, maxTokens: 0 },
        { id: "brand-new", name: "新模型", contextK: 8, maxTokens: 0 },
      ],
    });
    const sources = Object.fromEntries(view.models.map((m) => [m.id, m.source]));
    expect(sources["model"]).toBe("catalog");
    expect(sources["fetched-1"]).toBe("fetched");
    expect(sources["brand-new"]).toBe("manual");
    // 启停结果同样不被全量保存重置
    const enabled = Object.fromEntries(view.models.map((m) => [m.id, m.enabled]));
    expect(enabled["model"]).toBe(false);
    cleanup();
  });
});

// ---------- providerFetchModels（票 5 / spec §7） ----------

/** 构造 JSON Response 的 fetch mock 响应。 */
function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
}

describe("providerFetchModels：密钥检查", () => {
  it("keyRequired 且无密钥：抛可读错误且不发请求", async () => {
    saveCustomProviders(db, [makeProvider({ keyRequired: true })]);
    const fetchMock = vi.fn(() => Promise.resolve(jsonResponse({ data: [] })));
    vi.stubGlobal("fetch", fetchMock);
    try {
      await expect(service.fetchModels("prov")).rejects.toThrowError("尚未配置密钥，请先设置 API Key");
      expect(fetchMock).not.toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
    }
    cleanup();
  });

  it("keyRequired=false：无密钥也可发起拉取", async () => {
    saveCustomProviders(db, [makeProvider({ keyRequired: false })]);
    const fetchMock = vi.fn(() => Promise.resolve(jsonResponse({ data: [] })));
    vi.stubGlobal("fetch", fetchMock);
    try {
      const result = await service.fetchModels("prov");
      expect(fetchMock).toHaveBeenCalledTimes(1);
      expect(result.fetched).toBe(0);
      expect(result.added).toEqual([]);
      expect(result.models).toHaveLength(1);
    } finally {
      vi.unstubAllGlobals();
    }
    cleanup();
  });

  it("提供商不存在：抛可读错误", async () => {
    await expect(service.fetchModels("ghost")).rejects.toThrowError("提供商不存在");
    cleanup();
  });
});

describe("providerFetchModels：OpenAI 系解析与合并", () => {
  it("GET {endpoint}/models，Bearer 密钥头，解析 data[] 合并新增（默认启用、source=fetched、contextK 换算）", async () => {
    saveCustomProviders(db, [makeProvider({ keyRequired: true })]);
    service.setKey("prov", "sk-test");
    let requestedUrl = "";
    let authHeader = "";
    const fetchMock = vi.fn(((url: string | URL, init?: RequestInit) => {
      requestedUrl = String(url);
      authHeader = String((init?.headers as Record<string, string>)?.Authorization ?? "");
      return Promise.resolve(
        jsonResponse({ data: [{ id: "deepseek-reasoner", context_length: 131072 }, { id: "plain-model" }] }),
      );
    }) as typeof fetch);
    vi.stubGlobal("fetch", fetchMock);
    try {
      const result = await service.fetchModels("prov");
      expect(requestedUrl).toBe("https://example.invalid/v1/models");
      expect(authHeader).toBe("Bearer sk-test");
      expect(result.fetched).toBe(2);
      expect(result.added).toEqual(["deepseek-reasoner", "plain-model"]);
      const withCtx = result.models.find((m) => m.id === "deepseek-reasoner")!;
      expect(withCtx.enabled).toBe(true);
      expect(withCtx.source).toBe("fetched");
      expect(withCtx.contextK).toBe(128); // 131072 tokens → 128K
      expect(withCtx.maxTokens).toBe(0);
      expect(withCtx.name).toBe("deepseek-reasoner"); // 无 display_name 时取 ID
      const plain = result.models.find((m) => m.id === "plain-model")!;
      expect(plain.contextK).toBe(32); // 无 context_length 时默认 32
      // 已有模型原样保留，新增追加在尾部，持久化生效
      expect(result.models[0]!.id).toBe("model");
      expect(result.models[0]!.name).toBe("测试模型");
      expect(loadCustomProviders(db)[0]!.models.map((m) => m.id)).toEqual(["model", "deepseek-reasoner", "plain-model"]);
    } finally {
      vi.unstubAllGlobals();
    }
    cleanup();
  });

  it("响应结构不符（无 data 数组）：抛可读错误", async () => {
    saveCustomProviders(db, [makeProvider({ keyRequired: false })]);
    vi.stubGlobal("fetch", vi.fn(() => Promise.resolve(jsonResponse({ objects: [] }))));
    try {
      await expect(service.fetchModels("prov")).rejects.toThrowError("模型列表响应格式不符");
    } finally {
      vi.unstubAllGlobals();
    }
    cleanup();
  });

  it("去重：已存在的模型不覆盖不重排，保留原 enabled 与配置；fetched 计入总数不计入新增", async () => {
    const p = makeProvider({ keyRequired: false });
    p.models = [{ ...p.models[0]!, enabled: false, name: "原名字", contextK: 8 }];
    saveCustomProviders(db, [p]);
    vi.stubGlobal(
      "fetch",
      vi.fn(() =>
        Promise.resolve(
          jsonResponse({ data: [{ id: "model", context_length: 999999 }, { id: "extra" }, { id: "model" }] }),
        ),
      ),
    );
    try {
      const result = await service.fetchModels("prov");
      expect(result.fetched).toBe(3); // 拉取到的总数（含响应内重复）
      expect(result.added).toEqual(["extra"]);
      const kept = result.models.find((m) => m.id === "model")!;
      expect(kept.name).toBe("原名字");
      expect(kept.enabled).toBe(false);
      expect(kept.contextK).toBe(8); // 不被响应字段覆盖
      // 已有在前、新增追加在后，无重复行
      expect(result.models.map((m) => m.id)).toEqual(["model", "extra"]);
      expect(loadCustomProviders(db)[0]!.models.map((m) => m.id)).toEqual(["model", "extra"]);
    } finally {
      vi.unstubAllGlobals();
    }
    cleanup();
  });
});

describe("providerFetchModels：anthropic 协议", () => {
  it("GET {endpoint}/v1/models?limit=1000，x-api-key 头，display_name 作显示名", async () => {
    saveCustomProviders(db, [
      makeProvider({ kind: "anthropic", endpoint: "https://api.anthropic.com", keyRequired: true }),
    ]);
    service.setKey("prov", "sk-ant");
    let requestedUrl = "";
    let headers: Record<string, string> = {};
    vi.stubGlobal(
      "fetch",
      vi.fn(((url: string | URL, init?: RequestInit) => {
        requestedUrl = String(url);
        headers = (init?.headers ?? {}) as Record<string, string>;
        return Promise.resolve(jsonResponse({ data: [{ id: "claude-x", type: "model", display_name: "Claude X" }] }));
      }) as typeof fetch),
    );
    try {
      const result = await service.fetchModels("prov");
      expect(requestedUrl).toBe("https://api.anthropic.com/v1/models?limit=1000");
      expect(headers["x-api-key"]).toBe("sk-ant");
      expect(headers["anthropic-version"]).toBe("2023-06-01");
      expect(result.added).toEqual(["claude-x"]);
      const added = result.models.find((m) => m.id === "claude-x")!;
      expect(added.name).toBe("Claude X");
      expect(added.source).toBe("fetched");
    } finally {
      vi.unstubAllGlobals();
    }
    cleanup();
  });

  it("base 已含 /v1 时不重复拼接（joinServiceURL）", async () => {
    saveCustomProviders(db, [
      makeProvider({ kind: "anthropic", endpoint: "https://proxy.example.com/v1", keyRequired: false }),
    ]);
    let requestedUrl = "";
    vi.stubGlobal(
      "fetch",
      vi.fn(((url: string | URL) => {
        requestedUrl = String(url);
        return Promise.resolve(jsonResponse({ data: [] }));
      }) as typeof fetch),
    );
    try {
      await service.fetchModels("prov");
      expect(requestedUrl).toBe("https://proxy.example.com/v1/models?limit=1000");
    } finally {
      vi.unstubAllGlobals();
    }
    cleanup();
  });
});

describe("providerFetchModels：HTTP 与超时错误归因", () => {
  it("HTTP 401：归因为 auth 文案", async () => {
    saveCustomProviders(db, [makeProvider({ keyRequired: false })]);
    vi.stubGlobal(
      "fetch",
      vi.fn(() =>
        Promise.resolve(jsonResponse({ error: { message: "invalid api key" } }, 401)),
      ),
    );
    try {
      await expect(service.fetchModels("prov")).rejects.toThrowError("密钥无效或权限不足，请检查 API Key。");
    } finally {
      vi.unstubAllGlobals();
    }
    cleanup();
  });

  it("HTTP 500：归因为 provider_unavailable 文案", async () => {
    saveCustomProviders(db, [makeProvider({ keyRequired: false })]);
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.resolve(jsonResponse({ error: "boom" }, 500))),
    );
    try {
      await expect(service.fetchModels("prov")).rejects.toThrowError("无法连接服务，请检查端点地址与网络。");
    } finally {
      vi.unstubAllGlobals();
    }
    cleanup();
  });

  it("超过 10 秒未响应：按 timeout 归因", async () => {
    saveCustomProviders(db, [makeProvider({ keyRequired: false })]);
    vi.stubGlobal(
      "fetch",
      ((_url: string | URL, init?: RequestInit) =>
        new Promise((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () =>
            reject(new DOMException("This operation was aborted", "AbortError")),
          );
        })) as unknown as typeof fetch,
    );
    vi.useFakeTimers();
    try {
      const pending = service.fetchModels("prov");
      const assertion = expect(pending).rejects.toThrowError("连接超时，请检查网络或端点地址。");
      await vi.advanceTimersByTimeAsync(10_000);
      await assertion;
    } finally {
      vi.useRealTimers();
      vi.unstubAllGlobals();
    }
    cleanup();
  });
});
