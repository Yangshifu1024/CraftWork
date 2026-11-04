// 生成 golden 样本库（Node/Electron 侧事实源）：Rust 侧兼容测试读取并断言行为一致。
// 用法：node scripts/golden-db.mjs → 写出 tests/fixtures/craftwork-golden.db
// 仅 import src/main/store/db.ts（迁移链事实源），数据行用 node:sqlite 手工插入，
// 避免 events→view→model 依赖链触发类型剥离限制。
import { mkdirSync, rmSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { DB } from "../src/main/store/db.ts";

const outDir = join(process.cwd(), "tests", "fixtures");
const outPath = join(outDir, "craftwork-golden.db");
mkdirSync(outDir, { recursive: true });
for (const suffix of ["", "-wal", "-shm"]) rmSync(outPath + suffix, { force: true });

const tmp = join(tmpdir(), `cw-golden-${Date.now()}`);
mkdirSync(tmp, { recursive: true });
const dbPath = join(tmp, "craftwork.db");
const db = DB.open(dbPath);

const now = Date.now();
const exec = (sql, params = []) => db.prepare(sql).run(...params);

// 工作区：一个项目目录（trust=trusted）+ 主文件夹行（V6 迁移已从 workspaces.path 生成）
const wsId = "ws_golden00000001";
exec(
  "INSERT INTO workspaces (id, name, path, trust, purpose, created_at, last_opened_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
  [wsId, "示例项目", "/tmp/craftwork-demo", "trusted", "project", now, now],
);
const folders = db.prepare("SELECT * FROM workspace_folders WHERE workspace_id = ?").all(wsId);
if (folders.length === 0) {
  exec(
    "INSERT INTO workspace_folders (id, workspace_id, path, position, created_at) VALUES (?, ?, ?, 0, ?)",
    ["wfo_golden00000001", wsId, "/tmp/craftwork-demo", now],
  );
}

// 会话 1：完整对话（用户消息 → 文本回复 → 工具调用 → 完成 run）
exec(
  "INSERT INTO threads (id, mode, title, title_auto, model_id, workspace_id, status, archived, tokens, created_at, updated_at) VALUES (?, 'code', ?, 0, 'openai/gpt-4o', ?, 'idle', 0, 0, ?, ?)",
  ["th_gold1", "示例会话", wsId, now, now],
);
let seq = 0;
const addEvent = (threadId, messageId, runId, kind, payload) => {
  seq += 1;
  exec(
    "INSERT INTO events (id, thread_id, message_id, run_id, seq, kind, payload, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    [`part_golden${String(seq).padStart(10, "0")}`, threadId, messageId, runId, seq, kind, JSON.stringify(payload), now],
  );
  exec("UPDATE threads SET updated_at = ? WHERE id = ?", [now, threadId]);
};
addEvent("th_gold1", "msg_g1", null, "user_message", { text: "帮我看看这个仓库", attachments: [] });
addEvent("th_gold1", "msg_g2", null, "text", { partId: "part_g1", text: "好的，我来分析。" });
addEvent("th_gold1", "msg_g2", null, "tool_call", {
  partId: "part_g2", toolCallId: "call_g1", name: "fs_read", args: { path: "README.md" },
  risk: "R0", status: "done", output: "# Demo", summary: "读取 README", title: "读取文件",
});
exec(
  "INSERT INTO runs (id, thread_id, message_id, status, model_id, started_at, finished_at) VALUES (?, ?, ?, 'done', 'openai/gpt-4o', ?, ?)",
  ["run_golden000001", "th_gold1", "msg_g2", now, now + 1200],
);

// 会话 2：手动改名 + 已决定审批
exec(
  "INSERT INTO threads (id, mode, title, title_auto, model_id, workspace_id, status, archived, tokens, created_at, updated_at) VALUES (?, 'code', ?, 0, '', ?, 'idle', 0, 0, ?, ?)",
  ["th_gold2", "被手动改名的会话", wsId, now, now],
);
addEvent("th_gold2", "msg_g3", null, "user_message", { text: "写一个文件", attachments: [] });
exec(
  "INSERT INTO approvals (id, thread_id, run_id, tool_call_id, tool, risk, decision, scope, requested_at, decided_at) VALUES (?, ?, ?, ?, ?, 'R1', 'allow', 'session', ?, ?)",
  ["ap_golden0000001", "th_gold2", "run_golden000002", "call_x", "fs_write", now, now],
);

// 用量：给会话 1 一条 usage（上下文占用推导用）
exec(
  "INSERT INTO usage (ts, thread_id, model_id, input_tokens, output_tokens, cache_tokens, reasoning_tokens, cost_micros, duration_ms) VALUES (?, ?, ?, 120, 45, 30, 0, 900, 1200)",
  [now, "th_gold1", "openai/gpt-4o"],
);

// 设置：主题
exec("INSERT INTO settings (key, value) VALUES ('app', ?)", [JSON.stringify({ theme: "dark" })]);

db.close();

// WAL 合并后拷贝成品
{
  const probe = new DatabaseSync(dbPath);
  probe.exec("PRAGMA wal_checkpoint(TRUNCATE)");
  probe.close();
}
copyFileSync(dbPath, outPath);

// Node 侧预期基准输出（人工核对 + Rust 测试对照）
{
  const db2 = new DatabaseSync(outPath);
  const rows = db2
    .prepare("SELECT message_id, kind, seq, payload FROM events WHERE thread_id = 'th_gold1' ORDER BY seq")
    .all();
  console.log(JSON.stringify({ events: rows.map((r) => ({ messageId: r.message_id, kind: r.kind, seq: r.seq })) }));
  console.log("user_version:", db2.prepare("PRAGMA user_version").get().user_version);
  db2.close();
}
console.log(`golden 已生成：${outPath}`);
