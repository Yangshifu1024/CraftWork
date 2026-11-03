// 数据面板：审计记录 / 用量统计 / 备份恢复 三分区。
import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { api } from "../lib/api";
import { useOverlays } from "../store/overlays";
import type { AuditEntry, Backup, UsageSummary } from "../../../shared/types";

type Tab = "usage" | "audit" | "backups";

export function DataOverlay(): React.JSX.Element {
  const close = useOverlays((s) => s.close);
  const [tab, setTab] = useState<Tab>("usage");
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  const [audit, setAudit] = useState<AuditEntry[]>([]);
  const [backups, setBackups] = useState<Backup[]>([]);
  const [refreshKey, setRefreshKey] = useState(0);

  useEffect(() => {
    void api.auditUsage().then(setUsage);
    void api.auditList(200).then(setAudit);
    void api.auditBackups(50).then(setBackups);
  }, [refreshKey]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [close]);

  const fmtCost = (micros: number): string => `$${(micros / 1_000_000).toFixed(4)}`;
  const fmtTokens = (n: number): string => (n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1000 ? `${(n / 1000).toFixed(1)}K` : String(n));
  const fmtTime = (ts: number): string => new Date(ts).toLocaleString("zh-CN", { hour12: false });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={close}>
      <div
        className="flex max-h-[86vh] w-[860px] max-w-[calc(100vw-80px)] flex-col overflow-hidden rounded-2xl border border-popover-border bg-popover shadow-md"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex h-12 shrink-0 items-center gap-1 border-b border-border/50 px-3">
          {(
            [
              ["usage", "用量"],
              ["audit", "审计"],
              ["backups", "备份"],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              className={`rounded-lg px-3 py-1.5 text-ui-sm ${tab === id ? "bg-selected text-foreground" : "text-foreground-subtle hover:bg-surface-hover"}`}
              onClick={() => setTab(id)}
            >
              {label}
            </button>
          ))}
          <div className="flex-1" />
          <button className="rounded-md p-1.5 hover:bg-surface-hover" onClick={() => setRefreshKey((k) => k + 1)}>
            ⟳
          </button>
          <button className="rounded-md p-1.5 hover:bg-surface-hover" onClick={close}>
            <X className="size-4" />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          {tab === "usage" && usage && (
            <div className="flex flex-col gap-4">
              <div className="grid grid-cols-3 gap-3">
                {(
                  [
                    ["今天", usage.today],
                    ["近 7 天", usage.week],
                    ["近 30 天", usage.month],
                  ] as const
                ).map(([label, t]) => (
                  <div key={label} className="rounded-xl border border-border/50 bg-panel px-4 py-3">
                    <div className="text-ui-xs text-foreground-subtlest">{label}</div>
                    <div className="pt-1 font-mono text-ui-lg">
                      {fmtTokens(t.inputTokens + t.outputTokens)} tokens
                    </div>
                    <div className="text-ui-sm text-foreground-subtlest">{fmtCost(t.costMicros)}</div>
                  </div>
                ))}
              </div>
              <div className="rounded-xl border border-border/50 bg-panel">
                <div className="border-b border-border/40 px-4 py-2 text-ui-sm font-medium">按模型（近 30 天）</div>
                {usage.byModel.length === 0 ? (
                  <div className="px-4 py-6 text-center text-ui-sm text-foreground-subtlest">暂无用量</div>
                ) : (
                  usage.byModel.map((m) => (
                    <div key={m.modelId} className="flex items-center gap-2 border-b border-border/20 px-4 py-2 text-ui-sm last:border-0">
                      <span className="truncate font-mono">{m.modelId}</span>
                      <span className="ml-auto shrink-0 text-foreground-subtlest">{fmtTokens(m.inputTokens + m.outputTokens)}</span>
                      <span className="w-20 shrink-0 text-right text-foreground-subtlest">{fmtCost(m.costMicros)}</span>
                    </div>
                  ))
                )}
              </div>
            </div>
          )}
          {tab === "audit" && (
            <div className="overflow-hidden rounded-xl border border-border/50">
              {audit.length === 0 ? (
                <div className="py-10 text-center text-ui-sm text-foreground-subtlest">暂无审计记录</div>
              ) : (
                <table className="w-full text-ui-sm">
                  <thead className="bg-surface text-left text-ui-xs text-foreground-subtlest">
                    <tr>
                      <th className="px-3 py-2">时间</th>
                      <th className="px-3 py-2">工具</th>
                      <th className="px-3 py-2">风险</th>
                      <th className="px-3 py-2">决定</th>
                      <th className="px-3 py-2">范围</th>
                      <th className="px-3 py-2">详情</th>
                    </tr>
                  </thead>
                  <tbody>
                    {audit.map((a) => (
                      <tr key={a.id} className="border-t border-border/20">
                        <td className="whitespace-nowrap px-3 py-1.5 font-mono text-ui-xs">{fmtTime(a.ts)}</td>
                        <td className="px-3 py-1.5 font-mono">{a.tool}</td>
                        <td className="px-3 py-1.5">{a.risk}</td>
                        <td className="px-3 py-1.5">{a.decision}</td>
                        <td className="px-3 py-1.5">{a.scope}</td>
                        <td className="max-w-52 truncate px-3 py-1.5 text-foreground-subtle">{a.detail}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          )}
          {tab === "backups" && (
            <div className="overflow-hidden rounded-xl border border-border/50">
              {backups.length === 0 ? (
                <div className="py-10 text-center text-ui-sm text-foreground-subtlest">暂无备份</div>
              ) : (
                backups.map((b) => (
                  <div key={b.id} className="flex items-center gap-3 border-b border-border/20 px-4 py-2 text-ui-sm last:border-0">
                    <div className="min-w-0 flex-1">
                      <div className="truncate">{b.path}</div>
                      <div className="text-ui-xs text-foreground-subtlest">
                        {fmtTime(b.createdAt)} · {b.size} 字节
                      </div>
                    </div>
                    <button
                      className="h-7 shrink-0 rounded-lg border border-border px-3 text-ui-xs hover:bg-surface-hover"
                      onClick={() => {
                        void api
                          .auditRestore(b.id)
                          .then(() => setRefreshKey((k) => k + 1))
                          .catch((err) => alert(String(err?.message ?? err)));
                      }}
                    >
                      恢复
                    </button>
                  </div>
                ))
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
