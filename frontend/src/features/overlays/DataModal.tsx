import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "../../components/Icon";
import { api } from "../../lib/api";
import type { AuditEntry, Backup, UsageSummary } from "../../lib/types";
import type { DataTab } from "../../stores/ui";
import { cn, formatBytes, formatCost, formatTime, formatTokens } from "../../lib/utils";
import { useUiStore } from "../../stores/ui";

export function DataModal() {
  const open = useUiStore((state) => state.dataOpen);
  const tab = useUiStore((state) => state.dataTab);
  const openData = useUiStore((state) => state.openData);
  const close = useUiStore((state) => state.closeData);
  const askConfirm = useUiStore((state) => state.askConfirm);
  const toast = useUiStore((state) => state.toast);

  const [audit, setAudit] = useState<AuditEntry[]>([]);
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  const [backups, setBackups] = useState<Backup[]>([]);
  const [filter, setFilter] = useState<"all" | "auto" | "allow" | "deny">("all");
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!open) {
      return;
    }
    let alive = true;
    setLoading(true);
    void (async () => {
      try {
        if (tab === "audit") {
          const rows = await api.audit.list(200);
          if (alive) {
            setAudit(rows);
          }
        } else if (tab === "usage") {
          const summary = await api.audit.usage();
          if (alive) {
            setUsage(summary);
          }
        } else {
          const rows = await api.audit.backups(50);
          if (alive) {
            setBackups(rows);
          }
        }
      } finally {
        if (alive) {
          setLoading(false);
        }
      }
    })();
    return () => {
      alive = false;
    };
  }, [open, tab]);

  if (!open) {
    return null;
  }

  const filteredAudit = audit.filter((entry) =>
    filter === "all" ? true : entry.decision === filter,
  );

  const maxModelTokens = Math.max(
    1,
    ...(usage?.byModel.map((item) => item.inputTokens + item.outputTokens) ?? [1]),
  );

  async function restore(backup: Backup) {
    const confirmed = await askConfirm({
      title: "恢复这个备份？",
      description: "目标文件会被备份内容覆盖。恢复前请确认当前文件没有需要保留的修改。",
      detail: backup.path,
      confirmText: "恢复",
      danger: true,
    });
    if (!confirmed) {
      return;
    }
    try {
      await api.audit.restore(backup.id);
      toast({ title: "已恢复备份", description: backup.path, tone: "ok" });
    } catch (error) {
      toast({ title: "恢复失败", description: String(error), tone: "danger" });
    }
  }

  return createPortal(
    <div className="overlay-layer">
      <div className="overlay-backdrop" onClick={close} />
      <div className="data-modal" role="dialog" aria-modal="true" aria-label="数据面板">
        <div className="data-head">
          <Icon name="database" />
          <h2>{tab === "audit" ? "审计记录" : tab === "usage" ? "用量统计" : "备份与恢复"}</h2>
          <button type="button" className="icon-btn sm" aria-label="刷新" onClick={() => openData(tab)}>
            <Icon name="refresh" />
          </button>
          <button type="button" className="icon-btn sm" aria-label="关闭" onClick={close}>
            <Icon name="x" />
          </button>
        </div>

        <div className="data-tabs">
          <button type="button" className={cn("data-tab", tab === "audit" && "is-active")} onClick={() => openData("audit")}>
            <Icon name="shield-check" className="ico" />
            审计
          </button>
          <button type="button" className={cn("data-tab", tab === "usage" && "is-active")} onClick={() => openData("usage")}>
            <Icon name="zap" className="ico" />
            用量
          </button>
          <button type="button" className={cn("data-tab", tab === "backup" && "is-active")} onClick={() => openData("backup")}>
            <Icon name="archive" className="ico" />
            备份
          </button>
        </div>

        <div className="data-body">
          {loading ? (
            <div className="panel-empty">
              <Icon name="loader" size={20} className="spin" />
              <div>正在读取本地数据…</div>
            </div>
          ) : null}

          {!loading && tab === "audit" ? (
            <>
              <div className="audit-filters">
                {(["all", "auto", "allow", "deny"] as const).map((value) => (
                  <button
                    key={value}
                    type="button"
                    className={cn("btn sm", filter === value ? "primary" : "ghost")}
                    onClick={() => setFilter(value)}
                  >
                    {value === "all" ? "全部" : value === "auto" ? "自动" : value === "allow" ? "允许" : "拒绝"}
                  </button>
                ))}
                <span className="spacer" />
                <span className="sec-desc">共 {filteredAudit.length} 条记录</span>
              </div>
              <div className="audit-table-wrap">
                <table className="audit-table">
                  <thead>
                    <tr>
                      <th>时间</th>
                      <th>工具</th>
                      <th>风险</th>
                      <th>决定</th>
                      <th>范围</th>
                      <th>详情</th>
                    </tr>
                  </thead>
                  <tbody>
                    {filteredAudit.length === 0 ? (
                      <tr>
                        <td colSpan={6} style={{ textAlign: "center", color: "var(--text-4)", padding: 22 }}>
                          还没有审计记录
                        </td>
                      </tr>
                    ) : (
                      filteredAudit.map((entry) => (
                        <tr key={entry.id}>
                          <td className="a-time">{formatTime(entry.ts)}</td>
                          <td className="a-tool">{entry.tool}</td>
                          <td>
                            <span className={cn("risk-badge", `risk-${entry.risk}`)}>{entry.risk}</span>
                          </td>
                          <td>
                            <span className={cn("audit-decision", entry.decision)}>
                              {entry.decision === "allow" ? "允许" : entry.decision === "deny" ? "拒绝" : "自动"}
                            </span>
                          </td>
                          <td className="a-scope">{entry.scope || "—"}</td>
                          <td>{entry.detail || "—"}</td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
              </div>
            </>
          ) : null}

          {!loading && tab === "usage" && usage ? (
            <>
              <div className="usage-cards">
                <UsageCard label="今天" totals={usage.today} />
                <UsageCard label="近 7 天" totals={usage.week} />
                <UsageCard label="近 30 天" totals={usage.month} />
                <div className="usage-card">
                  <div className="usage-label">本地模型</div>
                  <div className="usage-value">{usage.byModel.length}</div>
                  <div className="usage-sub">按提供商聚合</div>
                </div>
              </div>

              <div className="settings-section-title" style={{ marginBottom: 8 }}>
                按模型
              </div>
              <div className="usage-bars">
                {usage.byModel.length === 0 ? (
                  <div className="sec-desc">还没有用量记录。</div>
                ) : (
                  usage.byModel.map((item) => (
                    <div key={item.modelId} className="usage-bar-row">
                      <span className="ub-name">{item.modelId}</span>
                      <span className="ub-track">
                        <i
                          className="ub-fill"
                          style={{ width: `${Math.max(4, ((item.inputTokens + item.outputTokens) / maxModelTokens) * 100)}%` }}
                        />
                      </span>
                      <span className="ub-num">
                        {formatTokens(item.inputTokens + item.outputTokens)} · {formatCost(item.costMicros)}
                      </span>
                    </div>
                  ))
                )}
              </div>

              <div className="settings-section-title" style={{ margin: "16px 0 8px" }}>
                每日趋势
              </div>
              <div className="usage-bars">
                {usage.byDay.map((day) => {
                  const max = Math.max(1, ...usage.byDay.map((item) => item.inputTokens + item.outputTokens));
                  return (
                    <div key={day.day} className="usage-bar-row">
                      <span className="ub-name">{day.day}</span>
                      <span className="ub-track">
                        <i
                          className="ub-fill"
                          style={{ width: `${Math.max(4, ((day.inputTokens + day.outputTokens) / max) * 100)}%` }}
                        />
                      </span>
                      <span className="ub-num">
                        {formatTokens(day.inputTokens + day.outputTokens)} · {day.runs} 次
                      </span>
                    </div>
                  );
                })}
              </div>
            </>
          ) : null}

          {!loading && tab === "usage" && !usage ? (
            <div className="panel-empty">
              <Icon name="zap" size={20} />
              <div>还没有用量数据</div>
            </div>
          ) : null}

          {!loading && tab === "backup" ? (
            <div className="backup-list">
              {backups.length === 0 ? (
                <div className="panel-empty">
                  <Icon name="archive" size={20} />
                  <div>还没有备份</div>
                  <div className="panel-empty-hint">写入文件前会自动备份到 ~/.craftwork/backups。</div>
                </div>
              ) : (
                backups.map((backup) => (
                  <div key={backup.id} className="backup-item">
                    <Icon name="file" />
                    <div className="backup-main">
                      <div className="backup-name">{backup.path}</div>
                      <div className="backup-meta">
                        {formatTime(backup.createdAt)} · {formatBytes(backup.size)} · {backup.hash.slice(0, 12)}
                      </div>
                    </div>
                    <span className="spacer" />
                    <button type="button" className="btn sm ghost" onClick={() => void restore(backup)}>
                      <Icon name="rotate-ccw" />
                      恢复
                    </button>
                  </div>
                ))
              )}
            </div>
          ) : null}
        </div>
      </div>
    </div>,
    document.body,
  );
}

function UsageCard({ label, totals }: { label: string; totals: UsageSummary["today"] }) {
  return (
    <div className="usage-card">
      <div className="usage-label">{label}</div>
      <div className="usage-value">
        {formatTokens(totals.inputTokens + totals.outputTokens)}
        <small>tokens</small>
      </div>
      <div className="usage-sub">
        {totals.runs} 次 · {formatCost(totals.costMicros)}
      </div>
    </div>
  );
}

export type { DataTab };
