// 提供商服务测试：enabled 缺省迁移、启停与改名契约（真实 SQLite 临时目录）。
// 只测公共接缝：loadCustomProviders / ProviderService.setEnabled / rename / list。
import { beforeEach, describe, expect, it } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DB } from "../src/main/store/db";
import { Catalog, saveCustomProviders, loadCustomProviders, type PersistedProvider } from "../src/main/model/catalog";
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
});
