import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { Icon, type IconName } from "../../components/Icon";
import { api } from "../../lib/api";
import type { ApprovalLevel, ProviderView, Settings } from "../../lib/types";
import { cn } from "../../lib/utils";
import { DEFAULT_SETTINGS, useAppStore } from "../../stores/app";
import { useUiStore, type SettingsTab } from "../../stores/ui";

const groups: Array<{ id: SettingsTab; label: string; icon: IconName }> = [
  { id: "general", label: "通用", icon: "settings" },
  { id: "mode", label: "模式", icon: "layers" },
  { id: "models", label: "模型与提供商", icon: "cpu" },
  { id: "tools", label: "工具与权限", icon: "shield" },
  { id: "mcp", label: "MCP 服务器", icon: "globe" },
  { id: "appearance", label: "外观", icon: "sun" },
  { id: "advanced", label: "高级", icon: "sliders" },
];

const approvalOptions: Array<{ id: ApprovalLevel; label: string; description: string; icon: IconName }> = [
  { id: "graded", label: "分级", description: "R0 自动；R1–R3 询问；R4 阻止", icon: "shield" },
  { id: "strict", label: "严格", description: "写入与执行逐次询问，不提供永久允许", icon: "alert" },
  { id: "readonly", label: "只读", description: "只允许只读操作", icon: "lock" },
];

const permissions = [
  { level: "R0", name: "只读", policy: "自动允许，记录审计" },
  { level: "R1", name: "写工作区", policy: "每次询问，可本会话 / 本工作区授权" },
  { level: "R2", name: "执行", policy: "每次询问，可按命令前缀授权" },
  { level: "R3", name: "网络或越界", policy: "每次询问，不提供永久允许" },
  { level: "R4", name: "破坏性", policy: "阻止，双重确认也不放行" },
];

const mcpTemplates = [
  { name: "Gmail", description: "只读邮件、生成应用内草稿", icon: "mail" as IconName },
  { name: "Google Calendar", description: "只读日历、生成日程草稿", icon: "calendar" as IconName },
  { name: "PostgreSQL", description: "只读查询与表结构浏览", icon: "database" as IconName },
];

export function SettingsModal() {
  const open = useUiStore((state) => state.settingsOpen);
  const tab = useUiStore((state) => state.settingsTab);
  const close = useUiStore((state) => state.closeSettings);
  const openSettings = useUiStore((state) => state.openSettings);
  const openData = useUiStore((state) => state.openData);
  const toast = useUiStore((state) => state.toast);
  const askPrompt = useUiStore((state) => state.askPrompt);
  const askConfirm = useUiStore((state) => state.askConfirm);

  const settings = useAppStore((state) => state.settings);
  const info = useAppStore((state) => state.info);
  const providers = useAppStore((state) => state.providers);
  const saveSettings = useAppStore((state) => state.saveSettings);
  const refreshProviders = useAppStore((state) => state.refreshProviders);

  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<string | null>(null);
  const [testing, setTesting] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      void refreshProviders();
    }
  }, [open, refreshProviders]);

  if (!open) {
    return null;
  }

  const visibleGroups = groups.filter(
    (group) => !query.trim() || group.label.toLowerCase().includes(query.trim().toLowerCase()),
  );

  function update(patch: Partial<Settings>) {
    void saveSettings(patch);
  }

  async function setProviderKey(provider: ProviderView) {
    const key = await askPrompt({
      title: `设置 ${provider.name} 密钥`,
      label: "密钥只写入本地 keys.json（0600），不会进入日志、数据库或对话。",
      placeholder: "粘贴 API Key",
      confirmText: "保存密钥",
    });
    if (!key) {
      return;
    }
    await api.providers.setKey(provider.id, key);
    await refreshProviders();
    toast({ title: "密钥已保存", description: provider.name, tone: "ok" });
  }

  async function clearProviderKey(provider: ProviderView) {
    const confirmed = await askConfirm({
      title: `清除 ${provider.name} 密钥？`,
      description: "删除后该提供商的模型将不可用。",
      confirmText: "清除",
      danger: true,
    });
    if (!confirmed) {
      return;
    }
    await api.providers.clearKey(provider.id);
    await refreshProviders();
    toast({ title: "密钥已清除", tone: "warn" });
  }

  async function testProvider(provider: ProviderView) {
    setTesting(provider.id);
    try {
      const result = await api.providers.test(provider.id);
      toast({
        title: result.ok ? "连接正常" : "连接失败",
        description: `${result.message}${result.latencyMs ? ` · ${result.latencyMs} ms` : ""}`,
        tone: result.ok ? "ok" : "danger",
      });
    } catch (error) {
      toast({ title: "连接测试失败", description: String(error), tone: "danger" });
    } finally {
      setTesting(null);
    }
  }

  return createPortal(
    <div className="overlay-layer">
      <div className="overlay-backdrop" onClick={close} />
      <div className="settings-modal" role="dialog" aria-modal="true" aria-label="设置">
        <div className="settings-head">
          <Icon name="settings" />
          <h2>设置</h2>
          <div className="settings-search">
            <Icon name="search" className="ico" />
            <input
              value={query}
              placeholder="搜索设置…"
              onChange={(event) => setQuery(event.target.value)}
            />
          </div>
          <button type="button" className="icon-btn sm" aria-label="关闭设置" onClick={close}>
            <Icon name="x" />
          </button>
        </div>

        <div className="settings-body">
          <nav className="settings-nav">
            <div className="settings-nav-group">设置</div>
            {visibleGroups.map((group) => (
              <button
                key={group.id}
                type="button"
                className={cn("settings-nav-item", tab === group.id && "is-active")}
                onClick={() => openSettings(group.id)}
              >
                <Icon name={group.icon} className="ico" />
                {group.label}
              </button>
            ))}
            <div className="settings-nav-group">数据</div>
            <button type="button" className="settings-nav-item" onClick={() => openData("usage")}>
              <Icon name="zap" className="ico" />
              用量统计
            </button>
            <button type="button" className="settings-nav-item" onClick={() => openData("audit")}>
              <Icon name="shield-check" className="ico" />
              审计记录
            </button>
            <button type="button" className="settings-nav-item" onClick={() => openData("backup")}>
              <Icon name="archive" className="ico" />
              备份与恢复
            </button>
          </nav>

          <div className="settings-content">
            {query.trim() && visibleGroups.length === 0 ? (
              <div className="panel-empty">
                <Icon name="search" size={20} />
                <div>没有匹配的设置分组</div>
                <div className="panel-empty-hint">试试「模型」「权限」「外观」等关键词。</div>
              </div>
            ) : null}

            {tab === "general" ? (
              <SettingsSection title="通用" desc="启动行为、语言与本地数据。">
                <Row label="界面语言" desc="当前版本固定为简体中文；完整国际化需要消息编号表。">
                  <select className="select" value="zh-CN" disabled>
                    <option value="zh-CN">简体中文</option>
                  </select>
                </Row>
                <Row label="当前模式" desc="切换后立即生效，并作为下次启动的默认模式。">
                  <select
                    className="select"
                    value={settings.mode}
                    onChange={(event) => update({ mode: event.target.value })}
                  >
                    <option value="code">Code 模式</option>
                    <option value="work">Work 模式</option>
                  </select>
                </Row>
                <Row label="更新渠道" desc="自动更新在 M4 接入；当前只保存偏好。">
                  <select
                    className="select"
                    value={settings.updateChannel}
                    onChange={(event) => update({ updateChannel: event.target.value })}
                  >
                    <option value="stable">稳定版</option>
                    <option value="beta">测试版</option>
                  </select>
                </Row>
                <Row label="自动检查更新" desc="仅检查签名清单，不上传任何数据。">
                  <Switch on={settings.autoUpdate} onChange={(value) => update({ autoUpdate: value })} />
                </Row>
                <Row label="遥测" desc="产品不收集遥测；此项固定关闭。">
                  <Switch on={settings.telemetry} onChange={() => undefined} disabled />
                </Row>
                <Row label="数据目录" desc="会话、密钥、日志与备份都保存在本机。">
                  <span className="mono">{info?.dataDir ?? "~/.craftwork"}</span>
                </Row>
              </SettingsSection>
            ) : null}

            {tab === "mode" ? (
              <SettingsSection title="模式" desc="Code 与 Work 共用同一套工作区与审批策略，只改变界面和可用工具。">
                <Row label="默认模式" desc="新建窗口与新建线程时使用。">
                  <div className="radio-cards">
                    {(["code", "work"] as const).map((mode) => (
                      <button
                        key={mode}
                        type="button"
                        className={cn("radio-card", settings.mode === mode && "is-on")}
                        onClick={() => update({ mode })}
                      >
                        <Icon name={mode === "code" ? "code" : "briefcase"} className="ico" />
                        {mode === "code" ? "Code" : "Work"}
                      </button>
                    ))}
                  </div>
                </Row>
                <Row label="Code 模式" desc="文件树、代码编辑、差异、终端、Shell 与 Git。">
                  <span className="badge accent">17 个内置工具</span>
                </Row>
                <Row label="Work 模式" desc="文档预览、Word / Excel、PDF、邮件与日历 MCP。">
                  <span className="badge work">文档 · 文件 · MCP</span>
                </Row>
              </SettingsSection>
            ) : null}

            {tab === "models" ? (
              <SettingsSection title="模型与提供商" desc="密钥只保存在本地；切换模型只影响后续 run。">
                <Row label="默认模型" desc="新线程使用的模型。">
                  <select
                    className="select"
                    value={settings.defaultModel}
                    onChange={(event) => update({ defaultModel: event.target.value })}
                  >
                    {providers.flatMap((provider) =>
                      provider.models.map((model) => (
                        <option key={model.id} value={model.id}>
                          {provider.name} · {model.name}
                        </option>
                      )),
                    )}
                  </select>
                </Row>

                {providers.map((provider) => (
                  <div key={provider.id} className={cn("provider-card", expanded === provider.id && "is-open")}>
                    <div className="provider-head">
                      <div className="provider-logo">{provider.name.slice(0, 2)}</div>
                      <div className="provider-main">
                        <div className="provider-name">
                          {provider.name}
                          <span className={cn("badge", provider.available ? "ok" : "neutral")}>
                            {provider.available ? "可用" : "未配置"}
                          </span>
                        </div>
                        <div className="provider-sub">{provider.endpoint || provider.note || provider.kind}</div>
                      </div>
                      <div className="provider-actions">
                        <button
                          type="button"
                          className="btn sm ghost"
                          disabled={testing === provider.id}
                          onClick={() => void testProvider(provider)}
                        >
                          {testing === provider.id ? <Icon name="loader" className="spin" /> : <Icon name="zap" />}
                          测试连接
                        </button>
                        <button
                          type="button"
                          className="icon-btn sm"
                          aria-label="展开提供商"
                          onClick={() => setExpanded(expanded === provider.id ? null : provider.id)}
                        >
                          <Icon name={expanded === provider.id ? "chevron-up" : "chevron-down"} />
                        </button>
                      </div>
                    </div>
                    {expanded === provider.id ? (
                      <div className="provider-body">
                        <div className="key-line">
                          <Icon name="key" className="ico" />
                          <span>密钥来源：{provider.keySource === "file" ? "keys.json" : provider.keySource === "env" ? "环境变量" : "未配置"}</span>
                          <span className="spacer" />
                          <span className="mono">{provider.keyMasked || "—"}</span>
                          <button type="button" onClick={() => void setProviderKey(provider)}>
                            设置
                          </button>
                          {provider.keySource ? (
                            <button type="button" onClick={() => void clearProviderKey(provider)}>
                              清除
                            </button>
                          ) : null}
                        </div>
                        <table className="cap-table" style={{ marginTop: 8 }}>
                          <thead>
                            <tr>
                              <th>模型</th>
                              <th>上下文</th>
                              <th>流式</th>
                              <th>工具</th>
                              <th>推理</th>
                              <th>图片</th>
                            </tr>
                          </thead>
                          <tbody>
                            {provider.models.map((model) => (
                              <tr key={model.id}>
                                <td>{model.name}</td>
                                <td>{model.contextK}k</td>
                                <Cap yes={model.caps.stream} />
                                <Cap yes={model.caps.tools} />
                                <Cap yes={model.caps.reasoning} />
                                <Cap yes={model.caps.vision} />
                              </tr>
                            ))}
                          </tbody>
                        </table>
                        {provider.note ? <div className="sec-desc" style={{ marginTop: 8 }}>{provider.note}</div> : null}
                      </div>
                    ) : null}
                  </div>
                ))}
              </SettingsSection>
            ) : null}

            {tab === "tools" ? (
              <SettingsSection title="工具与权限" desc="审批档位与风险分级共同决定一次工具调用是否执行。">
                <Row label="审批档位" desc="与输入框底部的盾牌按钮双向同步。">
                  <div className="radio-cards">
                    {approvalOptions.map((option) => (
                      <button
                        key={option.id}
                        type="button"
                        className={cn("radio-card", settings.approvalLevel === option.id && "is-on")}
                        onClick={() => update({ approvalLevel: option.id })}
                        title={option.description}
                      >
                        <Icon name={option.icon} className="ico" />
                        {option.label}
                      </button>
                    ))}
                  </div>
                </Row>
                <div className="set-row" style={{ display: "block" }}>
                  <table className="perm-table">
                    <thead>
                      <tr>
                        <th>等级</th>
                        <th>判定</th>
                        <th>默认策略</th>
                      </tr>
                    </thead>
                    <tbody>
                      {permissions.map((row) => (
                        <tr key={row.level}>
                          <td>
                            <span className={cn("perm-level", row.level)}>{row.level}</span>
                          </td>
                          <td>{row.name}</td>
                          <td>{row.policy}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
                <Row label="工作区授权" desc="选择「本会话 / 本工作区」后记录在这里；R3 / R4 永远不会出现。">
                  <span className="badge neutral">暂无永久授权</span>
                </Row>
                <Row label="沙箱" desc="macOS Seatbelt 与 Windows 受限令牌在 M4 接入；当前版本依赖审批与路径校验。">
                  <span className="badge warn">未启用</span>
                </Row>
              </SettingsSection>
            ) : null}

            {tab === "mcp" ? (
              <SettingsSection title="MCP 服务器" desc="MCP 客户端与服务器管理在 M4 接入；下面是推荐模板。">
                <div className="panel-empty" style={{ padding: "18px 0 22px" }}>
                  <Icon name="globe" size={22} />
                  <div>还没有已连接的 MCP 服务器</div>
                  <div className="panel-empty-hint">MCP 工具会以 mcp__服务器__工具 命名，并在首次连接时展示权限。</div>
                </div>
                {mcpTemplates.map((template) => (
                  <div key={template.name} className="mcp-card">
                    <div className="provider-head" style={{ padding: 0 }}>
                      <div className="provider-logo">
                        <Icon name={template.icon} />
                      </div>
                      <div className="provider-main">
                        <div className="provider-name">{template.name}</div>
                        <div className="provider-sub">{template.description}</div>
                      </div>
                      <span className="badge neutral">M4 接入</span>
                    </div>
                  </div>
                ))}
              </SettingsSection>
            ) : null}

            {tab === "appearance" ? (
              <SettingsSection title="外观" desc="主题与字体只影响本地界面。">
                <Row label="主题" desc="深色 / 浅色两套 token；代码主题独立。">
                  <div className="theme-cards">
                    {(["dark", "light"] as const).map((theme) => (
                      <button
                        key={theme}
                        type="button"
                        className={cn("theme-card", theme, settings.theme === theme && "is-on")}
                        onClick={() => update({ theme })}
                      >
                        <div className="theme-preview">
                          <i className="s" style={{ flex: 2 }} />
                          <i className="m" style={{ flex: 5 }} />
                          <i className="a" />
                        </div>
                        <div className="theme-label">
                          {theme === "dark" ? "深色" : "浅色"}
                          <Icon name={theme === "dark" ? "moon" : "sun"} />
                        </div>
                      </button>
                    ))}
                  </div>
                </Row>
                <Row label="界面字号" desc="影响消息、侧栏与设置；字号缩放与全局 token 在 M1 接入。">
                  <select className="select" value={settings.fontSize} disabled title="M1 接入">
                    {[12, 13, 14, 15].map((size) => (
                      <option key={size} value={size}>
                        {size}px
                      </option>
                    ))}
                  </select>
                </Row>
                <Row label="终端字号" desc="xterm.js 使用的字号。">
                  <select
                    className="select"
                    value={settings.terminalFontSize}
                    onChange={(event) => update({ terminalFontSize: Number(event.target.value) })}
                  >
                    {[11, 12, 13, 14, 15].map((size) => (
                      <option key={size} value={size}>
                        {size}px
                      </option>
                    ))}
                  </select>
                </Row>
                <Row label="代码主题" desc="预留给独立的编辑器配色；当前跟随界面主题，独立配色在 M1 接入。">
                  <select className="select" value={settings.codeTheme} disabled title="M1 接入">
                    <option value="dark">深色</option>
                    <option value="light">浅色</option>
                  </select>
                </Row>
              </SettingsSection>
            ) : null}

            {tab === "advanced" ? (
              <SettingsSection title="高级" desc="本地数据、版本与恢复。">
                <Row label="版本" desc="构建信息">
                  <span className="mono">
                    {info?.version ?? "0.1.0"} · {info?.commit ?? "dev"} · {info?.platform ?? "web"}
                  </span>
                </Row>
                <Row label="备份与恢复" desc="写入文件前的备份保存在 ~/.craftwork/backups。">
                  <button type="button" className="btn sm" onClick={() => openData("backup")}>
                    <Icon name="archive" />
                    查看备份
                  </button>
                </Row>
                <Row label="导出数据" desc="导出数据库与配置，不包含 keys.json。">
                  <button
                    type="button"
                    className="btn sm ghost"
                    onClick={() => toast({ title: "导出在 M4 接入", tone: "info" })}
                  >
                    <Icon name="download" />
                    导出
                  </button>
                </Row>
                <Row label="重置设置" desc="恢复默认设置，不删除会话与工作区。">
                  <button
                    type="button"
                    className="btn sm ghost"
                    onClick={async () => {
                      const confirmed = await askConfirm({
                        title: "恢复默认设置？",
                        description: "主题、字号、审批档位等会恢复默认；会话与工作区不受影响。",
                        confirmText: "恢复默认",
                        danger: true,
                      });
                      if (confirmed) {
                        update(DEFAULT_SETTINGS);
                        toast({ title: "已恢复默认设置", tone: "ok" });
                      }
                    }}
                  >
                    <Icon name="rotate-ccw" />
                    恢复默认
                  </button>
                </Row>
                <Row label="许可证" desc="Apache-2.0；贡献流程使用 DCO。">
                  <span className="badge neutral">Apache-2.0</span>
                </Row>
              </SettingsSection>
            ) : null}
          </div>
        </div>

        <div className="settings-foot">
          <span className="settings-tip">
            <Icon name="lock" />
            设置保存在本地，不随对话上传。
          </span>
          <span className="spacer" />
          <button type="button" className="btn primary" onClick={close}>
            完成
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

function SettingsSection({ title, desc, children }: { title: string; desc: string; children: React.ReactNode }) {
  return (
    <section className="settings-section">
      <div className="settings-section-title">{title}</div>
      <div className="sec-desc">{desc}</div>
      {children}
    </section>
  );
}

function Row({ label, desc, children }: { label: string; desc?: string; children: React.ReactNode }) {
  return (
    <div className="set-row">
      <div className="set-row-main">
        <div className="set-row-label">{label}</div>
        {desc ? <div className="set-row-desc">{desc}</div> : null}
      </div>
      <div className="set-control">{children}</div>
    </div>
  );
}

function Switch({ on, onChange, disabled }: { on: boolean; onChange: (value: boolean) => void; disabled?: boolean }) {
  return (
    <button
      type="button"
      className={cn("switch", on && "is-on")}
      aria-pressed={on}
      disabled={disabled}
      onClick={() => {
        if (!disabled) {
          onChange(!on);
        }
      }}
    />
  );
}

function Cap({ yes }: { yes: boolean }) {
  return (
    <td>
      <span className={cn("cap-pill", yes ? "y" : "n")}>{yes ? "✓" : "—"}</span>
    </td>
  );
}
