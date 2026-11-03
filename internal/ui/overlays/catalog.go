// 模型目录浏览器：从嵌入的 models.dev 快照里选择提供商与模型，导入为
// 普通用户配置。双栏布局：左栏提供商搜索列表（含 logo 与「已添加」标记），
// 右栏模型过滤 / 勾选 / 导入。应用没有任何内置或推荐的提供商条目。

package overlays

import (
	"fmt"
	"strings"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// catalogState 是目录浏览器的界面状态。
type catalogState struct {
	// providers 是目录提供商全量列表（本地过滤，不重复请求后端）。
	providers []service.CatalogProviderView
	loaded    bool
	// providerQuery 是左栏搜索关键字（匹配名称与 ID）。
	providerQuery string
	// selectedID 是右栏当前选中的提供商标识。
	selectedID string

	// models 是选中提供商的模型列表。
	models []service.CatalogModelView
	// modelQuery 是模型搜索关键字；过滤器与 keyRequired 对齐 React 版。
	modelQuery     string
	requireTools   bool
	requireVision  bool
	showDeprecated bool
	keyRequired    bool
	selectedModels map[string]bool
	importing      bool
	err            string
}

// loadCatalog 在浏览器打开时读取目录提供商列表。
func (s *Store) loadCatalog(st *settingsState) {
	if s.be == nil || s.be.Providers == nil {
		return
	}
	if items, err := s.be.Providers.CatalogProviders(""); err == nil {
		st.catalog.providers = items
	} else {
		st.catalog.err = err.Error()
	}
	st.catalog.loaded = true
}

// drawCatalog 画双栏目录浏览器（替换模型分组内容区）。
func (s *Store) drawCatalog(c *mygo.Context, st *settingsState) {
	cat := &st.catalog
	p := pal(c)
	if !cat.loaded {
		s.loadCatalog(st)
	}
	if cat.selectedModels == nil {
		// 首次进入：工具能力默认要求勾选（对齐 React 版初值）。
		cat.requireTools = true
		cat.selectedModels = map[string]bool{}
		cat.keyRequired = true
	}

	mygo.Column(c).Fill().Gap(theme.Space5).Children(func() {
		// 头部。
		mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space4).Children(func() {
			icons.Icon(c, "globe").Size(16, 16).TextColor(p.Text2)
			mygo.Text(c, "从模型目录添加").FontSize(theme.Fonts.LG()).Bold()
			mygo.Spacer(c)
			if iconBtn(c, "x", "关闭模型目录").Clicked() {
				st.catalogOpen = false
			}
		})

		// 双栏。Scroll 内的行不参与 Grow（无界约束下高度归零），给显式高度，
		// 列内各自的滚动区在有界行内生效。
		mygo.Row(c).Height(470).AlignItems(mygo.Stretch).Gap(theme.Space5).Children(func() {
			s.catalogProviderPane(c, cat)
			s.catalogModelPane(c, st, cat)
		})

		// 页脚：目录元信息。
		if meta := s.catalogMeta(); meta != nil {
			mutedText(c, fmt.Sprintf("目录：models.dev · %s · %d 个提供商 · %d 个模型 · %s",
				clipDate(meta.GeneratedAt), meta.ProviderCount, meta.ModelCount, meta.License))
		}
	})
}

// catalogProviderPane 画左栏：提供商搜索与列表。
func (s *Store) catalogProviderPane(c *mygo.Context, cat *catalogState) {
	p := pal(c)
	query := strings.ToLower(strings.TrimSpace(cat.providerQuery))
	mygo.Column(c).Width(280).FillHeight().Gap(theme.Space4).Children(func() {
		mygo.Field(c, "搜索提供商", func() {
			in := mygo.TextInput(c, &cat.providerQuery).Fill().Height(theme.HeightInput).
				Background(p.BgInput).Border(1, p.Border).Radius(theme.RadiusBtn).
				Placeholder("名称或 ID，例如 openrouter")
			_ = in
		}).Fill()
		// 目录列表用 ui.List（虚拟化集合视图）：Scroll 逐个构建 ButtonBase
		// 在真实窗口下不绘制（tester 有界测量与窗口不一致），List 只构建
		// 可见行，216 个提供商也稳定。
		visible := make([]*service.CatalogProviderView, 0, len(cat.providers))
		for i := range cat.providers {
			prov := &cat.providers[i]
			if query != "" &&
				!strings.Contains(strings.ToLower(prov.ID), query) &&
				!strings.Contains(strings.ToLower(prov.Name), query) {
				continue
			}
			visible = append(visible, prov)
		}
		list := mygo.List(c, nil, len(visible), func(i int) {
			prov := visible[i]
			active := cat.selectedID == prov.ID
			item := mygo.ButtonBase(c)
			item.Fill().Height(44).Radius(theme.RadiusRow).
				Padding(0, theme.Space4).AlignItems(mygo.Center).Gap(theme.Space4)
			if active {
				item.Background(p.BgActive)
			}
			item.Children(func() {
				s.providerLogo(c, prov.ID, prov.Name, 24)
				mygo.Column(c).Grow(1).Gap(1).Children(func() {
					mygo.Text(c, prov.Name).SingleLine()
					mygo.Text(c, fmt.Sprintf("%d 个模型", prov.ModelCount)).FontSize(theme.Fonts.XS()).TextColor(p.Text4).SingleLine()
				})
				if prov.Added {
					badge(c, "已添加", p.Text3, p.BgHover)
				}
				icons.Icon(c, "chevron-right").Size(13, 13).TextColor(p.Text4)
			})
			if item.Clicked() {
				s.selectCatalogProvider(cat, prov.ID)
			}
		}).Height(400).Justify(mygo.Start)
		if len(visible) == 0 {
			mutedText(c, "正在读取目录…")
		}
		_ = list
	})
}

// selectCatalogProvider 选中左栏提供商并读取其模型列表。
func (s *Store) selectCatalogProvider(cat *catalogState, id string) {
	if cat.selectedID == id {
		return
	}
	cat.selectedID = id
	cat.models = nil
	cat.modelQuery = ""
	cat.selectedModels = map[string]bool{}
	cat.requireTools = true
	cat.requireVision = false
	cat.showDeprecated = false
	cat.err = ""
	if s.be == nil || s.be.Providers == nil {
		return
	}
	if items, err := s.be.Providers.CatalogModels(id); err == nil {
		cat.models = items
	} else {
		cat.err = err.Error()
	}
}

// catalogModelPane 画右栏：提供商信息、过滤器、模型勾选表与导入操作。
func (s *Store) catalogModelPane(c *mygo.Context, st *settingsState, cat *catalogState) {
	p := pal(c)
	mygo.Column(c).Grow(1).FillHeight().Gap(theme.Space4).Children(func() {
		prov := s.catalogProviderByID(cat, cat.selectedID)
		if prov == nil {
			emptyState(c, "globe", "选择一个提供商", "左侧列表只来自 models.dev，不包含任何内置或推荐条目。")
			return
		}

		// 提供商信息与导入摘要。
		mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space4).Children(func() {
			mygo.Column(c).Grow(1).Gap(1).Children(func() {
				mygo.Text(c, prov.Name)
				sub := fmt.Sprintf("%s · %d 个模型", prov.ID, prov.ModelCount)
				if prov.Doc != "" {
					sub += " · " + prov.Doc
				}
				mutedText(c, sub)
			})
			if prov.Added {
				badge(c, "已添加，请直接编辑", p.Text3, p.BgHover)
			}
		})
		mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space5).Children(func() {
			mygo.Text(c, prov.Endpoint).FontSize(theme.Fonts.SM()).TextColor(p.Text3).Grow(1).SingleLine()
			mygo.Checkbox(c, &cat.keyRequired, "需要 API Key（本地端点可以关闭）")
		})

		// 过滤器行。
		mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space5).Children(func() {
			in := mygo.TextInput(c, &cat.modelQuery).Width(220).Height(theme.HeightInput).
				Background(p.BgInput).Border(1, p.Border).Radius(theme.RadiusBtn).
				Label("搜索模型").Placeholder("搜索模型 ID 或名称")
			_ = in
			mygo.Checkbox(c, &cat.requireTools, "工具")
			mygo.Checkbox(c, &cat.requireVision, "图片")
			mygo.Checkbox(c, &cat.showDeprecated, "显示弃用")
			mygo.Spacer(c)
			if smallGhostBtn(c, "全选当前结果").Clicked() {
				for _, item := range s.visibleCatalogModels(cat) {
					cat.selectedModels[item.ID] = true
				}
			}
			if smallGhostBtn(c, "清空").Clicked() {
				cat.selectedModels = map[string]bool{}
			}
		})

		// 模型表。
		visible := s.visibleCatalogModels(cat)
		cols := []mygo.TableColumn{
			{Title: "", Width: 36},
			{Title: "模型"},
			{Title: "上下文", Width: 76, Align: mygo.End},
			{Title: "价格 ($/M)", Width: 130, Align: mygo.End},
			{Title: "能力", Width: 170},
		}
		mygo.Table(c, nil, cols, len(visible), func(row, col int) {
			item := visible[row]
			p := pal(c)
			switch col {
			case 0:
				// 勾选框绑定本帧草稿值，变更写回勾选集合（表状态不持指针）。
				box := mygo.Box(c).FillHeight().Center()
				checked := cat.selectedModels[item.ID]
				box.Children(func() {
					cb := mygo.CheckboxBase(c, &checked)
					cb.Label("选择 " + item.Name)
					if cb.Changed() {
						cat.selectedModels[item.ID] = checked
					}
				})
			case 1:
				mygo.Column(c).Gap(1).Children(func() {
					mygo.Row(c).AlignItems(mygo.Center).Gap(theme.Space3).Children(func() {
						mygo.Text(c, item.Name).SingleLine()
						if item.Status != "" {
							badge(c, item.Status, p.Text3, p.BgHover)
						}
					})
					mygo.Text(c, item.ID).FontSize(theme.Fonts.XS()).TextColor(p.Text4).SingleLine()
				})
			case 2:
				mygo.Textf(c, "%dk", item.ContextK).AlignSelf(mygo.End)
			case 3:
				v := "—"
				if item.Cost != nil {
					v = fmt.Sprintf("$%g / $%g", item.Cost.InputPerMTok, item.Cost.OutputPerMTok)
				}
				mygo.Text(c, v).AlignSelf(mygo.End).TextColor(p.Text2)
			case 4:
				caps := make([]string, 0, 4)
				if item.Caps.Tools {
					caps = append(caps, "工具")
				}
				if item.Caps.Reasoning {
					caps = append(caps, "推理")
				}
				if item.Caps.Vision {
					caps = append(caps, "图片")
				}
				if item.Caps.Structured {
					caps = append(caps, "结构化")
				}
				if len(caps) == 0 {
					caps = append(caps, "—")
				}
				mygo.Text(c, strings.Join(caps, " / ")).TextColor(p.Text2).SingleLine()
			}
		}).Grow(1).BorderWidth(1).BorderColor(p.BorderSoft).Radius(theme.RadiusRow)

		if cat.err != "" {
			mygo.Text(c, cat.err).TextColor(p.Danger)
		}

		// 底部操作行。
		mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space4).Children(func() {
			mutedText(c, fmt.Sprintf("已选 %d 个模型", len(cat.selectedModels))).Grow(1)
			if mygo.Button(c, "取消").Clicked() {
				st.catalogOpen = false
			}
			importBtn := primaryBtn(c, "导入提供商").
				Disabled(cat.importing || len(cat.selectedModels) == 0 || prov.Added)
			if importBtn.Clicked() {
				s.importCatalogProvider(c, st, cat, prov)
			}
		})
	})
}

// catalogProviderByID 按 ID 找目录提供商视图。
func (s *Store) catalogProviderByID(cat *catalogState, id string) *service.CatalogProviderView {
	if id == "" {
		return nil
	}
	for i := range cat.providers {
		if cat.providers[i].ID == id {
			return &cat.providers[i]
		}
	}
	return nil
}

// visibleCatalogModels 按关键字与能力过滤器过滤模型列表。
func (s *Store) visibleCatalogModels(cat *catalogState) []service.CatalogModelView {
	query := strings.ToLower(strings.TrimSpace(cat.modelQuery))
	out := make([]service.CatalogModelView, 0, len(cat.models))
	for _, item := range cat.models {
		if query != "" &&
			!strings.Contains(strings.ToLower(item.ID), query) &&
			!strings.Contains(strings.ToLower(item.Name), query) {
			continue
		}
		if cat.requireTools && !item.Caps.Tools {
			continue
		}
		if cat.requireVision && !item.Caps.Vision {
			continue
		}
		if !cat.showDeprecated && item.Status == "deprecated" {
			continue
		}
		out = append(out, item)
	}
	return out
}

// importCatalogProvider 提交导入并收尾：关浏览器、刷新列表、按需提示配密钥。
func (s *Store) importCatalogProvider(c *mygo.Context, st *settingsState, cat *catalogState, prov *service.CatalogProviderView) {
	if len(cat.selectedModels) == 0 {
		cat.err = "至少选择一个模型"
		return
	}
	ids := make([]string, 0, len(cat.selectedModels))
	for id, on := range cat.selectedModels {
		if on {
			ids = append(ids, id)
		}
	}
	keyRequired := cat.keyRequired
	cat.importing = true
	cat.err = ""
	s.runAsync(func() func() {
		input := service.CatalogImportInput{
			ProviderID:  prov.ID,
			ModelIDs:    ids,
			KeyRequired: &keyRequired,
		}
		saved, err := s.be.Providers.ImportFromCatalog(input)
		return func() {
			cat := &st.catalog
			cat.importing = false
			if err != nil {
				cat.err = err.Error()
				return
			}
			st.catalogOpen = false
			st.providers = s.providerList()
			if st.draft.DefaultModel == "" && len(saved.Models) > 0 {
				s.save(c, func(x *service.Settings) { x.DefaultModel = saved.Models[0].Ref })
			}
			needsKey := saved.KeyRequired && saved.KeySource == ""
			if needsKey {
				Notify("还需配置 API Key",
					fmt.Sprintf("%s · %d 个模型；展开提供商卡片即可设置密钥", saved.Name, len(saved.Models)), ToneWarn)
			} else {
				Notify("提供商已添加", fmt.Sprintf("%s · %d 个模型", saved.Name, len(saved.Models)), ToneOK)
			}
		}
	})
}
