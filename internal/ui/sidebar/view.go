// 本文件是侧栏的立即模式 view：模式切换轨道滑块、会话分组列表与底部
// 两行入口。视觉取值全部来自 internal/ui/theme.go 的 token（base.css 的
// 机械翻译，CSS 像素值在注释里对照），class 对照 frontend/src/styles/
// base.css 的 .sidebar 区段。
//
// 槽位纪律：本文件只画侧栏列内部；三栏外壳（shell.go）经 Shell 的
// SidebarView 字段把 268 列的绘制委托进来。
//
// 浮层留桩：会话右键菜单暂用系统菜单（ContextMenu），删除确认走
// mygo.Dialog.Message 原生弹窗、重命名走 ui.Modal 简易浮层——07 号票
// 统一浮层体系后整体替换为应用内浮层（对照 React 版 popover-item 的
// 标题 + 描述双行样式）。

package sidebar

import (
	"strconv"
	"strings"
	"time"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui"
)

// stubNote 是 07 号票前占位入口的统一提示尾巴。
const stubNote = "待 07 号票（浮层与入口）接入"

// viewSide 是 view 帧内的一次性派生：色板 + 模式强调色 + 动作入口。
type viewSide struct {
	pal    *ui.Palette
	accent mygo.Color // 模式强调色（选中竖条、状态点、滑块）
	on     mygo.Color // 强调色之上的文字
	acts   *Actions
}

// View 画 268 固定侧栏列。由 shell.go 的侧栏槽位调用，pal 来自外壳
// 每帧套用的主题色板。
func (s *Sidebar) View(c *mygo.Context, pal *ui.Palette) {
	// 异步动作排队 的提示在帧开头弹出（c.Toast 只能在 view 帧内调用）。
	if message := s.TakeToast(); message != "" {
		c.Toast(message)
	}
	v := &viewSide{pal: pal, acts: s.actions}
	acc := pal.ModeAccent(uiMode(s.Mode))
	v.accent, v.on = acc.Accent, acc.On

	mygo.Column(c).Width(ui.SidebarWidth).
		Background(pal.SidebarBg).
		BorderWidth(0, 1, 0, 0).
		BorderColor(pal.Border).
		Children(func() {
			s.modeSwitch(c, v)
			s.threadList(c, v)
			s.footer(c, v)
			s.renameModal(c, v)
		})
}

// uiMode 把侧栏字符串模式映射为 ui 包的模式枚举。
func uiMode(m Mode) ui.Mode {
	if m == ModeWork {
		return ui.ModeWork
	}
	return ui.ModeCode
}

// ---------------------------------------------------------------------------
// 模式切换：轨道 + 滑块
// ---------------------------------------------------------------------------

// modeSwitch 画 Code / Work 切换轨道（.mode-switch）：40 高、圆角 10、
// 深底描边、内边距 4；模式色滑块随选中项 160ms 缓动平移（AnimateTo），
// 换色同步渐变（AnimateColor）。
//
// 滑块用 track.Draw 画在背景之后、子元素之前（mygo 的绝对定位子元素会
// 盖住普通兄弟，不能像 CSS 那样靠构建顺序垫底），按钮文字因此保持在
// 滑块之上。滑块的软投影（CSS box-shadow 0 2px 10px mode-accent-soft）
// 略去：Painter 无模糊阴影，色块本身已足够区分。
func (s *Sidebar) modeSwitch(c *mygo.Context, v *viewSide) {
	mygo.Box(c).Height(ui.Space5) // 上边距 10（.sidebar > .mode-switch 的 margin）
	// 轨道宽 = 侧栏宽 - 左右各 10。
	const trackW = ui.SidebarWidth - 2*ui.Space5
	anim := mygo.Animation{Duration: ui.Duration, Ease: ui.Ease}
	// 滑块宽 = 内容区一半 - 4（.mode-glider 的 calc(50% - 4px)）。
	const gliderW = (trackW-2*ui.Space4-2)/2 - ui.Space4

	track := mygo.Row(c).Width(trackW).Height(ui.HeightModeSwitch).
		Padding(ui.Space4).
		Radius(ui.RadiusPill).
		Background(v.pal.BgSunken).
		BorderWidth(1).
		BorderColor(v.pal.Border)
	// Work 态滑块平移一个滑块宽（CSS translateX(100%)）。
	x := float32(0)
	if s.Mode == ModeWork {
		x = gliderW
	}
	gx := track.AnimateTo("glider-x", x, anim)
	gbg := track.AnimateColor("glider-bg", v.accent, anim)
	track.Draw(func(p *mygo.Painter, r mygo.Rect) {
		p.Fill(mygo.Rect{
			X: r.X + ui.Space4 + gx,
			Y: r.Y + ui.Space4,
			W: gliderW,
			H: r.H - 2*ui.Space4 - 2, // 减上下 padding 与描边
		}, gbg, ui.RadiusCtl)
	})
	track.Children(func() {
		s.modeButton(c, v, ModeCode, "code", "Code")
		s.modeButton(c, v, ModeWork, "briefcase", "Work")
	})
}

// modeButton 画一个模式按钮（.mode-btn）：图标 15 + 文字；选中项反白
// 加粗（600），未选中次要色、悬停提亮。
func (s *Sidebar) modeButton(c *mygo.Context, v *viewSide, mode Mode, icon, label string) {
	selected := s.Mode == mode
	btn := mygo.ButtonBase(c).Grow(1).Radius(ui.RadiusCtl)
	if btn.Clicked() {
		// 本地先切（列表立即按该模式渲染）；服务同步由注入的动作做
		//（持久化到设置 + 拉新列表），未注入时（单测路径）也保持可用。
		s.SetMode(mode)
		if v.acts != nil && v.acts.SwitchMode != nil {
			v.acts.SwitchMode(mode)
		}
	}
	color := v.pal.Text3
	if btn.Hovered() && !selected {
		color = v.pal.Text2
	}
	weight := 400
	if selected {
		color = v.on
		weight = 600
	}
	btn.Children(func() {
		ui.Icon(c, icon).FontSize(15).TextColor(color)
		mygo.Text(c, label).FontSize(ui.Fonts.Base()).TextColor(color).FontWeight(weight)
	})
}

// ---------------------------------------------------------------------------
// 会话列表
// ---------------------------------------------------------------------------

// threadList 画滚动列表区（.thread-list，内边距 2 8 8）：临时组置顶、
// 「项目会话」区、项目分组、孤儿「其他」组与空态。
func (s *Sidebar) threadList(c *mygo.Context, v *viewSide) {
	groups := s.Groups()
	mygo.Scroll(c).Grow(1).
		Padding(ui.Space1, ui.Space4+4, ui.Space4+4).
		Children(func() {
			s.tempGroup(c, v, findGroup(groups, GroupTemp))
			s.projectSection(c, v, groups)
		})
}

// findGroup 按种类取分组；列表没有该组时返回零值（空组照常画组头）。
func findGroup(groups []Group, kind GroupKind) Group {
	for _, g := range groups {
		if g.Kind == kind {
			return g
		}
	}
	return Group{Kind: kind}
}

// groupHeadPad 是组头内边距（.thread-group-head：12 6 5）。
var groupHeadPad = pad{t: ui.Space6 + ui.Space4 + ui.Space1, r: ui.Space3, b: ui.Space1 + 3}

// projectGroups 只取项目组。
func projectGroups(groups []Group) []Group {
	out := make([]Group, 0, len(groups))
	for _, g := range groups {
		if g.Kind == GroupProject {
			out = append(out, g)
		}
	}
	return out
}

// pad 是 top/right/bottom 三元组，避免 view 里的字面算术。
type pad struct{ t, r, b float32 }

// tempGroup 画置顶「临时会话」组：组头（折叠箭头 + 名称 + 计数 + 新建）
// 与组内分页列表；空组给指引文案。
func (s *Sidebar) tempGroup(c *mygo.Context, v *viewSide, g Group) {
	open := !s.coll.IsCollapsed(g.Key)
	tempGroupHead(c, v, g.Items, open, s.ToggleTemp, func() {
		if v.acts != nil && v.acts.CreateTempThread != nil {
			v.acts.CreateTempThread()
		}
	})
	if !open {
		return
	}
	if len(g.Items) == 0 {
		listEmpty(c, v, "还没有临时会话，点上方 + 新建")
		return
	}
	s.groupItems(c, v, g)
}

// tempGroupHead 画临时组头：箭头开关 + 名称 + 计数 + 右侧 + 新建。
func tempGroupHead(c *mygo.Context, v *viewSide, items []Thread, open bool, onToggle func(), onCreate func()) {
	mygo.Row(c).AlignItems(mygo.Center).Gap(ui.Space3).Padding(groupHeadPad.t, groupHeadPad.r, groupHeadPad.b).
		Children(func() {
			groupToggle(c, v, open, "临时会话", "chevron-down", "chevron-right", onToggle)
			mygo.Text(c, "临时会话").FontSize(ui.Fonts.SM()).FontWeight(500).TextColor(v.pal.Text4).Grow(1)
			countBadge(c, v, len(items))
			groupAdd(c, v, "新建临时会话", onCreate)
		})
}

// projectSection 画「项目会话」区头、各项目组、孤儿组与项目空态。
func (s *Sidebar) projectSection(c *mygo.Context, v *viewSide, groups []Group) {
	projects := projectGroups(groups)
	var orphan Group
	hasOrphan := false
	for _, g := range groups {
		if g.Kind == GroupOrphan {
			orphan, hasOrphan = g, true
		}
	}

	// 区头（.thread-group-head.is-section，margin-top 6）：批量折叠开关
	// （有项目组才出现）+ 标题 + 总数 + 创建项目占位入口。
	mygo.Row(c).AlignItems(mygo.Center).Gap(ui.Space3).
		Padding(groupHeadPad.t+ui.Space3, groupHeadPad.r, groupHeadPad.b).
		Children(func() {
			if len(projects) > 0 {
				allCollapsed := s.coll.AllProjectsCollapsed(groups)
				// 折叠态显示 chevron-right（「展开所有项目」），否则 chevron-down。
				onToggle := func() { s.ToggleAllProjects() }
				if allCollapsed {
					groupToggle(c, v, false, "展开所有项目", "chevron-down", "chevron-right", onToggle)
				} else {
					groupToggle(c, v, true, "折叠所有项目", "chevron-down", "chevron-right", onToggle)
				}
			} else {
				mygo.Box(c).Width(ui.Space4 + ui.Space6 + ui.Space4) // 无开关时对齐临时组的箭头位（18）
			}
			mygo.Text(c, "项目会话").FontSize(ui.Fonts.SM()).FontWeight(500).TextColor(v.pal.Text4).Grow(1)
			countBadge(c, v, ProjectThreadCount(groups))
			groupAdd(c, v, "创建项目", func() {
				// 创建项目对话框归 07 号票（React 版 openCreateProject）；
				// 先占位提示，不做假入口。
				c.Toast("创建项目" + stubNote)
			})
		})

	if len(projects) == 0 && !hasOrphan {
		listEmpty(c, v, "还没有项目\n点上方 + 创建项目")
		return
	}
	for _, g := range projects {
		s.projectGroup(c, v, g)
	}
	if hasOrphan {
		s.orphanGroup(c, v, orphan)
	}
}

// groupItems 画组内分页列表：可见条数内逐条渲染，超出时「显示更多」。
func (s *Sidebar) groupItems(c *mygo.Context, v *viewSide, g Group) {
	limit := s.Limit(g.Key)
	visible, hasMore := VisibleItems(g.Items, limit)
	for _, t := range visible {
		s.threadItem(c, v, t)
	}
	if !hasMore {
		return
	}
	remaining := len(g.Items) - limit
	more := mygo.ButtonBase(c).Fill().AlignItems(mygo.Start).
		Padding(ui.Space4, ui.Space4+4, ui.Space3+ui.Space4, ui.Space4+ui.Space5+ui.Space7+ui.Space1). // 4 8 6 26
		Radius(ui.RadiusRow)
	if more.Hovered() {
		more.Background(v.pal.SidebarHover)
	}
	if more.Clicked() {
		s.ShowMore(g.Key)
	}
	more.Children(func() {
		mygo.Text(c, "显示更多（还有 "+strconv.Itoa(remaining)+" 条）").
			FontSize(ui.Fonts.SM()).TextColor(v.pal.Text3)
	})
}

// projectGroup 画一个项目组：文件夹图标随展开状态切换（folder-open /
// folder），组头 + 在该项目新建会话，组内分页。
func (s *Sidebar) projectGroup(c *mygo.Context, v *viewSide, g Group) {
	open := !s.coll.IsCollapsed(g.Key)
	mygo.Row(c).AlignItems(mygo.Center).Gap(ui.Space3).Padding(groupHeadPad.t, groupHeadPad.r, groupHeadPad.b).
		Children(func() {
			fold, name := "folder", "折叠项目会话"
			if open {
				fold = "folder-open"
			} else {
				name = "展开项目会话"
			}
			foldBtn := mygo.ButtonBase(c).Size(18, 18).Center().
				Radius(ui.RadiusBadge).Tooltip(name)
			if foldBtn.Clicked() {
				s.ToggleProject(g.Key)
			}
			foldBtn.Children(func() {
				ui.Icon(c, fold).FontSize(13).TextColor(v.pal.Text4)
			})
			mygo.Text(c, g.Name).FontSize(ui.Fonts.SM()).FontWeight(500).
				TextColor(v.pal.Text4).Grow(1).SingleLine()
			countBadge(c, v, len(g.Items))
			groupAdd(c, v, "新建会话", func() {
				if v.acts != nil && v.acts.CreateThreadIn != nil {
					v.acts.CreateThreadIn(g.WorkspaceID)
				}
			})
		})
	if !open {
		return
	}
	s.groupItems(c, v, g)
}

// orphanGroup 画「其他」组：归档图标组头，条目全量展示（React 版无分页）。
func (s *Sidebar) orphanGroup(c *mygo.Context, v *viewSide, g Group) {
	mygo.Row(c).AlignItems(mygo.Center).Gap(ui.Space3).Padding(groupHeadPad.t, groupHeadPad.r, groupHeadPad.b).
		Children(func() {
			mygo.Box(c).Width(18) // 对齐有开关的组头
			ui.Icon(c, "archive").FontSize(13).TextColor(v.pal.Text4)
			mygo.Text(c, g.Name).FontSize(ui.Fonts.SM()).FontWeight(500).TextColor(v.pal.Text4).Grow(1)
			countBadge(c, v, len(g.Items))
		})
	for _, t := range g.Items {
		s.threadItem(c, v, t)
	}
}

// groupToggle 画组头左侧的折叠开关：18×18 圆角小按钮，图标随展开状态
// 在 down / right 两个登记名间切换；点击调 onToggle。
func groupToggle(c *mygo.Context, v *viewSide, open bool, title, openIcon, closedIcon string, onToggle func()) {
	btn := mygo.ButtonBase(c).Size(18, 18).Center().
		Radius(ui.RadiusBadge).Tooltip(title)
	if btn.Clicked() && onToggle != nil {
		onToggle()
	}
	name := closedIcon
	if open {
		name = openIcon
	}
	btn.Children(func() {
		ui.Icon(c, name).FontSize(13).TextColor(v.pal.Text4)
	})
}

// groupAdd 画组头右侧的 + 按钮（20×20，悬停提亮）。
func groupAdd(c *mygo.Context, v *viewSide, title string, onClick func()) {
	btn := mygo.ButtonBase(c).Size(20, 20).Center().
		Radius(ui.RadiusChip).Tooltip(title)
	hover := btn.Hovered()
	if hover {
		btn.Background(v.pal.BgHover)
	}
	if btn.Clicked() && onClick != nil {
		onClick()
	}
	color := v.pal.Text3
	if hover {
		color = v.pal.Text
	}
	btn.Children(func() {
		ui.Icon(c, "plus").FontSize(14).TextColor(color)
	})
}

// countBadge 画组头数量徽章（.thread-group-count）：min-width 16、高 15、
// 圆角 4、BgActive 底、XS 字号。
func countBadge(c *mygo.Context, v *viewSide, n int) {
	mygo.Box(c).MinWidth(16).Height(15).
		Padding(0, ui.Space4).Center().Radius(ui.RadiusBadge).
		Background(v.pal.BgActive).
		Children(func() {
			mygo.Text(c, strconv.Itoa(n)).FontSize(ui.Fonts.XS()).FontWeight(500).TextColor(v.pal.Text3)
		})
}

// listEmpty 画空态文案（.thread-empty：26 16 内边距、居中、次要色）。
func listEmpty(c *mygo.Context, v *viewSide, text string) {
	mygo.Box(c).Fill().Center().
		Padding(ui.Space6+ui.Space7+ui.Space4, ui.Space6+ui.Space7+ui.Space1). // 26 16
		Children(func() {
			mygo.Text(c, text).FontSize(ui.Fonts.SM()).TextColor(v.pal.Text3).
				TextAlign(mygo.Center)
		})
}

// threadItem 画一个会话行（.thread-item）：两行布局（标题；时间 + 状态），
// 选中态模式色竖条 + 悬停/选中底色，右键菜单三项操作。
func (s *Sidebar) threadItem(c *mygo.Context, v *viewSide, t Thread) {
	active := t.ID == s.ActiveID()
	btn := mygo.ButtonBase(c).Fill().AlignItems(mygo.Stretch).Key(t.ID).
		Padding(ui.Space4, ui.Space1+3, ui.Space4, ui.Space5+1). // 8 5 8 11
		Radius(ui.RadiusCtl).
		Margin(0, 0, 1, 0) // 行间距 1（.thread-item margin-bottom）
	if active {
		btn.Background(v.pal.SidebarHover).BorderWidth(1).BorderColor(v.pal.Border)
	} else if btn.Hovered() {
		btn.Background(v.pal.SidebarHover)
	}
	if btn.Clicked() {
		s.Select(t.ID)
		if v.acts != nil && v.acts.SelectThread != nil {
			v.acts.SelectThread(t.ID)
		}
	}
	threadMenu(c, v.acts, btn, t)
	btn.Children(func() {
		// 选中态模式色竖条（.thread-item::before）：左缘 2px、上下缩进 9、
		// 圆角 2；出现/消失走 160ms 透明度渐变（CSS 的 opacity 过渡）。
		bar := mygo.Box(c).Absolute().Left(0).Top(9).Bottom(9).
			Width(2).Radius(ui.RadiusBadge)
		alpha := bar.AnimateTo("bar", boolAlpha(active), mygo.Animation{Duration: ui.Duration, Ease: ui.Ease})
		bar.Background(v.accent).Opacity(alpha)

		mygo.Text(c, t.Title).FontSize(ui.Fonts.Base()).
			TextColor(v.pal.Text).SingleLine()
		mygo.Row(c).Fill().AlignItems(mygo.Center).Justify(mygo.SpaceBetween).Gap(ui.Space3).
			Children(func() {
				mygo.Text(c, FormatTime(t.UpdatedAt, nowMillis())).
					FontSize(ui.Fonts.SM()).TextColor(v.pal.Text4)
				statusNode(c, v, t.Status)
			})
	})
}

// boolAlpha 把布尔转为动画目标透明度。
func boolAlpha(on bool) float32 {
	if on {
		return 1
	}
	return 0
}

// statusNode 画行尾状态（.thread-status）：运行脉冲点 / 等待黄 / 错误红 /
// 完成灰。脉冲用 Loop 驱动透明度往复（CSS keyframes pulse 的移植：
// 1.4s ease-in-out，1 ↔ 0.45）。
func statusNode(c *mygo.Context, v *viewSide, status string) {
	switch StatusOf(status) {
	case StatusRunning:
		statusBadge(c, v, v.accent, "运行中", true)
	case StatusAwaiting:
		statusBadge(c, v, v.pal.Warn, "等待审批", false)
	case StatusError:
		statusBadge(c, v, v.pal.Danger, "错误", false)
	default:
		mygo.Text(c, "已完成").FontSize(ui.Fonts.XS()).FontWeight(500).TextColor(v.pal.Text4)
	}
}

// statusBadge 画「圆点 + 文案」的状态徽标；pulse 时圆点透明度往复。
func statusBadge(c *mygo.Context, v *viewSide, color mygo.Color, label string, pulse bool) {
	mygo.Row(c).AlignItems(mygo.Center).Gap(ui.Space4).Children(func() {
		dot := mygo.Box(c).Size(5, 5).Radius(ui.RadiusRound).Background(color)
		if pulse {
			dot.Opacity(1 - 0.55*dot.Loop("pulse", 1400*time.Millisecond, mygo.Bounce(mygo.EaseInOut)))
		}
		mygo.Text(c, label).FontSize(ui.Fonts.XS()).FontWeight(500).TextColor(color)
	})
}

// threadMenu 挂会话右键菜单（React 版会话操作菜单的三项）。
// 暂用系统菜单；07 号票统一浮层体系后替换为应用内 popover（标题 + 描述）。
//
// 复制动作必须走注入的 Actions.CopyThreadID：mygo 剪贴板要经主循环
// （onMain），在 view 帧内直调会死锁，app.go 里经 win.Update 排队执行。
func threadMenu(c *mygo.Context, acts *Actions, el mygo.Element, t Thread) {
	el.ContextMenu(func(m *mygo.Menu) {
		if m.Item("复制会话 Id").Chosen() {
			if acts != nil && acts.CopyThreadID != nil {
				acts.CopyThreadID(t.ID)
			}
		}
		if m.Item("重命名").Chosen() {
			if acts != nil && acts.ShowRename != nil {
				acts.ShowRename(t)
			}
		}
		m.Separator()
		if m.Item("删除").Chosen() {
			if acts != nil && acts.DeleteThread != nil {
				acts.DeleteThread(t.ID)
			}
		}
	})
}

// ---------------------------------------------------------------------------
// 底部两行
// ---------------------------------------------------------------------------

// footer 画底部两行（.sidebar-foot：上描边、6 8 8 内边距、行间 2）：
// 用量 / 审计；命令面板 ⌘K / 设置 ⌘,。
func (s *Sidebar) footer(c *mygo.Context, v *viewSide) {
	mygo.Column(c).
		Padding(ui.Space3, ui.Space4+4, ui.Space4+4).
		Gap(ui.Space1).
		BorderWidth(1, 0, 0, 0).
		BorderColor(v.pal.Border).
		Background(v.pal.SidebarBg).
		Children(func() {
			mygo.Row(c).AlignItems(mygo.Center).Gap(ui.Space4).Children(func() {
				sideLink(c, v, "zap", "用量", strconv.FormatInt(s.counts.UsageTokens, 10), "", func() {
					if v.acts != nil && v.acts.Usage != nil {
						v.acts.Usage()
					} else {
						c.Toast("用量视图" + stubNote)
					}
				})
				sideLink(c, v, "shield-check", "审计", strconv.Itoa(s.counts.AuditItems), "", func() {
					if v.acts != nil && v.acts.Audit != nil {
						v.acts.Audit()
					} else {
						c.Toast("审计视图" + stubNote)
					}
				})
			})
			mygo.Row(c).AlignItems(mygo.Center).Gap(ui.Space4).Children(func() {
				sideLink(c, v, "command", "命令面板", "", "⌘K", func() {
					if v.acts != nil && v.acts.Palette != nil {
						v.acts.Palette()
					} else {
						c.Toast("命令面板" + stubNote)
					}
				})
				sideLink(c, v, "settings", "设置", "", "⌘,", func() {
					if v.acts != nil && v.acts.Settings != nil {
						v.acts.Settings()
					} else {
						c.Toast("设置" + stubNote)
					}
				})
			})
		})
}

// sideLink 画一个底部入口（.side-link：28 高、圆角 6、图标 14 + 名称 +
// 右侧计数或快捷键徽章；悬停提亮）。
func sideLink(c *mygo.Context, v *viewSide, icon, label, value, kbd string, onClick func()) {
	btn := mygo.ButtonBase(c).Grow(1).AlignItems(mygo.Center).
		Height(ui.HeightSideLink).
		Padding(0, ui.Space4+3).
		Gap(ui.Space3).
		Radius(ui.RadiusRow)
	hover := btn.Hovered()
	if hover {
		btn.Background(v.pal.SidebarHover)
	}
	if btn.Clicked() && onClick != nil {
		onClick()
	}
	color := v.pal.Text3
	if hover {
		color = v.pal.Text2
	}
	btn.Children(func() {
		ui.Icon(c, icon).FontSize(14).TextColor(color)
		mygo.Text(c, label).FontSize(ui.Fonts.SM()).TextColor(color).Grow(1).SingleLine()
		if value != "" {
			mygo.Text(c, value).FontSize(ui.Fonts.SM()).TextColor(v.pal.Text4)
		}
		if kbd != "" {
			kbdBadge(c, v, kbd)
		}
	})
}

// kbdBadge 画键位小徽章（.kbd：min-width 19、高 19、描边底边加粗、XS）。
func kbdBadge(c *mygo.Context, v *viewSide, label string) {
	mygo.Box(c).MinWidth(ui.HeightKbd).Height(ui.HeightKbd).
		Padding(0, ui.Space1+3).
		Center().
		Radius(ui.RadiusBadge).
		Background(v.pal.BgElevated).
		BorderWidth(1, 1, 2, 1).
		BorderColor(v.pal.BorderStrong).
		Children(func() {
			mygo.Text(c, label).FontSize(ui.Fonts.XS()).FontWeight(500).TextColor(v.pal.Text3)
		})
}

// ---------------------------------------------------------------------------
// 重命名浮层（07 号票前的简易实现）
// ---------------------------------------------------------------------------

// renameModal 画重命名浮层：ui.Modal + TextInput，保存即调服务。
// TODO(07): 换成统一浮层体系（对照 React 版 askPrompt 的标题 + 说明文案）。
func (s *Sidebar) renameModal(c *mygo.Context, v *viewSide) {
	if !s.rename.Open {
		return
	}
	mygo.Modal(c, &s.rename.Open, func() {
		mygo.Text(c, "重命名会话").Bold().FontSize(ui.Fonts.LG())
		mygo.Text(c, "新标题会立即保存。").FontSize(ui.Fonts.SM()).TextColor(v.pal.Text2)
		input := mygo.TextInput(c, &s.rename.Title).Placeholder("会话标题")
		submitted := input.Submitted()
		mygo.Row(c).Gap(ui.Space4 + 4).Justify(mygo.End).Children(func() {
			if mygo.Button(c, "取消").Clicked() {
				s.CloseRename()
			}
			if mygo.PrimaryButton(c, "保存").Clicked() || submitted {
				s.submitRename(v)
			}
		})
	})
}

// submitRename 提交重命名：空标题按取消处理（sessionActions 语义）。
func (s *Sidebar) submitRename(v *viewSide) {
	title := strings.TrimSpace(s.rename.Title)
	if title == "" {
		s.CloseRename()
		return
	}
	id := s.rename.ID
	s.CloseRename()
	if v.acts != nil && v.acts.RenameThread != nil {
		v.acts.RenameThread(id, title)
	}
}

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

// nowMillis 返回当前毫秒时间戳（列表时间列的「今天」基准）。
func nowMillis() int64 { return time.Now().UnixMilli() }
