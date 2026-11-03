// MCP 服务器管理（当前阶段：配置存储与视图；stdio 连接能力在后续里程碑接入）。
import type { MCPServerView } from "../../shared/types";
import type { DB } from "../store/db";
import { getSetting, setSetting } from "../store/records";
import { newId, IdPrefix } from "../../shared/ids";

const MCP_SERVERS_KEY = "mcp.servers";

interface PersistedMCPServer {
  id: string;
  name: string;
  command: string;
  args: string[];
  enabled: boolean;
}

export class MCPService {
  constructor(private readonly db: DB) {}

  private load(): PersistedMCPServer[] {
    const value = getSetting<PersistedMCPServer[]>(this.db, MCP_SERVERS_KEY, []);
    return Array.isArray(value) ? value : [];
  }

  private save(list: PersistedMCPServer[]): void {
    setSetting(this.db, MCP_SERVERS_KEY, list);
  }

  list(): MCPServerView[] {
    return this.load().map((s) => ({
      id: s.id,
      name: s.name,
      command: s.command,
      args: s.args,
      enabled: s.enabled,
      connected: false,
      error: s.enabled ? "MCP 连接能力将在后续版本提供" : undefined,
      tools: [],
    }));
  }

  add(name: string, commandLine: string): MCPServerView {
    const trimmedName = name.trim();
    if (!trimmedName) throw new Error("名称不能为空");
    const parts = splitCommandLine(commandLine.trim());
    const server: PersistedMCPServer = {
      id: newId(IdPrefix.McpServer),
      name: trimmedName,
      command: parts[0] ?? "",
      args: parts.slice(1),
      enabled: false,
    };
    const list = this.load();
    list.push(server);
    this.save(list);
    return this.list().find((s) => s.id === server.id)!;
  }

  remove(serverId: string): void {
    this.save(this.load().filter((s) => s.id !== serverId));
  }

  connect(serverId: string): MCPServerView {
    throw new Error("MCP 连接能力将在后续版本提供");
    void serverId;
  }

  disconnect(serverId: string): void {
    void serverId;
  }
}

/** 按空白拆分命令行（支持单/双引号与反斜杠转义）。 */
export function splitCommandLine(line: string): string[] {
  const parts: string[] = [];
  let current = "";
  let quote: '"' | "'" | null = null;
  let hasContent = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i]!;
    if (quote) {
      if (ch === "\\" && line[i + 1]) {
        current += line[i + 1]!;
        i++;
      } else if (ch === quote) {
        quote = null;
      } else {
        current += ch;
      }
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      hasContent = true;
    } else if (ch === "\\" && line[i + 1]) {
      current += line[i + 1]!;
      i++;
      hasContent = true;
    } else if (/\s/.test(ch)) {
      if (current || hasContent) {
        parts.push(current);
        current = "";
        hasContent = false;
      }
    } else {
      current += ch;
      hasContent = true;
    }
  }
  if (quote) throw new Error("命令行引号未闭合");
  if (current || hasContent) parts.push(current);
  if (parts.length === 0) throw new Error("命令不能为空");
  return parts;
}
