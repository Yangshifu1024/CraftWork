// 模型工厂禁用语义测试（票 #7 / spec §9）：调用侧启停收口。
// 只测公共接缝：ModelFactory.client（停用项抛可读 ModelError）与
// ModelFactory.fallbackModel（失败转移跳过停用候选）。
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DB } from "../src/main/store/db";
import { Catalog, saveCustomProviders, type PersistedProvider } from "../src/main/model/catalog";
import { DEFAULT_CAPS } from "../src/main/model/types";
import { ModelFactory, type KeyProvider } from "../src/main/model/factory";
import { Code, ModelError } from "../src/main/model/errors";

function makeProvider(overrides?: Partial<PersistedProvider>): PersistedProvider {
  return {
    id: "prov",
    name: "测试提供商",
    kind: "openai-compatible",
    endpoint: "https://example.invalid/v1",
    keyRequired: false,
    note: "",
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
      {
        id: "model-2",
        name: "备用模型",
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

const keys: KeyProvider = { getFor: () => "sk-test" };

let dir: string;
let db: DB;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "cw-factory-"));
  db = DB.open(join(dir, "test.db"));
});

afterEach(() => {
  db.close();
  rmSync(dir, { recursive: true, force: true });
});

function factoryFor(providers: PersistedProvider[]): ModelFactory {
  saveCustomProviders(db, providers);
  return new ModelFactory(new Catalog(db), keys);
}

describe("client 启停拦截", () => {
  it("模型被禁用时抛 provider_unavailable「该模型已停用，请重新选择模型」", () => {
    const factory = factoryFor([makeProvider({ models: [makeProvider().models[0]!, { ...makeProvider().models[1]!, enabled: false }] })]);
    const err = (() => {
      try {
        factory.client("prov/model-2");
        return null;
      } catch (e) {
        return e;
      }
    })();
    expect(err).toBeInstanceOf(ModelError);
    expect((err as ModelError).code).toBe(Code.ProviderUnavailable);
    expect((err as ModelError).message).toContain("该模型已停用，请重新选择模型");
  });

  it("提供商被禁用时抛 provider_unavailable，文案含提供商名与停用提示", () => {
    const factory = factoryFor([makeProvider({ enabled: false })]);
    const err = (() => {
      try {
        factory.client("prov/model");
        return null;
      } catch (e) {
        return e;
      }
    })();
    expect(err).toBeInstanceOf(ModelError);
    expect((err as ModelError).code).toBe(Code.ProviderUnavailable);
    expect((err as ModelError).message).toContain("测试提供商");
    expect((err as ModelError).message).toContain("该模型已停用，请重新选择模型");
  });

  it("enabled 缺省（旧数据）视为启用，正常构造客户端", () => {
    const legacy = { ...makeProvider() } as unknown as Record<string, unknown>;
    delete legacy["enabled"];
    legacy["models"] = (legacy["models"] as Array<Record<string, unknown>>).map((m) => {
      const copy = { ...m };
      delete copy["enabled"];
      return copy;
    });
    const factory = factoryFor([legacy as unknown as PersistedProvider]);
    expect(() => factory.client("prov/model")).not.toThrow();
    expect(factory.client("prov/model")).toBeTruthy();
  });

  it("启用模型不受同提供商其他禁用模型影响", () => {
    const factory = factoryFor([makeProvider({ models: [makeProvider().models[0]!, { ...makeProvider().models[1]!, enabled: false }] })]);
    expect(() => factory.client("prov/model")).not.toThrow();
  });
});

describe("fallbackModel 跳过停用候选", () => {
  it("候选模型被禁用时返回空串，不转移到停用模型", () => {
    const factory = factoryFor([makeProvider({ models: [makeProvider().models[0]!, { ...makeProvider().models[1]!, enabled: false }] })]);
    factory.setFallbackResolver(() => "prov/model-2");
    expect(factory.fallbackModel("prov/model")).toBe("");
  });

  it("候选提供商被禁用时返回空串", () => {
    const factory = factoryFor([
      makeProvider(),
      makeProvider({ id: "off", name: "停用商", enabled: false }),
    ]);
    factory.setFallbackResolver(() => "off/model");
    expect(factory.fallbackModel("prov/model")).toBe("");
  });

  it("候选启用且与当前不同时返回规范引用（对照）", () => {
    const factory = factoryFor([
      makeProvider(),
      makeProvider({ id: "other", name: "其他商", models: [makeProvider().models[0]!] }),
    ]);
    factory.setFallbackResolver(() => "other/model");
    expect(factory.fallbackModel("prov/model")).toBe("other/model");
  });
});
