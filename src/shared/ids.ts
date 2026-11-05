// ID 生成：「前缀_」+ 16 位十六进制（8 字节随机）。
// 随机源失败时返回全 0 后缀，靠数据库唯一约束暴露问题而不是静默重试。
const HEX = "0123456789abcdef";

/** 生成「前缀_16位hex」ID（与 Rust craft_core::ids 同构）。 */
export function newId(prefix: string): string {
  try {
    const bytes = new Uint8Array(8);
    globalThis.crypto.getRandomValues(bytes);
    let hex = "";
    for (const byte of bytes) {
      hex += HEX[byte >> 4];
      hex += HEX[byte & 0x0f];
    }
    return `${prefix}_${hex}`;
  } catch {
    return `${prefix}_0000000000000000`;
  }
}

/** ID 前缀约定 */
export const IdPrefix = {
  Thread: "th",
  Workspace: "ws",
  WorkspaceFolder: "wfo",
  Run: "run",
  Message: "msg",
  Part: "part",
  Approval: "ap",
  ToolCall: "call",
  Attachment: "att",
  Backup: "bk",
  McpServer: "mcp",
  CustomProvider: "custom",
} as const;
