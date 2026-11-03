// 密钥存储：~/.craftwork/keys.json（0600）。
// 密钥只存在于主进程与该文件；
// 损坏时降级启动（新空存储）并给出警告，由用户在设置页重填。
import { chmodSync, existsSync, mkdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { writeFileAtomic } from "./platform";

interface KeysFile {
  version: 1;
  providers: Record<string, string>;
}

export class SecretStore {
  private data: KeysFile = { version: 1, providers: {} };
  private warningList: string[] = [];

  private constructor(private readonly filePath: string) {}

  static open(dir: string): SecretStore {
    const filePath = join(dir, "keys.json");
    const store = new SecretStore(filePath);
    try {
      mkdirSync(dir, { recursive: true, mode: 0o700 });
    } catch {
      // 目录已存在
    }
    if (!existsSync(filePath)) return store;
    try {
      // 权限过宽时自动收紧
      const st = statSync(filePath);
      if ((st.mode & 0o077) !== 0) {
        try {
          chmodSync(filePath, 0o600);
        } catch {
          store.warningList.push("keys.json 权限过宽，自动收紧 0600 失败。");
        }
      }
      const parsed = JSON.parse(readFileSync(filePath, "utf8")) as Partial<KeysFile>;
      if (parsed?.providers && typeof parsed.providers === "object") {
        store.data = { version: 1, providers: { ...parsed.providers } };
      } else {
        store.warningList.push("keys.json 格式异常，已按空密钥库启动；请重新配置密钥。");
      }
    } catch {
      store.warningList.push("keys.json 读取失败（可能已损坏），已按空密钥库启动；请重新配置密钥。");
    }
    return store;
  }

  /** 损坏降级：空存储（之后 set 会覆盖原文件）。 */
  static newEmpty(dir: string): SecretStore {
    const store = new SecretStore(join(dir, "keys.json"));
    try {
      mkdirSync(dir, { recursive: true, mode: 0o700 });
    } catch {
      // 目录已存在
    }
    return store;
  }

  warnings(): string[] {
    return [...this.warningList];
  }

  get(providerId: string): string {
    return this.data.providers[providerId] ?? "";
  }

  /** 取密钥：文件值优先（空串视为未配置），再按 envNames 逐个回退环境变量。 */
  getFor(providerId: string, envNames: string[]): string {
    const file = this.get(providerId);
    if (file) return file;
    for (const name of envNames) {
      const v = process.env[name];
      if (v) return v;
    }
    return "";
  }

  source(providerId: string): string {
    return this.get(providerId) ? "file" : "";
  }

  sourceFor(providerId: string, envNames: string[]): string {
    if (this.get(providerId)) return "file";
    for (const name of envNames) {
      if (process.env[name]) return "env";
    }
    return "";
  }

  /** 遮罩展示：≤10 位全点；否则前 5 位…后 4 位。 */
  maskedFor(providerId: string, envNames: string[]): string {
    const key = this.getFor(providerId, envNames);
    if (!key) return "";
    if (key.length <= 10) return "••••••";
    return `${key.slice(0, 5)}…${key.slice(-4)}`;
  }

  set(providerId: string, value: string): void {
    if (value) this.data.providers[providerId] = value;
    else delete this.data.providers[providerId];
    this.persist();
  }

  delete(providerId: string): void {
    delete this.data.providers[providerId];
    this.persist();
  }

  private persist(): void {
    // 同目录临时文件 → 0600 → 原子替换
    writeFileAtomic(this.filePath, `${JSON.stringify(this.data, null, 2)}\n`, 0o600);
  }
}
