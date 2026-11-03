// 设置中心：920×660 模态，左侧七组导航 + 分组搜索 + 数据入口，右侧分组
// 表单。所有设置改动经 saveSettings 立即保存并生效（主题 / 字号经外观
// 钩子重建主题；审批档位 / 往返上限 / 失败转移模型由服务实时接管）。
//
// 分组内容对齐 React 版 SettingsModal：通用、模式、模型与提供商（提供商
// 卡片 / 密钥 / 能力表 / 测试连接 / 目录与手动添加）、工具与权限（四档 +
// 自由确认门 + R0–R4 表）、MCP、外观（主题卡 + 字号）、高级。
//
// 维护留档（review 判定，本轮不拆）：本文件已过千行且所有分组共用同一
// 状态与保存链路（Divergent Change 的早期形态）。再往任一分组加内容的
// 时候，按「一分组一文件」拆分——drawSettings 的分组分派、settingsState
// 与 saveSettings 留在本文件，models / tools / mcp / appearance /
// advanced 等分组的表单各成一文件（同包内拆，无接口变化）。不在本轮
// 拆是因为 review 修复轮不应夹带大文件重构，风险大于收益。

package overlays

import (
	"encoding/base64"
	"fmt"
	"strings"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/policy"
	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// settingsState 是设置中心的界面状态。
type settingsState struct {
	// open 是设置中心是否打开。
	open bool
	// tab 是当前分组；打开时不传 tab 则保持上次停留位置。
	tab SettingsTab
	// query 是分组搜索关键字（只过滤分组名，不深入设置项）。
	query string
	// draft 是表单绑定的设置草稿：打开时与保存成功后从服务同步。
	draft service.Settings
	// loadedTab 记录上帧分组，用于检测分组切换（MCP 列表进组刷新）。
	loadedTab SettingsTab

	// providers 是提供商列表，打开与变更后刷新。
	providers []service.ProviderView
	// expanded 是展开卡片对应的提供商 ID，空表示全部收起。
	expanded string
	// testing 是正在测试连接的提供商 ID，空表示没有进行中的测试。
	testing string

	// mcpServers 是 MCP 服务器列表（进入 MCP 分组时刷新）。
	mcpServers []service.MCPServerView
	// mcpBusy 是正在处理的请求标记："add" 或服务器 ID。
	mcpBusy string

	// catalogOpen 是目录浏览器是否打开（替换提供商列表视图）。
	catalogOpen bool
	// catalog 是目录浏览器的界面状态。
	catalog catalogState
	// editor 是提供商编辑器状态；editor.open 时替换整个列表视图。
	editor editorState
}

// settingsGroup 是左侧导航的一个分组定义。
type settingsGroup struct {
	id    SettingsTab
	label string
	icon  string
}

// groupsAlign 对齐 React 版 groups 的七组定义。
var groupsAlign = []settingsGroup{
	{TabGeneral, "通用", "settings"},
	{TabMode, "模式", "layers"},
	{TabModels, "模型与提供商", "cpu"},
	{TabTools, "工具与权限", "shield"},
	{TabMCP, "MCP 服务器", "globe"},
	{TabAppearance, "外观", "sun"},
	{TabAdvanced, "高级", "sliders"},
}

// approvalOptions 是四种审批档位的展示信息。
type approvalOption struct {
	level, label, desc, icon string
}

var approvalOptions = []approvalOption{
	{"graded", "分级", "R0 自动；R1–R3 询问；R4 阻止", "shield"},
	{"strict", "严格", "写入与执行逐次询问，不提供永久允许", "alert"},
	{"readonly", "只读", "只允许只读操作", "lock"},
	{"free", "自由", "放行所有操作包括 R4；切换前需要确认", "zap"},
}

// riskRows 是 R0–R4 的判定与默认策略说明。
type riskRow struct {
	level, name, policy string
}

var riskRows = []riskRow{
	{"R0", "只读", "自动允许，记录审计"},
	{"R1", "写工作区", "每次询问，可本会话 / 本工作区授权"},
	{"R2", "执行", "每次询问，可按命令前缀授权"},
	{"R3", "网络或越界", "每次询问，不提供永久允许"},
	{"R4", "破坏性", "阻止，双重确认也不放行"},
}

// mcpTemplates 是 MCP 服务器的展示模板（仅说明，不提供一键添加）。
type mcpTemplate struct {
	name, desc, icon string
}

var mcpTemplates = []mcpTemplate{
	{"Gmail", "只读邮件、生成应用内草稿", "mail"},
	{"Google Calendar", "只读日历、生成日程草稿", "calendar"},
	{"PostgreSQL", "只读查询与表结构浏览", "database"},
}

// logoEntry 是目录 logo 的加载状态：成功存位图，失败进负缓存。
type logoEntry struct {
	bmp     *mygo.Bitmap
	loading bool
	failed  bool
}

// openSettings 打开设置中心并同步设置草稿与提供商列表。
func (s *Store) openSettings(tab SettingsTab) {
	st := &s.settings
	st.open = true
	if tab != "" {
		st.tab = tab
	}
	st.query = ""
	st.draft = s.settingsSnapshot()
	st.providers = s.providerList()
	st.editor = editorState{}
	st.catalogOpen = false
	st.catalog = catalogState{}
}

// providerList 读取提供商列表；服务未注入时返回空。
func (s *Store) providerList() []service.ProviderView {
	if s.be == nil || s.be.Providers == nil {
		return nil
	}
	return s.be.Providers.List()
}

// refreshMCP 读取 MCP 服务器列表；失败弹通知但不阻塞界面。
func (s *Store) refreshMCP() {
	if s.be == nil || s.be.MCP == nil {
		return
	}
	s.settings.mcpServers = s.be.MCP.List()
}

// save 保存设置片段。次序对齐 React 版 saveSettings：先改草稿并立即应用
// 外观（保存失败时保留本地观感），再尝试持久化；成功后用保存值刷新草稿，
// 保证界面显示的是真正写入数据库的内容。审批档位 / 往返上限 / 失败转移
// 模型等由服务在 Save 时经实时 resolver 接管，无需这里额外处理。
func (s *Store) save(c *mygo.Context, patch func(*service.Settings)) {
	st := &s.settings
	patch(&st.draft)
	theme.SetAppearance(st.draft.Theme, st.draft.FontSize)
	if s.be == nil || s.be.Settings == nil {
		return
	}
	next := s.be.Settings.Get()
	patch(&next)
	saved, err := s.be.Settings.Save(next)
	if err != nil {
		Notify("保存设置失败", err.Error(), ToneDanger)
		return
	}
	st.draft = saved
}

// drawSettings 画设置中心模态。
func (s *Store) drawSettings(c *mygo.Context) {
	st := &s.settings
	if !st.open {
		return
	}
	p := pal(c)
	w, h := c.Size()

	// 进入 MCP 分组时刷新服务器列表（列表读取是内存缓存，主线程安全）。
	if st.tab == TabMCP && st.loadedTab != TabMCP {
		s.refreshMCP()
	}
	st.loadedTab = st.tab

	panel := mygo.DialogBase(c, &st.open, func(backdrop, panel mygo.Element) {
		styleBackdrop(backdrop)
		panel.Width(920).MaxWidth(w-48).Height(660).MaxHeight(h-48).
			Radius(14).Background(p.BgPanel).Border(1, p.BorderStrong).
			Shadow(p.ShadowLg.X, p.ShadowLg.Y, p.ShadowLg.Blur, p.ShadowLg.Spread, p.ShadowLg.Color)
		// esc / 遮罩关闭由 DialogBase 处理；关闭即收起子视图，下次打开不残留。
		panel.Gap(0).Children(func() {
			s.settingsHead(c, st)
			mygo.Row(c).Grow(1).AlignItems(mygo.Stretch).Children(func() {
				s.settingsNav(c, st)
				s.settingsContent(c, st)
			})
		})
	})
	if panel.Dismissed() {
		st.open = false
		st.catalogOpen = false
		st.editor = editorState{}
	}
}

// settingsHead 画顶栏：标题、分组搜索与关闭按钮。
func (s *Store) settingsHead(c *mygo.Context, st *settingsState) {
	p := pal(c)
	mygo.Row(c).Height(48).Padding(0, 14, 0, 18).
		AlignItems(mygo.Center).Gap(10).
		BorderWidth(0, 0, 1, 0).BorderColor(p.Border).Children(func() {
		icons.Icon(c, "settings").Size(16, 16).TextColor(p.Text2)
		mygo.Text(c, "设置").FontSize(theme.Fonts.XL()).Bold()
		mygo.SearchField(c, &st.query).Width(220).Height(28).
			Label("设置搜索").Placeholder("搜索设置…")
		mygo.Spacer(c)
		if iconBtn(c, "x", "关闭设置").Clicked() {
			st.open = false
			st.catalogOpen = false
			st.editor = editorState{}
		}
	})
}

// settingsNav 画左侧导航：搜索过滤后的七组 + 数据面板三个入口。
func (s *Store) settingsNav(c *mygo.Context, st *settingsState) {
	p := pal(c)
	query := strings.ToLower(strings.TrimSpace(st.query))
	visible := groupsAlign[:0]
	for _, g := range groupsAlign {
		if query == "" || strings.Contains(strings.ToLower(g.label), query) {
			visible = append(visible, g)
		}
	}

	nav := mygo.Column(c).Width(208).FillHeight().
		Padding(theme.Space6, theme.Space4).Gap(theme.Space1).
		Background(p.SidebarBg).BorderWidth(0, 1, 0, 0).BorderColor(p.Border)
	nav.Children(func() {
		navGroupLabel(c, "设置")
		if len(visible) == 0 {
			mygo.Text(c, "没有匹配的设置分组").FontSize(theme.Fonts.SM()).TextColor(p.Text4).Padding(theme.Space4, theme.Space5)
		}
		for _, g := range visible {
			active := st.tab == g.id
			item := mygo.ButtonBase(c)
			item.Fill().Height(theme.HeightSideLink).Radius(theme.RadiusRow).
				Padding(0, theme.Space5).AlignItems(mygo.Center).Gap(theme.Space5)
			if active {
				item.Background(p.BgActive)
			}
			item.Children(func() {
				icons.Icon(c, g.icon).Size(15, 15).TextColor(p.Text2)
				mygo.Text(c, g.label).TextColor(p.Text)
			})
			if item.Clicked() {
				st.tab = g.id
			}
		}
		navGroupLabel(c, "数据")
		dataEntries := []struct {
			label, icon string
			tab         DataTab
		}{
			{"用量统计", "zap", TabUsage},
			{"审计记录", "shield-check", TabAudit},
			{"备份与恢复", "archive", TabBackup},
		}
		for _, e := range dataEntries {
			item := mygo.ButtonBase(c)
			item.Fill().Height(theme.HeightSideLink).Radius(theme.RadiusRow).
				Padding(0, theme.Space5).AlignItems(mygo.Center).Gap(theme.Space5)
			item.Children(func() {
				icons.Icon(c, e.icon).Size(15, 15).TextColor(p.Text2)
				mygo.Text(c, e.label).TextColor(p.Text)
			})
			if item.Clicked() {
				// 数据面板压在设置中心之上；设置保持打开（对齐 React）。
				OpenData(e.tab)
			}
		}
	})
}

// navGroupLabel 是导航里的小节标题。
func navGroupLabel(c *mygo.Context, text string) {
	mygo.Text(c, text).FontSize(theme.Fonts.XS()).TextColor(pal(c).Text3).
		Padding(theme.Space5, theme.Space5, theme.Space2, theme.Space5)
}

// ---------------------------------------------------------------------------
// 右侧内容区
// ---------------------------------------------------------------------------

// settingsContent 画当前分组的表单；目录浏览器与提供商编辑器打开时替换
// 整个内容区（对齐 React 版的 is-catalog / editor 分支）。
func (s *Store) settingsContent(c *mygo.Context, st *settingsState) {
	p := pal(c)
	content := mygo.Scroll(c).Grow(1).Padding(theme.Space8, theme.Space8)
	content.Children(func() {
		defer func() {
			if r := recover(); r != nil {
				println("SMOKE-DEBUG: settingsContent panic:", fmt.Sprint(r))
			}
		}()
		switch {
		case st.tab == TabModels && st.catalogOpen:
			s.drawCatalog(c, st)
		case st.tab == TabModels && st.editor.open:
			s.drawProviderEditor(c, st)
		default:
			switch st.tab {
			case TabGeneral:
				s.tabGeneral(c, st)
			case TabMode:
				s.tabMode(c, st)
			case TabModels:
				s.tabModels(c, st)
			case TabTools:
				s.tabTools(c, st)
			case TabMCP:
				s.tabMCP(c, st)
			case TabAppearance:
				s.tabAppearance(c, st)
			case TabAdvanced:
				s.tabAdvanced(c, st)
			}
		}
	})
	_ = p
}

// tabGeneral 画通用分组：启动模式、更新偏好、系统通知与数据目录。
func (s *Store) tabGeneral(c *mygo.Context, st *settingsState) {
	settingsSection(c, "通用", "启动行为、语言与本地数据。", func() {
		setRow(c, "界面语言", "当前版本固定为简体中文；完整国际化需要消息编号表。", func() {
			enumSelect(c, &st.draft.Language, 200, []selectOption[string]{{"zh-CN", "简体中文"}}, nil).Disabled(true)
		})
		setRow(c, "当前模式", "切换后立即生效，并作为下次启动的默认模式。", func() {
			enumSelect(c, &st.draft.Mode, 200, []selectOption[string]{
				{"code", "Code 模式"},
				{"work", "Work 模式"},
			}, func() {
				s.save(c, func(x *service.Settings) { x.Mode = st.draft.Mode })
			})
		})
		setRow(c, "更新渠道", "自动更新在 M4 接入；当前只保存偏好。", func() {
			enumSelect(c, &st.draft.UpdateChannel, 200, []selectOption[string]{
				{"stable", "稳定版"},
				{"beta", "测试版"},
			}, func() {
				s.save(c, func(x *service.Settings) { x.UpdateChannel = st.draft.UpdateChannel })
			})
		})
		s.switchRow(c, st, "自动检查更新", "仅检查签名清单，不上传任何数据。", &st.draft.AutoUpdate)
		s.switchRow(c, st, "系统通知", "审批等待、回复完成或运行失败时发送系统通知；窗口聚焦时不打扰。", &st.draft.Notifications)
		s.switchRow(c, st, "遥测", "产品不收集遥测；此项固定关闭。", &st.draft.Telemetry)
		setRow(c, "数据目录", "会话、密钥、日志与备份都保存在本机。", func() {
			mygo.Text(c, s.dataDir()).FontSize(theme.Fonts.SM()).TextColor(pal(c).Text2)
		})
	})
}

// switchRow 画一行开关设置：开关绑定草稿字段，变更立即保存。
func (s *Store) switchRow(c *mygo.Context, st *settingsState, label, desc string, target *bool) {
	setRow(c, label, desc, func() {
		sw := mygo.Switch(c, target)
		if sw.Changed() {
			s.save(c, func(x *service.Settings) { *x = st.draft })
		}
	})
}

// dataDir 返回数据目录展示文案。
func (s *Store) dataDir() string {
	if s.be != nil && s.be.App != nil {
		return s.be.App.Info().DataDir
	}
	return "~/.craftwork"
}

// tabMode 画模式分组：默认模式与两种模式的能力范围说明。
func (s *Store) tabMode(c *mygo.Context, st *settingsState) {
	settingsSection(c, "模式", "Code 与 Work 共用同一套工作区与审批策略，只改变界面和可用工具。", func() {
		setRow(c, "默认模式", "新建窗口与新建会话时使用。", func() {
			mygo.Row(c).Gap(theme.Space4).Children(func() {
				for _, mode := range []struct {
					id, label, icon string
				}{{"code", "Code", "code"}, {"work", "Work", "briefcase"}} {
					on := st.draft.Mode == mode.id
					card := mygo.ButtonBase(c)
					card.Height(38).Padding(0, theme.Space7).Radius(theme.RadiusBtn).
						AlignItems(mygo.Center).Gap(theme.Space4).
						Background(pal(c).BgInput).
						BorderWidth(1).BorderColor(pal(c).Border)
					if on {
						card.BorderColor(pal(c).AccentLine).Background(pal(c).AccentSoft)
					}
					card.Children(func() {
						icons.Icon(c, mode.icon).Size(15, 15).TextColor(pal(c).Text2)
						mygo.Text(c, mode.label)
						if on {
							mygo.Text(c, mode.label).TextColor(pal(c).Accent)
						}
					})
					if card.Clicked() && !on {
						s.save(c, func(x *service.Settings) { x.Mode = mode.id })
					}
				}
			})
		})
		setRow(c, "Code 模式", "文件树、代码编辑、差异、终端、Shell 与 Git。", func() {
			badge(c, "17 个内置工具", pal(c).Accent, pal(c).AccentSoft)
		})
		setRow(c, "Work 模式", "文档预览、Word / Excel、PDF、邮件与日历 MCP。", func() {
			badge(c, "文档 · 文件 · MCP", pal(c).WorkMode.Accent, pal(c).WorkMode.Soft)
		})
	})
}

// tabModels 画模型与提供商分组：默认模型选择、提供商卡片与入口。
func (s *Store) tabModels(c *mygo.Context, st *settingsState) {
	meta := s.catalogMeta()
	settingsSection(c, "模型与提供商", "提供商全部由你添加；密钥只保存在本地，切换模型只影响后续 run。", func() {
		// 默认模型与失败转移模型
		defaultOptions, fallbackOptions := s.modelOptions(st)
		setRow(c, "默认模型", "新会话使用的模型；没有添加提供商时保持为空。", func() {
			enumSelect(c, &st.draft.DefaultModel, 280, defaultOptions, func() {
				s.save(c, func(x *service.Settings) { x.DefaultModel = st.draft.DefaultModel })
			})
		})
		setRow(c, "失败转移模型", "主模型遇到限流、5xx、超时或连接失败时自动切换；工具执行过程中不切换。", func() {
			enumSelect(c, &st.draft.FallbackModel, 280, fallbackOptions, func() {
				s.save(c, func(x *service.Settings) { x.FallbackModel = st.draft.FallbackModel })
			})
		})

		// 工具行：目录元信息 + 两个入口
		mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space4).Margin(0, 0, theme.Space4, 0).Children(func() {
			if meta != nil {
				mutedText(c, fmt.Sprintf("模型目录：models.dev · %s · %d 个提供商 · %d 个模型 · %s",
					clipDate(meta.GeneratedAt), meta.ProviderCount, meta.ModelCount, meta.License)).Grow(1)
			}
			if smallBtn(c, "从模型目录添加").Clicked() {
				st.catalogOpen = true
			}
			if smallGhostBtn(c, "手动添加").Clicked() {
				st.editor = newEditorState(nil)
			}
		})

		if len(st.providers) == 0 {
			emptyState(c, "cpu", "尚未添加提供商", "从模型目录选择提供商，或手动配置 Ollama、公司网关等本地端点。")
			return
		}
		for i := range st.providers {
			s.providerCard(c, st, &st.providers[i])
		}
	})
}

// modelOptions 组装默认模型与失败转移模型的下拉选项。
func (s *Store) modelOptions(st *settingsState) ([]selectOption[string], []selectOption[string]) {
	defaults := []selectOption[string]{{"", "未选择模型"}}
	fallbacks := []selectOption[string]{{"", "不自动转移"}}
	for i := range st.providers {
		prov := &st.providers[i]
		for _, m := range prov.Models {
			label := prov.Name + " · " + m.Name
			if !m.Available {
				label += "（未配置密钥）"
			}
			defaults = append(defaults, selectOption[string]{m.Ref, label})
			if m.Ref != st.draft.DefaultModel {
				fallbacks = append(fallbacks, selectOption[string]{m.Ref, label})
			}
		}
	}
	return defaults, fallbacks
}

// clipDate 取 ISO 日期的前 10 位（YYYY-MM-DD）。
func clipDate(s string) string {
	if len(s) >= 10 {
		return s[:10]
	}
	return s
}

// ---------------------------------------------------------------------------
// 提供商卡片（模型分组）
// ---------------------------------------------------------------------------

// providerCard 画一张提供商卡片：摘要行 + 展开后的密钥行与能力表。
func (s *Store) providerCard(c *mygo.Context, st *settingsState, prov *service.ProviderView) {
	p := pal(c)
	open := st.expanded == prov.ID
	mygo.Column(c).Fill().Gap(0).
		Margin(0, 0, theme.Space5, 0).
		Radius(theme.RadiusCard).Background(p.BgElevated).
		BorderWidth(1).BorderColor(p.Border).Children(func() {
		// 摘要行：logo、名称与可用徽章、端点、操作按钮。
		mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space5).
			Padding(theme.Space5, theme.Space6).Children(func() {
			s.providerLogo(c, prov.CatalogID, prov.Name, 28)
			mygo.Column(c).Grow(1).Gap(1).Children(func() {
				mygo.Row(c).AlignItems(mygo.Center).Gap(theme.Space4).Children(func() {
					mygo.Text(c, prov.Name)
					if prov.Available {
						badge(c, "可用", p.OK, p.OKSoft)
					} else {
						badge(c, "未配置", p.Text3, p.BgHover)
					}
				})
				sub := prov.Endpoint
				if sub == "" {
					sub = prov.Kind
				}
				mygo.Text(c, sub).FontSize(theme.Fonts.SM()).TextColor(p.Text3)
			})
			if iconBtn(c, "edit", "编辑 "+prov.Name).Clicked() {
				st.editor = newEditorState(prov)
			}
			if iconBtn(c, "trash", "删除 "+prov.Name).Clicked() {
				s.confirmDeleteProvider(c, st, prov)
			}
			testing := st.testing == prov.ID
			test := smallGhostBtn(c, "测试连接").Disabled(testing)
			if test.Clicked() {
				s.testProvider(c, prov.ID)
			}
			chev := "chevron-down"
			if open {
				chev = "chevron-up"
			}
			if iconBtn(c, chev, "展开提供商").Clicked() {
				if open {
					st.expanded = ""
				} else {
					st.expanded = prov.ID
				}
			}
		})
		if open {
			// 密钥行：来源、掩码、设置与清除。
			mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space4).
				Padding(theme.Space4, theme.Space6).
				BorderWidth(0, 0, 1, 0).BorderColor(p.BorderSoft).Children(func() {
				icons.Icon(c, "key").Size(14, 14).TextColor(p.Text3)
				source := "未配置"
				switch prov.KeySource {
				case "file":
					source = "keys.json"
				case "env":
					source = "环境变量"
				}
				mygo.Text(c, "密钥来源："+source).FontSize(theme.Fonts.SM()).TextColor(p.Text2)
				mygo.Spacer(c)
				mask := prov.KeyMasked
				if mask == "" {
					mask = "—"
				}
				mygo.Text(c, mask).FontSize(theme.Fonts.SM()).TextColor(p.Text4)
				if smallBtn(c, "设置").Clicked() {
					s.promptSetKey(c, st, prov)
				}
				if prov.KeySource != "" {
					if smallGhostBtn(c, "清除").Clicked() {
						s.confirmClearKey(c, st, prov)
					}
				}
			})
			// 模型能力表。
			s.capabilityTable(c, prov).Margin(theme.Space4, theme.Space6, theme.Space6, theme.Space6)
		}
	})
}

// capabilityTable 画提供商下各模型的能力表（ui.Table）。
func (s *Store) capabilityTable(c *mygo.Context, prov *service.ProviderView) mygo.Element {
	p := pal(c)
	cols := []mygo.TableColumn{
		{Title: "模型"},
		{Title: "上下文", Width: 70, Align: mygo.End},
		{Title: "最大输出", Width: 80, Align: mygo.End},
		{Title: "价格 输入/输出", Width: 130, Align: mygo.End},
		{Title: "工具", Width: 44, Align: mygo.Center},
		{Title: "推理", Width: 44, Align: mygo.Center},
		{Title: "图片", Width: 44, Align: mygo.Center},
		{Title: "状态", Width: 80},
	}
	n := len(prov.Models)
	return mygo.Table(c, nil, cols, n, func(row, col int) {
		m := prov.Models[row]
		p := pal(c)
		switch col {
		case 0:
			mygo.Column(c).Gap(1).Children(func() {
				mygo.Text(c, m.Name).SingleLine()
				mygo.Text(c, m.ID).FontSize(theme.Fonts.XS()).TextColor(p.Text4).SingleLine()
			})
		case 1:
			mygo.Textf(c, "%dk", m.ContextK).AlignSelf(mygo.End)
		case 2:
			v := "默认"
			if m.MaxTokens > 0 {
				v = fmt.Sprint(m.MaxTokens)
			}
			mygo.Text(c, v).AlignSelf(mygo.End).TextColor(p.Text2)
		case 3:
			v := "—"
			if m.Cost != nil {
				v = fmt.Sprintf("$%g / $%g", m.Cost.InputPerMTok, m.Cost.OutputPerMTok)
			}
			mygo.Text(c, v).AlignSelf(mygo.End).TextColor(p.Text2)
		case 4, 5, 6:
			yes := m.Caps.Tools
			if col == 5 {
				yes = m.Caps.Reasoning
			}
			if col == 6 {
				yes = m.Caps.Vision
			}
			cell := mygo.Box(c).FillHeight().Center()
			if yes {
				cell.Children(func() { icons.Icon(c, "check").Size(13, 13).TextColor(p.OK) })
			} else {
				cell.Children(func() { mygo.Text(c, "—").TextColor(p.Text4) })
			}
		case 7:
			v := m.Status
			if v == "" {
				v = "—"
			}
			mygo.Text(c, v).TextColor(p.Text3)
		}
	}).BorderWidth(1).BorderColor(p.BorderSoft).Radius(theme.RadiusRow).Height(rowTableHeight(n))
}

// rowTableHeight 估一个能力表的高度：表头 + 行高（内容自适应的兜底）。
func rowTableHeight(n int) float32 {
	if n == 0 {
		return 34
	}
	return float32(34 + n*40)
}

// promptSetKey 弹输入框收密钥并保存（只写本地 keys.json）。
func (s *Store) promptSetKey(c *mygo.Context, st *settingsState, prov *service.ProviderView) {
	Prompt(c, PromptOpts{
		Title:       "设置 " + prov.Name + " 密钥",
		Label:       "密钥只写入本地 keys.json（0600），不会进入日志、数据库或对话。",
		Placeholder: "粘贴 API Key",
		ConfirmText: "保存密钥",
	}, func(value string, accepted bool) {
		if !accepted {
			return
		}
		if err := s.be.Providers.SetKey(prov.ID, value); err != nil {
			Notify("保存密钥失败", err.Error(), ToneDanger)
			return
		}
		st.providers = s.providerList()
		Notify("密钥已保存", prov.Name, ToneOK)
	})
}

// confirmClearKey 二次确认后清除密钥。
func (s *Store) confirmClearKey(c *mygo.Context, st *settingsState, prov *service.ProviderView) {
	Confirm(c, ConfirmOpts{
		Title:       "清除 " + prov.Name + " 密钥？",
		Description: "删除后该提供商的模型将不可用。",
		ConfirmText: "清除",
		Danger:      true,
	}, func(ok bool) {
		if !ok {
			return
		}
		if err := s.be.Providers.ClearKey(prov.ID); err != nil {
			Notify("清除密钥失败", err.Error(), ToneDanger)
			return
		}
		st.providers = s.providerList()
		Notify("密钥已清除", "", ToneWarn)
	})
}

// confirmDeleteProvider 二次确认后删除提供商。
func (s *Store) confirmDeleteProvider(c *mygo.Context, st *settingsState, prov *service.ProviderView) {
	Confirm(c, ConfirmOpts{
		Title:       "删除 " + prov.Name + "？",
		Description: "该提供商的模型会立即从列表中移除，API Key 同时删除。",
		ConfirmText: "删除",
		Danger:      true,
	}, func(ok bool) {
		if !ok {
			return
		}
		if err := s.be.Providers.DeleteProvider(prov.ID); err != nil {
			Notify("删除提供商失败", err.Error(), ToneDanger)
			return
		}
		st.providers = s.providerList()
		st.expanded = ""
		st.draft = s.settingsSnapshot()
		Notify("提供商已删除", "", ToneOK)
	})
}

// testProvider 在后台测试连接，结果以通知展示。
func (s *Store) testProvider(c *mygo.Context, providerID string) {
	s.settings.testing = providerID
	s.runAsync(func() func() {
		result, err := s.be.Providers.Test(providerID)
		return func() {
			s.settings.testing = ""
			switch {
			case err != nil:
				Notify("连接测试失败", err.Error(), ToneDanger)
			case result.OK:
				Notify("连接正常", fmt.Sprintf("%s · %d ms", result.Message, result.LatencyMs), ToneOK)
			default:
				Notify("连接失败", result.Message, ToneDanger)
			}
		}
	})
}

// providerLogo 画提供商 logo：目录 ID 非空时异步抓取 data URL 并解码为
// 位图（结果缓存）；离线、失败或 SVG 一律回退两位字符文字头像。
func (s *Store) providerLogo(c *mygo.Context, catalogID, name string, size float32) mygo.Element {
	p := pal(c)
	avatar := func() mygo.Element {
		box := mygo.Box(c).Width(size).Height(size).Radius(theme.RadiusCtl).Center().
			Background(p.BgHover)
		box.Children(func() {
			text := name
			if len(text) > 2 {
				text = text[:2]
			}
			mygo.Text(c, text).FontSize(theme.Fonts.XS()).TextColor(p.Text3)
		})
		return box
	}
	if s.be == nil || s.be.Providers == nil || catalogID == "" {
		return avatar()
	}
	if s.logos == nil {
		s.logos = map[string]logoEntry{}
	}
	entry, ok := s.logos[catalogID]
	if !ok {
		// 首次见到：标记加载中并发起后台抓取。
		s.logos[catalogID] = logoEntry{loading: true}
		s.runAsync(func() func() {
			dataURL, err := s.be.Providers.CatalogLogo(catalogID)
			var bmp *mygo.Bitmap
			if err == nil && strings.HasPrefix(dataURL, "data:image/") {
				if i := strings.Index(dataURL, "base64,"); i >= 0 {
					if data, derr := base64.StdEncoding.DecodeString(dataURL[i+len("base64,"):]); derr == nil {
						bmp, derr = mygo.DecodeBitmap(data)
						if derr != nil {
							bmp = nil
						}
					}
				}
			}
			return func() {
				e := s.logos[catalogID]
				e.loading = false
				e.failed = bmp == nil
				e.bmp = bmp
				s.logos[catalogID] = e
			}
		})
		return avatar()
	}
	if entry.bmp != nil {
		return mygo.Image(c, entry.bmp).Width(size).Height(size)
	}
	return avatar()
}

// catalogMeta 读取目录元信息（页脚展示）；读取失败返回 nil。
func (s *Store) catalogMeta() *service.CatalogMeta {
	if s.be == nil || s.be.Providers == nil {
		return nil
	}
	meta, err := s.be.Providers.CatalogMeta()
	if err != nil {
		return nil
	}
	return &meta
}

// ---------------------------------------------------------------------------
// 工具与权限 / MCP / 外观 / 高级
// ---------------------------------------------------------------------------

// tabTools 画工具与权限分组：审批档位（自由档带确认门）、往返上限与
// R0–R4 风险表。
func (s *Store) tabTools(c *mygo.Context, st *settingsState) {
	p := pal(c)
	settingsSection(c, "工具与权限", "审批档位与风险分级共同决定一次工具调用是否执行。", func() {
		setRow(c, "审批档位", "与输入框底部的盾牌按钮双向同步。", func() {
			mygo.Row(c).Gap(theme.Space4).Children(func() {
				for _, opt := range approvalOptions {
					on := st.draft.ApprovalLevel == opt.level
					card := mygo.ButtonBase(c)
					card.Height(38).Padding(0, theme.Space6).Radius(theme.RadiusBtn).
						AlignItems(mygo.Center).Gap(theme.Space4).
						Background(p.BgInput).BorderWidth(1).BorderColor(p.Border)
					if on {
						card.BorderColor(p.AccentLine).Background(p.AccentSoft)
					}
					card.Children(func() {
						icons.Icon(c, opt.icon).Size(14, 14).TextColor(p.Text2)
						mygo.Text(c, opt.label)
					})
					if card.Clicked() && !on {
						s.changeApprovalLevel(c, opt.level)
					}
				}
			})
		})
		setRow(c, "单次运行最大往返", "Agent 一次运行的模型调用轮数上限；「不限制」适合长任务，请在场观察。改动对下一次运行生效。", func() {
			enumSelect(c, &st.draft.MaxIterations, 160, []selectOption[int]{
				{24, "24 轮"},
				{60, "60 轮"},
				{120, "120 轮"},
				{300, "300 轮"},
				{600, "600 轮"},
				{0, "不限制"},
			}, func() {
				s.save(c, func(x *service.Settings) { x.MaxIterations = st.draft.MaxIterations })
			})
		})
		// R0–R4 风险等级表。
		mygo.Column(c).Fill().Gap(0).Margin(theme.Space4, 0, 0, 0).
			Radius(theme.RadiusRow).BorderWidth(1).BorderColor(p.BorderSoft).Children(func() {
			head := []string{"等级", "判定", "默认策略"}
			mygo.Row(c).Fill().Padding(theme.Space4, theme.Space6).Gap(theme.Space6).
				Background(p.BgSunken).BorderWidth(0, 0, 1, 0).BorderColor(p.BorderSoft).Children(func() {
				mygo.Box(c).Width(52).Children(func() { mygo.Text(c, head[0]).FontSize(theme.Fonts.SM()).TextColor(p.Text3) })
				mygo.Box(c).Width(110).Children(func() { mygo.Text(c, head[1]).FontSize(theme.Fonts.SM()).TextColor(p.Text3) })
				mygo.Text(c, head[2]).FontSize(theme.Fonts.SM()).TextColor(p.Text3).Grow(1)
			})
			for i, row := range riskRows {
				color := p.Risk(policyLevelRisk(row.level))
				mygo.Row(c).Fill().Padding(theme.Space4, theme.Space6).Gap(theme.Space6).Children(func() {
					mygo.Box(c).Width(52).Children(func() {
						badge(c, row.level, color, color.Alpha(0.13))
					})
					mygo.Box(c).Width(110).Children(func() { mygo.Text(c, row.name).FontSize(theme.Fonts.SM()) })
					mygo.Text(c, row.policy).FontSize(theme.Fonts.SM()).TextColor(p.Text2).Grow(1)
					_ = i
				})
				if i < len(riskRows)-1 {
					mygo.Row(c).Fill().Height(1).Background(p.BorderSoft)
				}
			}
		})
		setRow(c, "工作区授权", "选择「本会话 / 本工作区」后记录在这里；R3 / R4 永远不会出现。", func() {
			badge(c, "暂无永久授权", p.Text3, p.BgHover)
		})
		setRow(c, "沙箱", "macOS Seatbelt 与 Windows 受限令牌尚未实现；当前依赖审批与路径校验。", func() {
			badge(c, "未启用", p.Warn, p.WarnSoft)
		})
	})
}

// changeApprovalLevel 切换审批档位；切到自由档先过风险确认门。
func (s *Store) changeApprovalLevel(c *mygo.Context, level string) {
	st := &s.settings
	if st.draft.ApprovalLevel == level {
		return
	}
	if level == "free" {
		FreeConfirm(c, func(ok bool) {
			if ok {
				s.save(c, func(x *service.Settings) { x.ApprovalLevel = level })
			}
		})
		return
	}
	s.save(c, func(x *service.Settings) { x.ApprovalLevel = level })
}

// tabMCP 画 MCP 服务器分组：服务器卡片、连接操作与模板提示。
func (s *Store) tabMCP(c *mygo.Context, st *settingsState) {
	p := pal(c)
	settingsSection(c, "MCP 服务器", "本地 stdio 服务器；工具以 mcp__服务器__工具 命名，调用前走审批。", func() {
		mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space4).Children(func() {
			add := smallBtn(c, "添加服务器").Disabled(st.mcpBusy == "add")
			if add.Clicked() {
				s.addMCPServer(c, st)
			}
			if smallGhostBtn(c, "刷新").Clicked() {
				s.refreshMCP()
			}
		})
		if len(st.mcpServers) == 0 {
			emptyState(c, "globe", "还没有已连接的 MCP 服务器", "添加本地命令后，服务器工具会在下一次对话中可用。")
		}
		for i := range st.mcpServers {
			s.mcpCard(c, st, &st.mcpServers[i])
		}
		mutedText(c, "服务器以当前用户权限运行；模型密钥与常见令牌会从环境变量中过滤。").Margin(theme.Space5, 0, 0, 0)
		for _, tpl := range mcpTemplates {
			mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space5).
				Padding(theme.Space5, theme.Space6).Margin(theme.Space4, 0, 0, 0).
				Radius(theme.RadiusCard).Background(p.BgElevated).BorderWidth(1).BorderColor(p.Border).Children(func() {
				mygo.Box(c).Width(28).Height(28).Radius(theme.RadiusCtl).Center().Background(p.BgHover).Children(func() {
					icons.Icon(c, tpl.icon).Size(15, 15).TextColor(p.Text2)
				})
				mygo.Column(c).Grow(1).Gap(1).Children(func() {
					mygo.Text(c, tpl.name)
					mygo.Text(c, tpl.desc).FontSize(theme.Fonts.SM()).TextColor(p.Text3)
				})
				badge(c, "模板", p.Text3, p.BgHover)
			})
		}
	})
}

// mcpCard 画一张 MCP 服务器卡片：状态、命令、错误与工具清单。
func (s *Store) mcpCard(c *mygo.Context, st *settingsState, server *service.MCPServerView) {
	p := pal(c)
	status, color, soft := "未连接", p.Text3, p.BgHover
	switch {
	case server.Connected:
		status, color, soft = "已连接", p.OK, p.OKSoft
	case server.Error != "":
		status, color, soft = "连接失败", p.Danger, p.DangerSoft
	}
	mygo.Column(c).Fill().Gap(theme.Space4).Margin(theme.Space5, 0, 0, 0).
		Radius(theme.RadiusCard).Background(p.BgElevated).BorderWidth(1).BorderColor(p.Border).Children(func() {
		mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space5).Padding(theme.Space5, theme.Space6).Children(func() {
			mygo.Box(c).Width(28).Height(28).Radius(theme.RadiusCtl).Center().Background(p.BgHover).Children(func() {
				icons.Icon(c, "globe").Size(15, 15).TextColor(p.Text2)
			})
			mygo.Column(c).Grow(1).Gap(1).Children(func() {
				mygo.Row(c).AlignItems(mygo.Center).Gap(theme.Space4).Children(func() {
					mygo.Text(c, server.Name)
					badge(c, status, color, soft)
				})
				cmd := append([]string{server.Command}, server.Args...)
				mygo.Text(c, strings.Join(cmd, " ")).FontSize(theme.Fonts.SM()).TextColor(p.Text3).SingleLine()
				if server.Error != "" {
					mygo.Text(c, server.Error).FontSize(theme.Fonts.SM()).TextColor(p.Danger)
				}
			})
			busy := st.mcpBusy == server.ID
			if server.Connected {
				btn := smallGhostBtn(c, "断开").Disabled(busy)
				if btn.Clicked() {
					s.setMCPToggle(c, st, server, false)
				}
			} else {
				btn := smallBtn(c, "连接").Disabled(busy)
				if btn.Clicked() {
					s.setMCPToggle(c, st, server, true)
				}
			}
			if iconBtn(c, "trash", "移除 "+server.Name).Clicked() {
				s.confirmRemoveMCP(c, st, server)
			}
		})
		if len(server.Tools) > 0 {
			mygo.Row(c).Fill().Gap(theme.Space3).Padding(0, theme.Space6, theme.Space5, theme.Space6).Children(func() {
				for _, tool := range server.Tools {
					chip := mygo.Box(c).Padding(1, theme.Space5).Radius(theme.RadiusBadge).
						Background(p.BgSunken).BorderWidth(1).BorderColor(p.Border)
					chip.Children(func() {
						mygo.Text(c, tool.Name).FontSize(theme.Fonts.XS()).TextColor(p.Text2)
					})
				}
			})
		}
	})
}

// addMCPServer 分两步收集名称与命令，后台添加并连接。
func (s *Store) addMCPServer(c *mygo.Context, st *settingsState) {
	Prompt(c, PromptOpts{
		Title:       "添加 MCP 服务器",
		Label:       "给服务器起一个名称，例如 filesystem。",
		Placeholder: "filesystem",
		ConfirmText: "下一步",
	}, func(name string, ok bool) {
		if !ok {
			return
		}
		Prompt(c, PromptOpts{
			Title:       name + " 的启动命令",
			Label:       "命令以当前用户权限在本地运行；模型密钥会从环境变量中过滤。",
			Placeholder: "npx -y @modelcontextprotocol/server-filesystem .",
			ConfirmText: "添加并连接",
		}, func(command string, ok bool) {
			if !ok {
				return
			}
			st.mcpBusy = "add"
			s.runAsync(func() func() {
				server, err := s.be.MCP.Add(name, command)
				return func() {
					st.mcpBusy = ""
					switch {
					case err != nil:
						Notify("添加 MCP 服务器失败", err.Error(), ToneDanger)
					case server.Error != "":
						Notify("服务器已保存，但连接失败", server.Error, ToneWarn)
					default:
						Notify("MCP 服务器已连接", fmt.Sprintf("%s · %d 个工具", server.Name, len(server.Tools)), ToneOK)
					}
					s.refreshMCP()
				}
			})
		})
	})
}

// setMCPToggle 连接或断开一台 MCP 服务器（后台执行）。
func (s *Store) setMCPToggle(c *mygo.Context, st *settingsState, server *service.MCPServerView, connect bool) {
	st.mcpBusy = server.ID
	s.runAsync(func() func() {
		var updated service.MCPServerView
		var err error
		if connect {
			updated, err = s.be.MCP.Connect(server.ID)
		} else {
			err = s.be.MCP.Disconnect(server.ID)
		}
		return func() {
			st.mcpBusy = ""
			if err != nil {
				Notify("操作 MCP 服务器失败", err.Error(), ToneDanger)
			} else if connect && updated.Error != "" {
				Notify("连接失败", updated.Error, ToneDanger)
			} else if connect {
				Notify("已连接", fmt.Sprintf("%s · %d 个工具", updated.Name, len(updated.Tools)), ToneOK)
			}
			s.refreshMCP()
		}
	})
}

// confirmRemoveMCP 二次确认后移除 MCP 服务器。
func (s *Store) confirmRemoveMCP(c *mygo.Context, st *settingsState, server *service.MCPServerView) {
	Confirm(c, ConfirmOpts{
		Title:       "移除 " + server.Name + "？",
		Description: "会断开连接并删除服务器配置；已注册的工具立即失效。",
		ConfirmText: "移除",
		Danger:      true,
	}, func(ok bool) {
		if !ok {
			return
		}
		if err := s.be.MCP.Remove(server.ID); err != nil {
			Notify("移除 MCP 服务器失败", err.Error(), ToneDanger)
			return
		}
		s.refreshMCP()
	})
}

// tabAppearance 画外观分组：三张主题卡与字号设置。
func (s *Store) tabAppearance(c *mygo.Context, st *settingsState) {
	p := pal(c)
	settingsSection(c, "外观", "主题与字体只影响本地界面。", func() {
		setRow(c, "主题", "跟随系统、深色或浅色；代码主题独立。", func() {
			mygo.Row(c).Gap(theme.Space5).Children(func() {
				for _, opt := range []struct{ id, label, icon string }{
					{"system", "跟随系统", "monitor"},
					{"dark", "深色", "moon"},
					{"light", "浅色", "sun"},
				} {
					on := st.draft.Theme == opt.id
					card := mygo.ButtonBase(c)
					card.Width(116).Padding(theme.Space4).Radius(theme.RadiusBtn).
						Background(p.BgInput).BorderWidth(1).BorderColor(p.Border)
					if on {
						card.BorderColor(p.AccentLine)
					}
					card.Children(func() {
						sidebar, main, accent := theme.ThemePreview(opt.id)
						// ButtonBase 默认横排：卡片内容显式竖排。
						mygo.Column(c).Gap(theme.Space3).Children(func() {
							// 预览块：侧栏条 / 主区 / 强调色。
							mygo.Row(c).Width(96).Height(40).Gap(2).Clip().Children(func() {
								mygo.Box(c).Width(14).Height(40).Background(sidebar)
								mygo.Box(c).Grow(1).Height(40).Background(main)
								mygo.Box(c).Width(10).Height(40).Background(accent)
							})
							mygo.Row(c).AlignItems(mygo.Center).Gap(theme.Space3).Children(func() {
								mygo.Text(c, opt.label).FontSize(theme.Fonts.SM())
								icons.Icon(c, opt.icon).Size(13, 13).TextColor(p.Text3)
							})
						})
					})
					if card.Clicked() && !on {
						s.save(c, func(x *service.Settings) { x.Theme = opt.id })
					}
				}
			})
		})
		setRow(c, "界面字号", "界面基准字号，影响消息、侧栏、面板与浮层；终端与编辑器使用独立字号。", func() {
			enumSelect(c, &st.draft.FontSize, 140, []selectOption[int]{
				{12, "12px"}, {13, "13px"}, {14, "14px"}, {15, "15px"}, {16, "16px"},
			}, func() {
				s.save(c, func(x *service.Settings) { x.FontSize = st.draft.FontSize })
			})
		})
		setRow(c, "终端字号", "终端仿真器使用的字号。", func() {
			enumSelect(c, &st.draft.TerminalFontSize, 140, []selectOption[int]{
				{11, "11px"}, {12, "12px"}, {13, "13px"}, {14, "14px"}, {15, "15px"},
			}, func() {
				s.save(c, func(x *service.Settings) { x.TerminalFontSize = st.draft.TerminalFontSize })
			})
		})
		setRow(c, "代码主题", "预留给独立的编辑器配色；当前跟随界面主题。", func() {
			enumSelect(c, &st.draft.CodeTheme, 140, []selectOption[string]{
				{"dark", "深色"}, {"light", "浅色"},
			}, nil).Disabled(true)
		})
	})
}

// tabAdvanced 画高级分组：版本信息、备份入口、导出留桩、重置与许可证。
func (s *Store) tabAdvanced(c *mygo.Context, st *settingsState) {
	p := pal(c)
	info := s.appInfo()
	settingsSection(c, "高级", "本地数据、版本与恢复。", func() {
		setRow(c, "版本", "构建信息", func() {
			mygo.Text(c, fmt.Sprintf("%s · %s · %s/%s", info.Version, info.Commit, info.Platform, info.Arch)).
				FontSize(theme.Fonts.SM()).TextColor(p.Text2)
		})
		setRow(c, "备份与恢复", "写入文件前的备份保存在数据目录 backups/ 下。", func() {
			if smallBtn(c, "查看备份").Clicked() {
				OpenData(TabBackup)
			}
		})
		setRow(c, "导出数据", "导出数据库与配置，不包含 keys.json。", func() {
			if smallGhostBtn(c, "导出").Clicked() {
				Notify("导出在 M4 接入", "", ToneInfo)
			}
		})
		setRow(c, "重置设置", "恢复默认设置，不删除会话与工作区。", func() {
			if smallGhostBtn(c, "恢复默认").Clicked() {
				Confirm(c, ConfirmOpts{
					Title:       "恢复默认设置？",
					Description: "主题、字号、审批档位等会恢复默认；会话与工作区不受影响。",
					ConfirmText: "恢复默认",
					Danger:      true,
				}, func(ok bool) {
					if ok {
						s.save(c, func(x *service.Settings) { *x = service.DefaultSettings() })
						Notify("已恢复默认设置", "", ToneOK)
					}
				})
			}
		})
		setRow(c, "许可证", "Apache-2.0；贡献流程使用 DCO。", func() {
			badge(c, "Apache-2.0", p.Text3, p.BgHover)
		})
	})
}

// appInfo 返回应用信息；服务未注入时返回零值。
func (s *Store) appInfo() service.AppInfo {
	if s.be == nil || s.be.App == nil {
		return service.AppInfo{}
	}
	return s.be.App.Info()
}

// policyLevelRisk 把风险等级字符串映射到 policy.Risk；未知值回退 R0。
func policyLevelRisk(level string) policy.Risk {
	switch level {
	case "R1":
		return policy.R1
	case "R2":
		return policy.R2
	case "R3":
		return policy.R3
	case "R4":
		return policy.R4
	default:
		return policy.R0
	}
}
