// composer.go 是输入区（Composer）的三个浮层与 R0–R4 说明浮层（11 号票
// 接线，15 号票锚定化）：模型选择、审批档位切换、上下文详情与风险等级
// 对照。
//
// 形态对齐 React 版 Composer.tsx 的四个 Popover：面板贴触发控件锚定弹出、
// backdrop 透明（不压暗窗口），承载仍走 DialogBase（Esc / 点击外部关闭
// 由浮层体系处理，不另起第二套退出链；锚定落位见 anchor.go 与
// placePanelAnchored）。行为要点：
//   - 模型选择：清单来自用户已添加提供商下的可用模型，支持按名称 / ID /
//     提供商搜索；勾选态优先当前会话模型，无会话模型退默认模型；选择写入
//     设置默认模型（model.Ref 语义，服务保存时归一化），与设置中心「模型
//     与提供商」的默认模型下拉共用同一保存路径，天然双向同步。
//   - 审批档位：四档与设置中心「工具与权限」同源（approvalOptions）；保存
//     走同一条 SettingsService.Save（策略引擎实时接管 + 审计落账）；自由档
//     先过风险确认门（FreeConfirm）；保存成功后经回调把新档位同步给聊天列
//     的只读展示（composer 打开浮层时注册）。
//   - 上下文详情与风险等级是纯展示浮层：数据由调用方在打开瞬间快照传入，
//     风险表直接引用设置中心的 riskRows，不写第二份清单。
package overlays

import (
	"fmt"
	"strings"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// composerKind 标识当前打开的输入区浮层；空串表示没有浮层。
type composerKind string

const (
	composerModel    composerKind = "model"
	composerApproval composerKind = "approval"
	composerRisk     composerKind = "risk"
	composerContext  composerKind = "context"
)

// ContextDetail 是上下文详情浮层的快照数据：调用方（聊天列）在打开瞬间
// 从会话状态组装，浮层只做展示，不回读会话。
type ContextDetail struct {
	// Used 是当前上下文占用（token，实测优先、估算兜底）。
	Used int64
	// WindowK 是模型上下文窗口（千 token）。
	WindowK int
	// Percent 是占用百分比（0–100）。
	Percent int
	// Tokens 是本会话累计 token。
	Tokens int64
	// CacheTokens 是最近一轮的缓存命中 token；0 显示占位符。
	CacheTokens int64
	// Speed 是最近一轮输出速度（tok/s）；0 显示占位符。
	Speed float64
}

// composerState 是输入区浮层的状态：同一时间只开一个（kind 互斥）。
// 打开瞬间快照清单与当前值，浮层期间不重读服务。
type composerState struct {
	// open 是 DialogBase 的打开标记；kind 决定面板内容。
	open bool
	kind composerKind
	// query 是模型搜索词。
	query string
	// models 是打开时快照的可用模型清单（用户提供商下的可用模型）。
	models []service.ModelView
	// current 是打开时快照的模型引用（勾选态）：当前会话模型优先，无会话
	// 模型时退设置里的默认模型（OpenComposerModels 的 current 参数语义）。
	current string
	// level 是打开时快照的当前档位（勾选态与同值短路）。
	level string
	// onLevel 是档位保存成功后的回调（原始档位字符串），聊天列用它同步
	// 输入区只读展示；nil 表示无人订阅。
	onLevel func(level string)
	// detail 是上下文详情浮层的展示数据。
	detail ContextDetail
	// anchor 是打开瞬间快照的触发控件 Bounds（窗口 DIP 坐标，mygo
	// Element.Bounds 的上一帧值；触发控件位置帧间稳定）。零值（仅测试
	// 路径会传）回退 fraction 居中落位。
	anchor mygo.Rect
	// panelH 是上一帧面板的实测高（Bounds 是上一帧的，打开首帧为 0，
	// 用估算值参与翻转决策；见 placeAnchored）。
	panelH float32
	// place 是最近一帧的锚定落位决策（ComposerFlyoutFlipped 走查读数）。
	place anchoredPlace
}

// composerAnchorOK 报告锚点是否有效：触发控件没画出来（测试直接调 Open
// 传零值）时浮层退回 fraction 居中，行为与 11 号票一致。
func composerAnchorOK(a mygo.Rect) bool { return a.W > 0 && a.H > 0 }

// OpenComposerModels 打开模型选择浮层：快照用户提供商下的可用模型与当前
// 模型引用。anchor 是触发控件的 Bounds（面板贴它锚定弹出，见
// placeAnchored）。勾选态优先当前会话模型（current），没有会话模型时退
// 设置里的默认模型——「当前模型勾选态」跟会话走，「选择写入默认模型」
// 是保存语义，两句并存互不冲突（review 修复轮：此前勾选态只看默认模型，
// 会话已切换模型时勾选落后于会话）。
func OpenComposerModels(current string, anchor mygo.Rect) {
	s := &store.composer
	s.open, s.kind = true, composerModel
	s.query = ""
	s.models = store.composerModels()
	if current == "" {
		current = store.settingsSnapshot().DefaultModel
	}
	s.current = current
	s.openAt(anchor)
}

// ComposerModelsCurrent 返回模型浮层打开时快照的勾选引用（走查断言用）：
// 当前会话模型优先，无会话模型时是设置里的默认模型。
func ComposerModelsCurrent() string { return store.composer.current }

// OpenComposerApproval 打开审批档位浮层（anchor 语义同 OpenComposerModels）。
// onChanged 在档位保存成功后于主线程执行，参数是新档位的原始字符串
// （graded / strict / readonly / free）。
func OpenComposerApproval(onChanged func(level string), anchor mygo.Rect) {
	s := &store.composer
	s.open, s.kind = true, composerApproval
	s.level = store.settingsSnapshot().ApprovalLevel
	s.onLevel = onChanged
	s.openAt(anchor)
}

// OpenComposerRisk 打开 R0–R4 风险等级说明浮层（anchor 语义同
// OpenComposerModels）。
func OpenComposerRisk(anchor mygo.Rect) {
	store.composer.open, store.composer.kind = true, composerRisk
	store.composer.openAt(anchor)
}

// OpenComposerContext 打开上下文详情浮层；数据由调用方快照传入（anchor
// 语义同 OpenComposerModels）。
func OpenComposerContext(detail ContextDetail, anchor mygo.Rect) {
	s := &store.composer
	s.open, s.kind = true, composerContext
	s.detail = detail
	s.openAt(anchor)
}

// openAt 记录打开锚点并复位落位状态：panelH 清零让首帧用估算值，
// place 清零避免读到上一轮浮层的落位。
func (st *composerState) openAt(anchor mygo.Rect) {
	st.anchor = anchor
	st.panelH = 0
	st.place = anchoredPlace{}
}

// CloseComposer 关闭输入区浮层（供测试与外部复位）。
func CloseComposer() { store.composer.open = false }

// composerModels 汇总用户已添加提供商下的可用模型（React 版 providers
// flatMap models filter available 同口径）；服务未注入时返回空。
func (s *Store) composerModels() []service.ModelView {
	var out []service.ModelView
	for _, provider := range s.providerList() {
		for _, m := range provider.Models {
			if m.Available {
				out = append(out, m)
			}
		}
	}
	return out
}

// drawComposer 画当前打开的输入区浮层。Dismissed（Esc / 点击外部）直接
// 收起；锚点有效时面板贴触发控件弹出、backdrop 透明（React 版 Popover
// 形态），无效时退回 11 号票的 fraction 居中模态（测试路径兜底）。
func (s *Store) drawComposer(c *mygo.Context) {
	st := &s.composer
	if !st.open {
		return
	}
	width := theme.ComposerApprovalWidth
	switch st.kind {
	case composerModel:
		width = theme.ComposerModelWidth
	case composerRisk:
		width = theme.ComposerRiskWidth
	case composerContext:
		width = theme.ComposerContextWidth
	}
	anchored := composerAnchorOK(st.anchor)
	if anchored {
		// 翻转与钳制决策用上一帧实测高，首帧退估算值（误差只影响极端
		// 小窗下的上下翻转判断，正常窗口触发控件贴底、上方总是充足）。
		panelH := st.panelH
		if panelH <= 0 {
			panelH = theme.ComposerFlyoutEstH
		}
		w, h := c.Size()
		st.place = placeAnchored(st.anchor, width, panelH, w, h,
			theme.ComposerFlyoutGap, theme.ComposerFlyoutEdgeMargin)
	}
	panel := mygo.DialogBase(c, &st.open, func(backdrop, panel mygo.Element) {
		if anchored {
			// 透明 backdrop：不压暗窗口（styleBackdrop 的半透明黑只属于
			// 中央模态）；点击关闭语义由 DialogBase 的 backdrop 点击处理。
			backdrop.Background(mygo.Transparent)
			stylePanel(c, panel)
			placePanelAnchored(c, backdrop, panel, width, st.place, st.anchor)
		} else {
			styleBackdrop(backdrop)
			stylePanel(c, panel)
			placePanel(c, backdrop, panel, width, 0.18)
		}
		panel.Padding(theme.ComposerPanelPadT, theme.ComposerPanelPadX, theme.ComposerPanelPadB).
			Gap(theme.Space3).Children(func() {
			switch st.kind {
			case composerModel:
				s.drawComposerModels(c, st)
			case composerApproval:
				s.drawComposerApproval(c, st)
			case composerRisk:
				s.drawComposerRisk(c)
			case composerContext:
				s.drawComposerContext(c, st.detail)
			}
		})
	})
	if b := panel.Bounds(); b.H > 0 {
		st.panelH = b.H // 上一帧实测高，下一帧翻转决策更准（帧间内容静态）
	}
	if panel.Dismissed() {
		st.open = false
	}
}

// ComposerFlyoutFlipped 报告最近一帧锚定浮层是否翻到了触发控件下方
// （ui.Tester 走查用；正常窗口触发控件贴底，恒为否）。
func ComposerFlyoutFlipped() bool { return store.composer.place.Flipped }

// drawComposerModels 画模型选择浮层：搜索框 + 虚拟化清单 + 备注。
func (s *Store) drawComposerModels(c *mygo.Context, st *composerState) {
	p := pal(c)
	mygo.Text(c, "选择模型").Bold()
	if len(st.models) == 0 {
		mutedText(c, "没有可用模型，请先在设置中添加提供商并配置密钥。")
		return
	}
	// 搜索框：按名称 / 模型 ID / 提供商名过滤（React 版同口径）。
	mygo.Field(c, "模型搜索", func() {
		mygo.TextInput(c, &st.query).Fill().Height(theme.HeightInput).
			Background(p.BgInput).Border(1, p.Border).Radius(theme.RadiusBtn).
			Placeholder("搜索模型或提供商")
	}).Fill()

	query := strings.ToLower(strings.TrimSpace(st.query))
	visible := make([]service.ModelView, 0, len(st.models))
	for _, m := range st.models {
		if query != "" &&
			!strings.Contains(strings.ToLower(m.Name), query) &&
			!strings.Contains(strings.ToLower(m.ID), query) &&
			!strings.Contains(strings.ToLower(m.ProviderName), query) {
			continue
		}
		visible = append(visible, m)
	}
	if len(visible) == 0 {
		mutedText(c, "没有匹配的模型。")
	}
	// 清单虚拟化（07 号票的坑：Scroll 内逐个 ButtonBase 在真实窗口不绘制），
	// 固定高度限制浮层尺寸，行身份按位置保持（搜索过滤后整表重建）。
	mygo.List(c, nil, len(visible), func(i int) {
		m := visible[i]
		active := m.Ref == st.current
		item := mygo.ButtonBase(c)
		item.Fill().Height(theme.ComposerModelRowHeight).Radius(theme.RadiusRow).
			Padding(0, theme.Space4).AlignItems(mygo.Center).Gap(theme.Space4).
			Label("选择 " + m.Name)
		if item.Hovered() {
			item.Background(p.BgHover)
		}
		item.Children(func() {
			dot := mygo.Box(c).Size(8, 8).Radius(theme.RadiusRound)
			if active {
				dot.Background(p.Accent)
			} else {
				dot.Background(p.Border)
			}
			mygo.Column(c).Grow(1).Gap(1).Children(func() {
				mygo.Text(c, m.Name).SingleLine()
				mygo.Text(c, modelItemDesc(m)).FontSize(theme.Fonts.XS()).
					TextColor(p.Text4).SingleLine()
			})
			if active {
				icons.Icon(c, "check").Size(14, 14).TextColor(p.Accent)
			}
		})
		if item.Clicked() {
			// 与设置中心的默认模型下拉同一保存路径：服务归一化为 model.Ref、
			// 校验可用性，保存后广播设置变更（双向同步的另一半在设置中心
			// 每次打开时的快照里）。选择后关闭浮层。
			s.save(c, func(x *service.Settings) { x.DefaultModel = m.Ref })
			st.kind = ""
		}
	}).Height(theme.ComposerListHeight).Justify(mygo.Start)
	mutedText(c, "所选模型将设为默认，新建会话使用；与设置中心「模型与提供商」双向同步。")
}

// modelItemDesc 拼模型行的说明：提供商 · 上下文 · 能力（已支持项）。
func modelItemDesc(m service.ModelView) string {
	var caps []string
	if m.Caps.Tools {
		caps = append(caps, "工具")
	}
	if m.Caps.Reasoning {
		caps = append(caps, "推理")
	}
	if m.Caps.Vision {
		caps = append(caps, "图片")
	}
	desc := fmt.Sprintf("%s · %dk 上下文", m.ProviderName, m.ContextK)
	if len(caps) > 0 {
		desc += " · " + strings.Join(caps, " / ")
	}
	return desc
}

// applyComposerLevel 保存新档位并触发回调（自由档已过确认门后才会到这里）。
func (s *Store) applyComposerLevel(c *mygo.Context, st *composerState, level string) {
	if st.level == level {
		st.open = false
		return
	}
	// 与设置中心同一保存路径：策略引擎实时接管、审计在服务层落账。
	s.save(c, func(x *service.Settings) { x.ApprovalLevel = level })
	st.level = level
	st.open = false
	if cb := st.onLevel; cb != nil {
		cb(level)
	}
}

// drawComposerApproval 画审批档位浮层：四档清单（与设置中心同源）+ 备注。
func (s *Store) drawComposerApproval(c *mygo.Context, st *composerState) {
	p := pal(c)
	mygo.Text(c, "审批档位").Bold()
	for _, opt := range approvalOptions {
		active := st.level == opt.level
		item := mygo.ButtonBase(c)
		// 显式高度：Tester 800 高窗口里四行 + 标题 + 备注必须全部落在
		// placePanel 的可视区内（模型行 ComposerModelRowHeight 同理）。
		item.Fill().Height(theme.ComposerLevelRowHeight).Radius(theme.RadiusRow).
			Padding(0, theme.Space4).AlignItems(mygo.Center).Gap(theme.Space4)
		if item.Hovered() {
			item.Background(p.BgHover)
		}
		item.Children(func() {
			icons.Icon(c, opt.icon).Size(15, 15).TextColor(p.Text2)
			mygo.Column(c).Grow(1).Gap(1).Children(func() {
				mygo.Text(c, opt.label)
				mygo.Text(c, opt.desc).FontSize(theme.Fonts.XS()).TextColor(p.Text3)
			})
			if active {
				icons.Icon(c, "check").Size(14, 14).TextColor(p.Accent)
			}
		})
		if item.Clicked() {
			// 自由档先过风险确认门（React 版 guardFreeLevel 同序）：档位浮层
			// 先收，确认门单独叠在界面上；取消则停在原档位。其余档位直接
			// 保存并回调，本帧渲染到此为止（浮层已收）。
			if opt.level == "free" && st.level != "free" {
				st.open = false
				// 回调捕获的 c 是本帧上下文，实际在用户点击确认的那一帧经
				// resolveFree 执行。延迟到旧帧上下文上是安全的：applyComposerLevel
				// → Store.save 不使用该参数，保存失败的提示走 Notify 排队、
				// 由通知层在下一帧消费（toast.go 的迟到上下文设计）。
				FreeConfirm(c, func(ok bool) {
					if ok {
						s.applyComposerLevel(c, st, "free")
					}
				})
				return
			}
			s.applyComposerLevel(c, st, opt.level)
			return
		}
	}
	mutedText(c, "档位与设置 · 工具与权限双向同步。")
}

// drawComposerRisk 画风险等级说明浮层：R0–R4 对照表（设置中心 riskRows
// 同源）+ 处置随档位变化的备注。
func (s *Store) drawComposerRisk(c *mygo.Context) {
	p := pal(c)
	mygo.Text(c, "风险等级").Bold()
	for _, row := range riskRows {
		color := p.Risk(policyLevelRisk(row.level))
		mygo.Row(c).Fill().AlignItems(mygo.Start).Gap(theme.Space4).Children(func() {
			mygo.Box(c).Width(44).Children(func() {
				badge(c, row.level, color, color.Alpha(0.13))
			})
			mygo.Column(c).Grow(1).Gap(1).Children(func() {
				mygo.Text(c, row.name).FontSize(theme.Fonts.SM())
				mygo.Text(c, row.policy).FontSize(theme.Fonts.SM()).TextColor(p.Text3)
			})
		})
	}
	mutedText(c, "实际处置随审批档位变化：只读只放行 R0，严格逐次询问，自由全部放行。")
}

// drawComposerContext 画上下文详情浮层：占用口径、会话累计、缓存命中与
// 最近一轮速度（React 版 ctx-stats 同四行）。
func (s *Store) drawComposerContext(c *mygo.Context, d ContextDetail) {
	p := pal(c)
	mygo.Text(c, "上下文").Bold()
	row := func(key, val string, muted bool) {
		color := p.Text
		if muted {
			color = p.Text3
		}
		mygo.Row(c).Fill().Justify(mygo.SpaceBetween).Gap(theme.Space4).Children(func() {
			mygo.Text(c, key).FontSize(theme.Fonts.SM()).TextColor(p.Text3)
			mygo.Text(c, val).FontSize(theme.Fonts.SM()).TextColor(color).SingleLine()
		})
	}
	cache := "—"
	if d.CacheTokens > 0 {
		cache = formatTokens(d.CacheTokens)
	}
	speed := "—"
	if d.Speed > 0 {
		speed = fmt.Sprintf("%d tok/s", int(d.Speed+0.5))
	}
	row("当前上下文", fmt.Sprintf("%s / %dk（%d%%）", formatTokens(d.Used), d.WindowK, d.Percent), false)
	row("本会话累计", formatTokens(d.Tokens), false)
	row("最近一轮缓存命中", cache, cache == "—")
	row("最近一轮速度", speed, speed == "—")
	mutedText(c, "占用为最近一轮模型请求的实测值；没有调用记录时按消息内容估算。")
}
