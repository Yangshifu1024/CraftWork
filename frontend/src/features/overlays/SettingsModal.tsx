/**
 * 设置弹窗。
 *
 * 负责：按分组展示并修改应用设置（通用、模式、模型与提供商、工具与权限、MCP、外观、高级），
 * 同时承载提供商密钥管理、连接测试、MCP 服务器增删与连接，以及跳转到数据面板。
 *
 * 布局为左侧分组导航 + 右侧内容区；所有设置修改都通过 saveSettings 立即保存并生效。
 */
import { useCallback, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { Icon, type IconName } from "../../components/Icon";
import { api } from "../../lib/api";
import { guardFreeLevel } from "../../lib/approval";
import type { ApprovalLevel, CatalogMeta, MCPServerView, ProviderView, Settings } from "../../lib/types";
import { cn } from "../../lib/utils";
import { DEFAULT_SETTINGS, useAppStore } from "../../stores/app";
import { type SettingsTab, useUiStore } from "../../stores/ui";
import { CatalogBrowser } from "./CatalogBrowser";
import { ProviderEditor } from "./ProviderEditor";

/** 左侧导航的分组定义；id 与 settingsTab 对应，图标取自 Icon 组件。 */
const groups: Array<{ id: SettingsTab; label: string; icon: IconName }> = [
  { id: "general", label: "通用", icon: "settings" },
  { id: "mode", label: "模式", icon: "layers" },
  { id: "models", label: "模型与提供商", icon: "cpu" },
  { id: "tools", label: "工具与权限", icon: "shield" },
  { id: "mcp", label: "MCP 服务器", icon: "globe" },
  { id: "appearance", label: "外观", icon: "sun" },
  { id: "advanced", label: "高级", icon: "sliders" },
];

/** 四种审批档位的展示信息；实际策略由后端按档位与风险等级计算。 */
const approvalOptions: Array<{ id: ApprovalLevel; label: string; description: string; icon: IconName }> = [
  { id: "graded", label: "分级", description: "R0 自动；R1–R3 询问；R4 阻止", icon: "shield" },
  { id: "strict", label: "严格", description: "写入与执行逐次询问，不提供永久允许", icon: "alert" },
  { id: "readonly", label: "只读", description: "只允许只读操作", icon: "lock" },
  { id: "free", label: "自由", description: "放行所有操作包括 R4；切换前需要确认", icon: "zap" },
];

/** 风险等级说明表，用于在「工具与权限」里解释每一级的默认策略。 */
const permissions = [
  { level: "R0", name: "只读", policy: "自动允许，记录审计" },
  { level: "R1", name: "写工作区", policy: "每次询问，可本会话 / 本工作区授权" },
  { level: "R2", name: "执行", policy: "每次询问，可按命令前缀授权" },
  { level: "R3", name: "网络或越界", policy: "每次询问，不提供永久允许" },
  { level: "R4", name: "破坏性", policy: "阻止，双重确认也不放行" },
];

/** MCP 服务器模板，仅作展示提示；MCP 的完整能力仍是 M4 的留桩。 */
const mcpTemplates = [
  { name: "Gmail", description: "只读邮件、生成应用内草稿", icon: "mail" as IconName },
  { name: "Google Calendar", description: "只读日历、生成日程草稿", icon: "calendar" as IconName },
  { name: "PostgreSQL", description: "只读查询与表结构浏览", icon: "database" as IconName },
];

/**
 * 设置弹窗组件。
 *
 * props：无。数据来源：
 * - useUiStore：弹窗开关、当前分组，以及确认框 / 输入框 / 提示；
 * - useAppStore：设置值、应用信息与提供商列表，修改统一走 saveSettings。
 *
 * 本地状态：
 * - query：分组搜索关键字；
 * - expanded：当前展开的提供商卡片 ID；
 * - testing：正在测试连接的提供商 ID；
 * - mcpServers / mcpBusy：MCP 服务器列表与正在处理的服务器；
 * - providerEditor：提供商编辑器是否打开以及编辑对象。
 */
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
  const refreshSettings = useAppStore((state) => state.refreshSettings);

  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<string | null>(null);
  const [testing, setTesting] = useState<string | null>(null);
  const [mcpServers, setMcpServers] = useState<MCPServerView[]>([]);
  const [mcpBusy, setMcpBusy] = useState<string | null>(null);
  const [providerEditor, setProviderEditor] = useState<{ provider: ProviderView | null } | null>(null);
  const [catalogOpen, setCatalogOpen] = useState(false);
  const [catalogMeta, setCatalogMeta] = useState<CatalogMeta | null>(null);

  /**
   * 读取已保存的 MCP 服务器列表。
   * 失败时给出提示但不阻塞界面，列表保持上一次的内容。
   * 依赖仅模块级 api 与稳定的 store action，用 useCallback 固定引用供 effect 依赖。
   */
  const refreshMCP = useCallback(async () => {
    try {
      setMcpServers(await api.mcp.list());
    } catch (error) {
      toast({ title: "读取 MCP 服务器失败", description: String(error), tone: "danger" });
    }
  }, [toast]);

  useEffect(() => {
    // 只有打开弹窗并停留在 MCP 分组时才读取服务器列表，其他分组不做请求。
    if (open && tab === "mcp") {
      void refreshMCP();
    }
    // refreshMCP 只在打开 MCP 标签时读取服务器列表。
  }, [open, tab, refreshMCP]);

  useEffect(() => {
    // 关闭时收起提供商编辑器与目录浏览器，避免下次打开残留上一次的状态。
    if (!open) {
      setProviderEditor(null);
      setCatalogOpen(false);
    }
  }, [open]);

  useEffect(() => {
    // 进入模型分组时读取目录版本，页脚用它展示数据来源。
    if (open && tab === "models" && !catalogMeta) {
      api.providers
        .catalogMeta()
        .then(setCatalogMeta)
        .catch(() => setCatalogMeta(null));
    }
  }, [open, tab, catalogMeta]);

  useEffect(() => {
    // 每次打开都刷新提供商列表，保证密钥状态与可用性是最新的。
    if (open) {
      void refreshProviders();
    }
  }, [open, refreshProviders]);

  if (!open) {
    return null;
  }

  // 分组搜索只匹配分组名称，不深入具体设置项。
  const visibleGroups = groups.filter(
    (group) => !query.trim() || group.label.toLowerCase().includes(query.trim().toLowerCase()),
  );

  /**
   * 保存部分设置。
   * 这里不等待结果，后端保存成功后 store 会同步最新值并触发界面更新。
   */
  function update(patch: Partial<Settings>) {
    void saveSettings(patch);
  }

  /**
   * 切换审批档位。
   * 自由档位会放行包括 R4 在内的所有操作，切换前弹出风险确认框；
   * 用户取消时什么都不保存，界面停留在原档位。
   */
  async function changeApprovalLevel(level: ApprovalLevel) {
    if (!(await guardFreeLevel(level, settings.approvalLevel))) {
      return;
    }
    update({ approvalLevel: level });
  }

  /**
   * 设置提供商密钥。
   * 通过输入框取得密钥后写入本地 keys.json（不会进入日志、数据库或对话），
   * 再刷新提供商列表，让模型可用状态立即更新。
   */
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

  /**
   * 清除提供商密钥，需要二次确认；
   * 清除后该提供商的模型会变为不可用。
   */
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

  /**
   * 测试提供商连接。
   * 测试期间把提供商 ID 记到 testing，用来禁用对应卡片的按钮；
   * 结果通过提示条展示，成功时附带延迟。
   */
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

  /**
   * 添加 MCP 服务器。
   * 分两步收集名称与启动命令，然后保存并尝试连接；
   * 即使连接失败也保留配置，只在卡片上显示错误。
   */
  async function addMCPServer() {
    const name = await askPrompt({
      title: "添加 MCP 服务器",
      label: "给服务器起一个名称，例如 filesystem。",
      placeholder: "filesystem",
      confirmText: "下一步",
    });
    if (!name) {
      return;
    }
    const command = await askPrompt({
      title: `${name} 的启动命令`,
      label: "命令以当前用户权限在本地运行；模型密钥会从环境变量中过滤。",
      placeholder: "npx -y @modelcontextprotocol/server-filesystem .",
      confirmText: "添加并连接",
    });
    if (!command) {
      return;
    }
    setMcpBusy("add");
    try {
      const server = await api.mcp.add(name, command);
      toast({
        title: server.error ? "服务器已保存，但连接失败" : "MCP 服务器已连接",
        description: server.error ?? `${server.name} · ${(server.tools ?? []).length} 个工具`,
        tone: server.error ? "warn" : "ok",
      });
      await refreshMCP();
    } catch (error) {
      toast({ title: "添加 MCP 服务器失败", description: String(error), tone: "danger" });
    } finally {
      setMcpBusy(null);
    }
  }

  /** 连接指定的 MCP 服务器，并刷新列表以拿到工具清单。 */
  async function connectMCP(server: MCPServerView) {
    setMcpBusy(server.id);
    try {
      const updated = await api.mcp.connect(server.id);
      toast({
        title: updated.error ? "连接失败" : "已连接",
        description: updated.error ?? `${updated.name} · ${(updated.tools ?? []).length} 个工具`,
        tone: updated.error ? "danger" : "ok",
      });
      await refreshMCP();
    } catch (error) {
      toast({ title: "连接 MCP 服务器失败", description: String(error), tone: "danger" });
    } finally {
      setMcpBusy(null);
    }
  }

  /** 断开指定的 MCP 服务器连接，配置保留，可再次连接。 */
  async function disconnectMCP(server: MCPServerView) {
    setMcpBusy(server.id);
    try {
      await api.mcp.disconnect(server.id);
      await refreshMCP();
    } catch (error) {
      toast({ title: "断开 MCP 服务器失败", description: String(error), tone: "danger" });
    } finally {
      setMcpBusy(null);
    }
  }

  /**
   * 移除 MCP 服务器：需要二次确认，移除后已注册的工具立即失效。
   */
  async function removeMCP(server: MCPServerView) {
    const confirmed = await askConfirm({
      title: `移除 ${server.name}？`,
      description: "会断开连接并删除服务器配置；已注册的工具立即失效。",
      confirmText: "移除",
      danger: true,
    });
    if (!confirmed) {
      return;
    }
    try {
      await api.mcp.remove(server.id);
      await refreshMCP();
    } catch (error) {
      toast({ title: "移除 MCP 服务器失败", description: String(error), tone: "danger" });
    }
  }

  /**
   * 添加提供商后刷新列表；如果还没有默认模型，自动选中新添加的第一个模型。
   *
   * 后端不会自动替用户选模型，这里只在默认值为空时补一次，已有选择保持不变。
   */
  async function afterProviderAdded(provider: ProviderView) {
    await refreshProviders();
    if (!useAppStore.getState().settings.defaultModel && provider.models[0]) {
      await saveSettings({ defaultModel: provider.models[0].ref });
    } else {
      await refreshSettings();
    }
  }

  /** 目录导入完成后的收尾：关闭浏览器、刷新列表并按需设置默认模型。 */
  async function handleCatalogImported(provider: ProviderView) {
    setCatalogOpen(false);
    await afterProviderAdded(provider);
    const needsKey = provider.keyRequired && !provider.keySource;
    toast({
      title: needsKey ? "还需配置 API Key" : "提供商已添加",
      description: needsKey
        ? `${provider.name} · ${provider.models.length} 个模型；展开提供商卡片即可设置密钥`
        : `${provider.name} · ${provider.models.length} 个模型`,
      tone: needsKey ? "warn" : "ok",
    });
  }

  /**
   * 删除提供商。
   *
   * 所有提供商都是用户添加的，删除即移除配置并删除密钥；如果默认模型或
   * 失败转移模型指向它，后端会一并清空。
   */
  async function deleteProvider(provider: ProviderView) {
    const confirmed = await askConfirm({
      title: `删除 ${provider.name}？`,
      description: "该提供商的模型会立即从列表中移除，API Key 同时删除。",
      confirmText: "删除",
      danger: true,
    });
    if (!confirmed) {
      return;
    }
    try {
      await api.providers.deleteProvider(provider.id);
      await refreshProviders();
      await refreshSettings();
      toast({ title: "提供商已删除", tone: "ok" });
    } catch (error) {
      toast({ title: "删除提供商失败", description: String(error), tone: "danger" });
    }
  }

  return createPortal(
    <div className="overlay-layer">
      <div className="settings-modal" role="dialog" aria-modal="true" aria-label="设置">
        <div className="settings-head">
          {/* 顶栏：标题、分组搜索与关闭按钮；搜索只过滤左侧分组 */}
          <Icon name="settings" />
          <h2>设置</h2>
          <div className="settings-search">
            <Icon name="search" className="ico" />
            <input value={query} placeholder="搜索设置…" onChange={(event) => setQuery(event.target.value)} />
          </div>
          <button type="button" className="icon-btn sm" aria-label="关闭设置" onClick={close}>
            <Icon name="x" />
          </button>
        </div>

        <div className="settings-body">
          <nav className="settings-nav">
            {/* 设置分组导航；搜索无匹配时这里的列表会变短甚至为空 */}
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
            {/* 数据入口直接打开数据面板弹窗，不在这里内嵌展示 */}
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
            <div className="settings-content-inner">
              {/* 搜索有输入但没有任何分组匹配时给出空状态 */}
              {query.trim() && visibleGroups.length === 0 ? (
                <div className="panel-empty">
                  <Icon name="search" size={20} />
                  <div>没有匹配的设置分组</div>
                  <div className="panel-empty-hint">试试「模型」「权限」「外观」等关键词。</div>
                </div>
              ) : null}

              {tab === "general" ? (
                /* 通用分组：启动模式、更新偏好、系统通知与数据目录 */
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
                  <Row
                    label="系统通知"
                    desc="审批等待、回复完成或运行失败时发送系统通知；窗口聚焦时不打扰，应用内提示可点击回到对应会话。"
                  >
                    <Switch on={settings.notifications} onChange={(value) => update({ notifications: value })} />
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
                /* 模式分组：默认模式，以及两种模式各自的能力范围说明 */
                <SettingsSection title="模式" desc="Code 与 Work 共用同一套工作区与审批策略，只改变界面和可用工具。">
                  <Row label="默认模式" desc="新建窗口与新建会话时使用。">
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
                /* 模型与提供商分组：所有提供商都由用户添加，没有任何内置或推荐条目 */
                <SettingsSection
                  title="模型与提供商"
                  desc="提供商全部由你添加；密钥只保存在本地，切换模型只影响后续 run。"
                >
                  {catalogOpen || providerEditor ? null : (
                    <>
                      <Row label="默认模型" desc="新会话使用的模型；没有添加提供商时保持为空。">
                        <select
                          className="select"
                          value={settings.defaultModel}
                          onChange={(event) => update({ defaultModel: event.target.value })}
                        >
                          <option value="">未选择模型</option>
                          {providers.flatMap((provider) =>
                            provider.models.map((model) => (
                              <option key={model.ref} value={model.ref}>
                                {provider.name} · {model.name}
                                {model.available ? "" : "（未配置密钥）"}
                              </option>
                            )),
                          )}
                        </select>
                      </Row>

                      <Row
                        label="失败转移模型"
                        desc="主模型遇到限流、5xx、超时或连接失败时自动切换；工具执行过程中不切换，未配置密钥的模型不会生效。"
                      >
                        <select
                          className="select"
                          value={settings.fallbackModel ?? ""}
                          onChange={(event) => update({ fallbackModel: event.target.value })}
                        >
                          <option value="">不自动转移</option>
                          {providers.flatMap((provider) =>
                            provider.models
                              .filter((model) => model.ref !== settings.defaultModel)
                              .map((model) => (
                                <option key={model.ref} value={model.ref}>
                                  {provider.name} · {model.name}
                                  {model.available ? "" : "（未配置密钥）"}
                                </option>
                              )),
                          )}
                        </select>
                      </Row>
                    </>
                  )}

                  {catalogOpen ? (
                    /* 目录浏览器：从 models.dev 快照里选择提供商与模型 */
                    <CatalogBrowser
                      meta={catalogMeta}
                      onCancel={() => setCatalogOpen(false)}
                      onImported={(provider) => void handleCatalogImported(provider)}
                    />
                  ) : providerEditor ? (
                    /* 编辑器打开时替换整个提供商列表，聚焦单个提供商的配置 */
                    <ProviderEditor
                      provider={providerEditor.provider}
                      onCancel={() => setProviderEditor(null)}
                      onSaved={(saved) => {
                        setProviderEditor(null);
                        void afterProviderAdded(saved);
                        if (saved.keyRequired && !saved.keySource) {
                          toast({
                            title: "还需配置 API Key",
                            description: "在提供商卡片中展开并设置密钥后，模型才会出现在模型列表。",
                            tone: "warn",
                          });
                        }
                      }}
                    />
                  ) : (
                    <>
                      <div className="provider-toolbar">
                        <button type="button" className="btn sm" onClick={() => setCatalogOpen(true)}>
                          <Icon name="globe" className="ico" />
                          从模型目录添加
                        </button>
                        <button
                          type="button"
                          className="btn sm ghost"
                          onClick={() => setProviderEditor({ provider: null })}
                        >
                          <Icon name="plus" className="ico" />
                          手动添加
                        </button>
                      </div>

                      {providers.length === 0 ? (
                        /* 首次启动：没有任何提供商，给出明确的下一步 */
                        <div className="panel-empty">
                          <Icon name="cpu" size={20} />
                          <div>尚未添加提供商</div>
                          <div className="panel-empty-hint">
                            从模型目录选择提供商，或手动配置 Ollama、公司网关等本地端点。
                          </div>
                        </div>
                      ) : null}

                      {providers.map((provider) => (
                        <div key={provider.id} className={cn("provider-card", expanded === provider.id && "is-open")}>
                          <div className="provider-head">
                            {/* 提供商摘要：名称、可用状态与操作按钮 */}
                            <div className="provider-logo">{provider.name.slice(0, 2)}</div>
                            <div className="provider-main">
                              <div className="provider-name">
                                {provider.name}
                                <span className={cn("badge", provider.available ? "ok" : "neutral")}>
                                  {provider.available ? "可用" : "未配置"}
                                </span>
                              </div>
                              <div className="provider-sub">{provider.endpoint || provider.kind}</div>
                            </div>
                            <div className="provider-actions">
                              <button
                                type="button"
                                className="icon-btn sm"
                                title="编辑提供商"
                                aria-label={`编辑 ${provider.name}`}
                                onClick={() => setProviderEditor({ provider })}
                              >
                                <Icon name="edit" />
                              </button>
                              <button
                                type="button"
                                className="icon-btn sm"
                                title="删除提供商"
                                aria-label={`删除 ${provider.name}`}
                                onClick={() => void deleteProvider(provider)}
                              >
                                <Icon name="trash" />
                              </button>
                              <button
                                type="button"
                                className="btn sm ghost"
                                disabled={testing === provider.id}
                                onClick={() => void testProvider(provider)}
                              >
                                {testing === provider.id ? (
                                  <Icon name="loader" className="spin" />
                                ) : (
                                  <Icon name="zap" />
                                )}
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
                            /* 展开内容：密钥来源与设置入口，以及该提供商下各模型的能力表 */
                            <div className="provider-body">
                              <div className="key-line">
                                <Icon name="key" className="ico" />
                                <span>
                                  密钥来源：
                                  {provider.keySource === "file"
                                    ? "keys.json"
                                    : provider.keySource === "env"
                                      ? "环境变量"
                                      : "未配置"}
                                </span>
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
                                    <th>最大输出</th>
                                    <th>价格（输入/输出）</th>
                                    <th>工具</th>
                                    <th>推理</th>
                                    <th>图片</th>
                                    <th>状态</th>
                                  </tr>
                                </thead>
                                <tbody>
                                  {provider.models.map((model) => (
                                    <tr key={model.ref}>
                                      <td>{model.name}</td>
                                      <td>{model.contextK}k</td>
                                      <td>{model.maxTokens ? model.maxTokens : "默认"}</td>
                                      <td>
                                        {model.cost
                                          ? `$${model.cost.inputPerMTok} / $${model.cost.outputPerMTok}`
                                          : "—"}
                                      </td>
                                      <Cap yes={model.caps.tools} />
                                      <Cap yes={model.caps.reasoning} />
                                      <Cap yes={model.caps.vision} />
                                      <td>{model.status || "—"}</td>
                                    </tr>
                                  ))}
                                </tbody>
                              </table>
                            </div>
                          ) : null}
                        </div>
                      ))}

                      {catalogMeta ? (
                        <div className="sec-desc provider-catalog-foot">
                          模型目录：models.dev · {catalogMeta.generatedAt.slice(0, 10)} · {catalogMeta.providerCount}{" "}
                          个提供商 · {catalogMeta.modelCount} 个模型 · {catalogMeta.license}
                        </div>
                      ) : null}
                    </>
                  )}
                </SettingsSection>
              ) : null}

              {tab === "tools" ? (
                /* 工具与权限分组：审批档位、风险等级说明与当前限制 */
                <SettingsSection title="工具与权限" desc="审批档位与风险分级共同决定一次工具调用是否执行。">
                  <Row label="审批档位" desc="与输入框底部的盾牌按钮双向同步。">
                    {/* 四个档位单选：分级、严格、只读、自由；自由档位切换前需确认 */}
                    <div className="radio-cards">
                      {approvalOptions.map((option) => (
                        <button
                          key={option.id}
                          type="button"
                          className={cn("radio-card", settings.approvalLevel === option.id && "is-on")}
                          onClick={() => void changeApprovalLevel(option.id)}
                          title={option.description}
                        >
                          <Icon name={option.icon} className="ico" />
                          {option.label}
                        </button>
                      ))}
                    </div>
                  </Row>
                  <Row
                    label="单次运行最大往返"
                    desc="Agent 一次运行的模型调用轮数上限，超过后停止本次运行；「不限制」适合长任务，但请在场观察，避免模型空转消耗。改动对下一次运行生效。"
                  >
                    <select
                      className="select"
                      value={settings.maxIterations}
                      onChange={(event) => update({ maxIterations: Number(event.target.value) })}
                    >
                      {[24, 60, 120, 300, 600, 0].map((limit) => (
                        <option key={limit} value={limit}>
                          {limit === 0 ? "不限制" : `${limit} 轮`}
                        </option>
                      ))}
                    </select>
                  </Row>
                  <div className="set-row" style={{ display: "block" }}>
                    {/* 风险等级表：说明 R0–R4 的判定与默认策略 */}
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
                  <Row label="沙箱" desc="macOS Seatbelt 与 Windows 受限令牌尚未实现；当前依赖审批与路径校验。">
                    <span className="badge warn">未启用</span>
                  </Row>
                </SettingsSection>
              ) : null}

              {tab === "mcp" ? (
                /* MCP 分组：已配置服务器、操作按钮与模板提示 */
                <SettingsSection
                  title="MCP 服务器"
                  desc="本地 stdio 服务器；工具以 mcp__服务器__工具 命名，调用前走审批。"
                >
                  <div className="mcp-toolbar">
                    {/* 添加服务器需要依次输入名称与启动命令；刷新重新读取列表 */}
                    <button
                      type="button"
                      className="btn sm"
                      disabled={mcpBusy === "add"}
                      onClick={() => void addMCPServer()}
                    >
                      {mcpBusy === "add" ? (
                        <Icon name="loader" className="ico spin" />
                      ) : (
                        <Icon name="plus" className="ico" />
                      )}
                      添加服务器
                    </button>
                    <button type="button" className="btn sm ghost" onClick={() => void refreshMCP()}>
                      刷新
                    </button>
                  </div>

                  {mcpServers.length === 0 ? (
                    <div className="panel-empty" style={{ padding: "18px 0 22px" }}>
                      <Icon name="globe" size={22} />
                      <div>还没有已连接的 MCP 服务器</div>
                      <div className="panel-empty-hint">添加本地命令后，服务器工具会在下一次对话中可用。</div>
                    </div>
                  ) : null}

                  {mcpServers.map((server) => (
                    /* 每个服务器卡片：连接状态、启动命令、错误信息、工具清单与操作 */
                    <div key={server.id} className="mcp-card">
                      <div className="mcp-head">
                        <div className="mcp-icon">
                          <Icon name="globe" />
                        </div>
                        <div className="mcp-main">
                          <div className="mcp-name">
                            {server.name}
                            <span
                              className={cn("badge", server.connected ? "ok" : server.error ? "danger" : "neutral")}
                            >
                              {server.connected ? "已连接" : server.error ? "连接失败" : "未连接"}
                            </span>
                          </div>
                          <div className="mcp-sub">{[server.command, ...(server.args ?? [])].join(" ")}</div>
                          {server.error ? (
                            <div className="mcp-sub" style={{ color: "var(--danger)", whiteSpace: "normal" }}>
                              {server.error}
                            </div>
                          ) : null}
                        </div>
                        <div className="provider-actions">
                          {server.connected ? (
                            <button
                              type="button"
                              className="btn sm ghost"
                              disabled={mcpBusy === server.id}
                              onClick={() => void disconnectMCP(server)}
                            >
                              断开
                            </button>
                          ) : (
                            <button
                              type="button"
                              className="btn sm"
                              disabled={mcpBusy === server.id}
                              onClick={() => void connectMCP(server)}
                            >
                              连接
                            </button>
                          )}
                          <button
                            type="button"
                            className="icon-btn sm"
                            aria-label={`移除 ${server.name}`}
                            onClick={() => void removeMCP(server)}
                          >
                            <Icon name="trash" />
                          </button>
                        </div>
                      </div>
                      {(server.tools ?? []).length > 0 ? (
                        <div className="mcp-tools">
                          <div className="mcp-tool-chips">
                            {(server.tools ?? []).map((tool) => (
                              <span key={tool.name} className="mcp-tool-chip" title={tool.description}>
                                {tool.name}
                              </span>
                            ))}
                          </div>
                        </div>
                      ) : null}
                    </div>
                  ))}

                  <div className="sec-desc" style={{ marginTop: 10 }}>
                    服务器以当前用户权限运行；模型密钥与常见令牌会从环境变量中过滤。
                  </div>

                  {mcpTemplates.map((template) => (
                    /* 模板只是说明性示例，不提供一键添加 */
                    <div key={template.name} className="mcp-card">
                      <div className="provider-head" style={{ padding: 0 }}>
                        <div className="provider-logo">
                          <Icon name={template.icon} />
                        </div>
                        <div className="provider-main">
                          <div className="provider-name">{template.name}</div>
                          <div className="provider-sub">{template.description}</div>
                        </div>
                        <span className="badge neutral">模板</span>
                      </div>
                    </div>
                  ))}
                </SettingsSection>
              ) : null}

              {tab === "appearance" ? (
                /* 外观分组：主题与字号；未接线的控件保持禁用并标注里程碑 */
                <SettingsSection title="外观" desc="主题与字体只影响本地界面。">
                  <Row label="主题" desc="跟随系统、深色或浅色；代码主题独立。">
                    {/* 主题卡片：用缩略块预览侧栏、主区与强调色的比例；跟随系统时左右各示一半 */}
                    <div className="theme-cards">
                      {(["system", "dark", "light"] as const).map((theme) => (
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
                            {theme === "system" ? "跟随系统" : theme === "dark" ? "深色" : "浅色"}
                            <Icon name={theme === "system" ? "monitor" : theme === "dark" ? "moon" : "sun"} />
                          </div>
                        </button>
                      ))}
                    </div>
                  </Row>
                  <Row
                    label="界面字号"
                    desc="界面基准字号，影响消息、侧栏、面板与浮层；终端、编辑器与代码块使用独立字号。"
                  >
                    <select
                      className="select"
                      value={settings.fontSize}
                      onChange={(event) => update({ fontSize: Number(event.target.value) })}
                    >
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
                /* 高级分组：版本信息、备份入口、导出留桩、重置设置与许可证 */
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
                    {/* 重置前二次确认；确认后写回 DEFAULT_SETTINGS */}
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
        </div>
      </div>
    </div>,
    document.body,
  );
}

/**
 * 设置分组的通用外壳。
 *
 * @param title 分组标题；
 * @param desc 分组说明；
 * @param children 分组内的设置项。
 * 每个分组都用它包裹，保证标题、说明与间距一致。
 */
function SettingsSection({ title, desc, children }: { title: string; desc: string; children: React.ReactNode }) {
  return (
    <section className="settings-section">
      <div className="settings-section-title">{title}</div>
      <div className="sec-desc">{desc}</div>
      {children}
    </section>
  );
}

/**
 * 单条设置项。
 *
 * @param label 设置名称；
 * @param desc 可选的补充说明；
 * @param children 右侧的控件。
 */
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

/**
 * 开关控件。
 *
 * @param on 当前是否开启；
 * @param onChange 状态变化回调，参数是切换后的值；
 * @param disabled 为 true 时点击不改变状态。
 */
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

/**
 * 模型能力表格里的单元格。
 *
 * @param yes 是否支持该项能力；支持显示对勾，不支持显示短横线。
 */
function Cap({ yes }: { yes: boolean }) {
  return (
    <td>
      <span className={cn("cap-pill", yes ? "y" : "n")}>{yes ? "✓" : "—"}</span>
    </td>
  );
}
