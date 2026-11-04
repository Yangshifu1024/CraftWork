// 提供商服务测试：enabled 缺省迁移、启停与改名契约（真实 SQLite 临时目录）。
// 只测公共接缝：loadCustomProviders / ProviderService.setEnabled / rename / list。
import { beforeEach, describe, expect, it } from "vitest";
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
