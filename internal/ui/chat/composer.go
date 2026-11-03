// composer.go 是消息输入区：ui.TextArea（2→10 行自适应）、Enter 发送 /
// Shift+Enter 换行（OnTextKey）、运行中的引导（steer）形态、停止按钮
//（双击 Esc armed）、附件条（chip 可移除）与三个附件入口：
//   - 拖放：元素 DroppedFiles（整个聊天列都接收投放）；
//   - 粘贴：OnPaste + mygo.Clipboard.ReadImage → AppService.SavePastedAttachment；
//   - 拾取：Actions.PickFiles（mygo.Dialog.Open，goroutine）。
//
// 12MB 与视觉模型拦截提示对齐 React 版（stores/threads.ts submit 与
// lib/paste.ts 的口径）。
//
// 模型切换 / 审批档位 / 上下文详情三浮层与 R0–R4 说明入口在 11 号票接线：
// composerBar 三处展示可点击，浮层本体与保存路径在 internal/ui/overlays
//（与设置中心同源，双向同步）。

package chat

import (
	"strconv"
	"strings"
	"time"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/overlays"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// escArmWindow 是双击 Esc 的确认窗口（AppShell.tsx 同值）：第一击进入
// 待确认后，窗口内再按一次才真正停止。
const escArmWindow = 800 * time.Millisecond

// composerArea 画输入区与下方的快捷键提示（.composer-wrap）。外框与
// 消息正文列同参数限宽（80% + 保底，两段上下对齐）；聚焦态高亮整个
// 外框（描边切模式强调色），失焦回中性描边。
func (s *Chat) composerArea(c *mygo.Context, v *viewSide) {
	wrap := mygo.Column(c).FillWidth().Padding(theme.Space3, 0, 9, 0).
		AlignItems(mygo.Center).Background(v.pal.ChatBg).
		Draw(func(_ *mygo.Painter, r mygo.Rect) { s.lastWrapW = r.W })

	// 拖放附件：整个聊天列都接收投放（React 版 onDragOver 在 .chat-pane 上）。
	if paths := wrap.DroppedFiles(); len(paths) > 0 {
		s.AddAttachments(paths)
	}

	wrap.Children(func() {
		// 输入框容器（zai 输入壳，03-composer.md）：恒定 80% 居中（与消息
		// 列同参数，用户给定观感约定）、16px 圆角、0.5px 描边、bg-input 白底、
		// 软阴影；聚焦描边灰加深（zai 聚焦不变色不发光）。宽度显式计算
		//（包裹层上一帧实测，首帧窗口宽近似）——mygo 的 WidthPercent 首帧
		// 求解对构建时序敏感，显式宽度绕开。
		winW, _ := c.Size()
		chatW := s.lastWrapW
		if chatW <= 0 {
			chatW = winW
		}
		frame := mygo.Box(c).Width(chatColumnWidth(chatW)).Radius(16).
			Background(v.pal.BgInput).
			Shadow(0, 6, 18, 0, v.pal.ComposerShadow).
			Draw(func(_ *mygo.Painter, r mygo.Rect) { s.lastComposerW = r.W })
		// 聚焦态：焦点在内部任意可编辑区（输入框）即算，帧内记录供走查。
		s.composerFocused = frame.FocusWithin()
		frame.BorderWidth(0.5).BorderColor(composerBorder(v.pal, s.composerFocused))
		frame.Children(func() {
			mygo.Column(c).FillWidth().Children(func() {
				s.attachStrip(c, v)
				s.inputRow(c, v)
				s.composerBar(c, v)
			})
		})
	})
}

// composerBorder 返回 composer 输入壳的描边色：聚焦态灰加深（zai 主题
// 聚焦不变色不发光，styles.css:557,722），失焦常态描边。
func composerBorder(pal *theme.Palette, focused bool) mygo.Color {
	if focused {
		return pal.ComposerBorderFocus
	}
	return pal.ComposerBorder
}

// ComposerFocused 报告 composer 外框是否处于聚焦态（最近一帧的
// FocusWithin 记录；ui.Tester 走查用）。
func (s *Chat) ComposerFocused() bool { return s.composerFocused }

// attachStrip 画待发送附件条（.attach-strip：chip 可移除）。
func (s *Chat) attachStrip(c *mygo.Context, v *viewSide) {
	attachments := s.Attachments()
	if len(attachments) == 0 {
		return
	}
	mygo.Row(c).FillWidth().Gap(theme.Space3).Padding(9, theme.Space5, 0, theme.Space5).Children(func() {
		for _, att := range attachments {
			entry := mygo.Row(c).Gap(2).AlignItems(mygo.Center)
			entry.Children(func() {
				// 图片附件画缩略图（点击放大），其余画文件 chip。
				s.attachItem(c, v, att.Path, att.Name)
				// 移除只从列表删，不删磁盘文件。
				remove := mygo.ButtonBase(c).Size(18, 18).Radius(theme.RadiusBadge).Center().
					Label("移除 " + att.Name)
				if remove.Clicked() {
					s.RemoveAttachment(att.Path)
				}
				if remove.Hovered() {
					remove.Background(v.pal.BgHover)
				}
				remove.Children(func() {
					icons.Icon(c, "x").FontSize(11).TextColor(v.pal.Text4)
				})
			})
		}
	})
}

// inputRow 画输入框：单行起步（ZCode min-h-10，LexicalChatInput.tsx:1601
// 的 1→约 5 行自适应）；Enter（含 ⌘/⌃）发送或引导，Shift+Enter 换行；
// Esc 双击停止；粘贴图片转附件。底色透明让位输入壳、自带描边关掉
// （组件 styleFn 每帧只改描边色，构建期宽度置 0 即无框）——zai 输入壳
// 是一体的白底卡片，聚焦表达在外框描边。
func (s *Chat) inputRow(c *mygo.Context, v *viewSide) {
	mygo.Box(c).FillWidth().Padding(10, 12, 2, 12).Children(func() {
		placeholder := "向 CraftWork 描述你的任务…"
		if v.mode == theme.ModeWork {
			placeholder = "描述要处理的文档或办公任务…"
		}
		input := mygo.TextArea(c, s.DraftRef()).
			FillWidth().
			Placeholder(placeholder).
			PlaceholderColor(v.pal.Text4).
			Lines(1, 8).
			Background(mygo.Transparent).
			Border(0, v.pal.Border).
			FocusRing(false).
			AutoFocus()
		input.OnTextKey(func(k mygo.TextKey) bool {
			return s.textKey(v, k)
		})
		input.OnPaste(func(text string) bool {
			return s.paste(text)
		})
	})
}

// textKey 是 OnTextKey 的分流：Enter 提交、双击 Esc 停止；其余按键
// （含 Shift+Enter 换行）交还输入框。
//
// OnTextKey 在帧间于主线程触发：这里的读写在主线程约定之内；
// k.Text() 是输入框当前文本（可能领先于绑定值），提交用它。
func (s *Chat) textKey(v *viewSide, k mygo.TextKey) bool {
	switch k.Key {
	case mygo.KeyEnter:
		// Shift+Enter 换行；裸 Enter 与 ⌘/⌃+Enter 都提交
		//（assistant-ui submitMode=enter + ⌘Enter 同路）。
		if k.Mods == mygo.Shift {
			return false
		}
		text := strings.TrimSpace(k.Text())
		attachments := s.Attachments()
		if text == "" && len(attachments) == 0 {
			return true // 空输入吞掉回车，不插换行
		}
		s.draft = text
		s.submit()
		k.SetText("")
		return true
	default:
		// Esc 的退出链（运行中双击停止、待审批拒绝）在窗口级处理
		//（HandleWindowEscape，AppShell 的 Esc 链同序）：无论焦点在
		// 输入框还是审批按钮上，Esc 都走同一条链，这里不拦截。
		return false
	}
}

// paste 是 OnPaste 的分流：有文本放行（正常插入）；无文本消费事件并
// 尝试读剪贴板图片转附件。图片读取与落盘在动作内走后台 goroutine——
// 剪贴板与文件写入都不占住按键派发（04 号票：剪贴板调用不进 view 帧）。
// 不支持视觉的模型直接拦截提示（与文件入口同一闸口）。
func (s *Chat) paste(text string) bool {
	if text != "" {
		return false // 纯文本粘贴保持原样
	}
	if !s.supportsVision() {
		s.QueueToast("当前模型不支持图片输入：图片未加入附件，切换到支持视觉的模型后再试")
		return true
	}
	if s.acts == nil || s.acts.PasteImage == nil {
		return false
	}
	s.acts.PasteImage()
	return true
}

// submit 走状态壳的提交入口（Actions.Submit；失败由 client 还原草稿）。
func (s *Chat) submit() { s.Submit() }

// composerBar 画工具条（左动作组、右侧收尾）：三个图标按钮与文字垂直
// 居中对齐（用户指定——zai 的 items-end 在按钮高度不一时中心线错位）。
func (s *Chat) composerBar(c *mygo.Context, v *viewSide) {
	mygo.Row(c).FillWidth().AlignItems(mygo.Center).Gap(theme.Space3).
		Padding(4, 12, 10, 12).Children(func() {
		// 左侧动作组：「+」动作菜单——附件与图片入口收进菜单（ZCode 的
		// + 菜单同构，ChatPromptEditor.tsx:440-441），面板用 PopoverBase。
		plus := mygo.ButtonBase(c).Size(28, 28).Radius(theme.RadiusCtl).Center().
			Label("添加附件或图片")
		if plus.Hovered() {
			plus.Background(v.pal.BgHover)
		}
		plus.Children(func() {
			icons.Icon(c, "plus").FontSize(15).TextColor(v.pal.Text3)
		})
		if plus.Clicked() {
			s.plusOpen = !s.plusOpen
		}
		mygo.PopoverBase(c, plus, &s.plusOpen, func(panel mygo.Element) {
			panel.Width(200).Radius(10).Background(v.pal.BgFloat).
				BorderWidth(1).BorderColor(v.pal.Border).Shadow(0, 6, 18, 0, v.pal.ComposerShadow).
				Padding(theme.Space2).Gap(2).Children(func() {
				s.plusItem(c, v, "paperclip", "添加附件", true)
				s.plusItem(c, v, "image", s.imageToolTitle(), s.supportsVision())
			})
		})
		// 审批档位：点击开档位浮层（保存走设置服务同一路径），旁附 R0–R4
		// 说明入口（11 号票）。浮层是 Popover（15 号票返工）：锚定触发控件、
		// 点击外部 / Esc 关闭，由 overlays 的 ComposerXxxFlyout 挂载。
		level := mygo.ButtonBase(c).Gap(theme.Space2).AlignItems(mygo.Center).
			Height(28).Radius(theme.RadiusCtl).Padding(0, theme.Space2).Label("切换审批档位")
		if level.Hovered() {
			level.Background(v.pal.BgHover)
		}
		level.Children(func() {
			icons.Icon(c, "shield").FontSize(13).TextColor(v.pal.Text3)
			mygo.Text(c, s.levelLabel()).FontSize(theme.Fonts.SM()).TextColor(v.pal.Text3)
			icons.Icon(c, "chevron-down").FontSize(12).TextColor(v.pal.Text4)
		})
		if level.Clicked() {
			overlays.OpenComposerApproval(s.approvalLevelSynced)
		}
		overlays.ComposerApprovalFlyout(c, level)
		infoBtn := s.iconTool(c, v, "info", "风险等级说明", true)
		if infoBtn.Clicked() {
			overlays.OpenComposerRisk()
		}
		overlays.ComposerRiskFlyout(c, infoBtn)
		mygo.Spacer(c)
		// 右侧：上下文占用（点击开详情浮层）+ 模型名（点击开模型选择）+ 发送位。
		s.contextHint(c, v)
		pick := mygo.ButtonBase(c).Gap(theme.Space2).AlignItems(mygo.Center).
			Height(28).Radius(theme.RadiusCtl).Padding(0, theme.Space2).Label("切换模型")
		if pick.Hovered() {
			pick.Background(v.pal.BgHover)
		}
		pick.Children(func() {
			// 空态收口（15 号票，对照 React 版 Composer.tsx）：无会话模型
			// 时不画强调色圆点、显示灰字占位——「蓝点 + 空文本」的孤点观感
			// 即由此而来。React 版按可用模型数区分「未配置模型 / 选择模型」，
			// 原生拿不到该计数，统一用动作语义的「选择模型」（点击本按钮
			// 打开的选择浮层自带空态指引）。
			if name := s.modelLabel(); name != "" {
				mygo.Box(c).Size(6, 6).Radius(theme.RadiusRound).Background(v.acc.Accent)
				mygo.Text(c, name).FontSize(theme.Fonts.SM()).TextColor(v.pal.Text2)
			} else {
				mygo.Text(c, "选择模型").FontSize(theme.Fonts.SM()).TextColor(v.pal.Text4)
			}
			icons.Icon(c, "chevron-down").FontSize(12).TextColor(v.pal.Text4)
		})
		if pick.Clicked() {
			// 勾选态优先当前会话模型；无会话模型时浮层内部退默认模型。
			// 点选后双写：默认模型（浮层内 save）+ 当前会话模型（回调）。
			overlays.OpenComposerModels(s.Thread().ModelID, func(ref string) {
				if s.acts != nil && s.acts.SetThreadModel != nil {
					s.acts.SetThreadModel(s.Thread().ID, ref)
				}
			})
		}
		overlays.ComposerModelsFlyout(c, pick)
		s.sendCluster(c, v)
	})
}

// plusItem 画「+」动作菜单里的一项；禁用项淡化且不可点。两项都走
// PickFiles（图片入口与附件同一拾取器，粘贴链路单独处理）。
func (s *Chat) plusItem(c *mygo.Context, v *viewSide, icon, label string, enabled bool) {
	item := mygo.ButtonBase(c).FillWidth().Height(32).Radius(theme.RadiusRow).
		Padding(0, theme.Space3).AlignItems(mygo.Center).Gap(theme.Space3).Label(label)
	if enabled && item.Hovered() {
		item.Background(v.pal.BgHover)
	}
	item.Children(func() {
		color := v.pal.Text2
		if !enabled {
			color = v.pal.Text4
		}
		icons.Icon(c, icon).FontSize(14).TextColor(color)
		mygo.Text(c, label).FontSize(theme.Fonts.SM()).TextColor(color).SingleLine()
	})
	if item.Clicked() && enabled {
		s.plusOpen = false
		if s.acts != nil && s.acts.PickFiles != nil {
			s.acts.PickFiles()
		}
	}
}

// imageToolTitle 返回图片按钮的语义（禁用原因在 title 里，对齐 React 版）。
func (s *Chat) imageToolTitle() string {
	if s.supportsVision() {
		return "添加图片"
	}
	return "当前模型不支持图片"
}

// iconTool 画一枚 24px 图标工具按钮，返回按钮元素（调用方用 Clicked()
// 判点击；要挂锚定浮层时元素作 Popover 锚点）。enabled 为 false 时不注册
// 点击、淡化显示（禁用态）。
func (s *Chat) iconTool(c *mygo.Context, v *viewSide, icon, title string, enabled bool) mygo.Element {
	btn := mygo.ButtonBase(c).Size(24, 24).Radius(theme.RadiusCtl).Center().Label(title)
	if enabled {
		if btn.Hovered() {
			btn.Background(v.pal.BgHover)
		}
	}
	btn.Children(func() {
		color := v.pal.Text3
		if !enabled {
			color = v.pal.Text4
		}
		icons.Icon(c, icon).FontSize(14).TextColor(color)
	})
	return btn
}

// contextHint 画上下文占用：圆环 + 百分比文本 + 速度（<50% 灰、50–80% 黄、
// ≥80% 红；与审批告警色同源）。速度生成中用实时估算（字符/4 ÷ 秒），
// 空闲时显示最近一轮的后端实测值。点击开详情浮层（11 号票：占用口径、
// 会话累计、缓存命中、速度的分解展示）。
//
// 空态收口（15 号票，对照 React 版）：无会话模型时整块不渲染
// （React 版 currentModel ? <ctx-hint> : null 同口径）——没有模型时占用
// 环、0% 与速度占位「—」拼成的「○ 0% —」只是噪音；从未产生速度
// （shown ≤ 0）时速度位不画「—」（详情浮层内的键值占位不受影响）。
func (s *Chat) contextHint(c *mygo.Context, v *viewSide) {
	if s.Thread().ModelID == "" {
		return
	}
	used, max, speed := s.contextStats()
	if max <= 0 {
		return
	}
	percent := int(used * 100 / max)
	color := v.pal.Text3
	switch {
	case percent >= 80:
		color = v.pal.Danger
	case percent >= 50:
		color = v.pal.Warn
	}
	// 实时速度估算（Composer.tsx liveSpeed 同口径）：统计当前流式回复的
	// 字符数，按每 4 字符约 1 token 折算，除以自首个增量起的秒数；
	// 流式结束清零，由最近一轮的真实速度（thread.contextSpeed）接管。
	live := s.liveSpeed()
	shown := speed
	if s.Running() && live > 0 {
		shown = live
	}
	hint := mygo.ButtonBase(c).Gap(theme.Space2).AlignItems(mygo.Center).
		Radius(theme.RadiusCtl).Padding(2, theme.Space2).Label("上下文详情")
	if hint.Hovered() {
		hint.Background(v.pal.BgHover)
	}
	hint.Children(func() {
		contextRing(c, 14, 2, float32(percent)/100, v.pal.Border, color)
		mygo.Text(c, strconv.Itoa(percent)+"%").FontSize(theme.Fonts.XS()).TextColor(color)
		if shown > 0 {
			mygo.Text(c, strconv.Itoa(int(shown))+" tok/s").FontSize(theme.Fonts.XS()).TextColor(v.pal.Text3)
		}
	})
	if hint.Clicked() {
		overlays.OpenComposerContext(s.contextDetail(used, max, shown))
	}
	overlays.ComposerContextFlyout(c, hint)
}

// liveSpeed 返回运行中流式输出的实时速度估算；没有流式正文时归零。
func (s *Chat) liveSpeed() float64 {
	characters := 0
	for _, m := range s.slice.Messages {
		if m.Role != RoleAssistant || m.Status != StatusStreaming {
			continue
		}
		for i := range m.Parts {
			if m.Parts[i].Kind == PartText {
				characters += len([]rune(strOf(m.Parts[i].Data, "text")))
			}
		}
	}
	if characters == 0 {
		s.streamStartMs = 0
		return 0
	}
	if s.streamStartMs == 0 {
		s.streamStartMs = theme.Now()
	}
	elapsed := float64(theme.Now()-s.streamStartMs) / 1000
	if elapsed <= 0 {
		return 0
	}
	return float64(characters) / 4 / elapsed
}

// contextRing 画上下文占用圆环（ContextRing.tsx 的移植）：SVG dasharray
// 语义用 mygo 的 Draw 弧线实现——底环整圆描边，进度弧按 ratio 钳制后从
// 12 点方向顺时针描边，颜色三档（调用方给色）。
func contextRing(c *mygo.Context, size, strokeWidth float32, ratio float32, base, color mygo.Color) {
	box := mygo.Box(c).Size(size, size)
	radius := (size - strokeWidth) / 2
	if ratio < 0 {
		ratio = 0
	}
	if ratio > 1 {
		ratio = 1 // 超出窗口时画满环（红色档自然生效）
	}
	box.Draw(func(p *mygo.Painter, r mygo.Rect) {
		cx, cy := r.X+r.W/2, r.Y+r.H/2
		// 底环：完整圆（SVG circle 底描边同位）。
		var full mygo.Path
		full.Circle(cx, cy, radius)
		p.StrokePath(&full, strokeWidth, base)
		if ratio <= 0 {
			return
		}
		// 进度弧：Trim 按比例取圆的一部分，旋转 -90° 让起点在 12 点方向
		//（ContextRing 的 rotate(-90) 同语义）。
		var arc mygo.Path
		arc.Circle(cx, cy, radius)
		arc.Trim(0, ratio)
		p.Transform(mygo.Rotation(-90), cx, cy, func() {
			p.StrokePath(&arc, strokeWidth, color)
		})
	})
}

// sendCluster 画发送位（zai 形态：32px 圆形，ChatPromptEditor.tsx:487-495）：
// 空闲是发送（brand 反色底——亮色黑底白箭头）；运行中按输入内容切换——
// 有内容是「引导」（corner-down-left），空输入保持停止（SendCluster.tsx
// 同语义，停止键 ghost 圆形 Square 实心）。
func (s *Chat) sendCluster(c *mygo.Context, v *viewSide) {
	if s.Running() {
		if strings.TrimSpace(s.draft) != "" || len(s.Attachments()) > 0 {
			// 引导按钮：中性底、悬停变强调色。
			btn := mygo.ButtonBase(c).Size(32, 32).Radius(theme.RadiusRound).Center().
				Label("作为引导发送").
				Background(v.pal.BgActive)
			if btn.Clicked() {
				s.submit()
			}
			if btn.Hovered() {
				btn.Background(v.acc.Soft)
			}
			btn.Children(func() {
				icons.Icon(c, "corner-down-left").FontSize(14).TextColor(v.pal.Text2)
			})
			return
		}
		// 停止按钮：armed 时强提示（对齐 React is-armed 的视觉暗示）。
		btn := mygo.ButtonBase(c).Size(32, 32).Radius(theme.RadiusRound).Center().
			Label("停止生成").
			Background(v.pal.Brand)
		if btn.Clicked() {
			s.DisarmInterrupt()
			s.SetRunning(false)
			if s.acts != nil && s.acts.Cancel != nil {
				s.acts.Cancel()
			}
		}
		if s.InterruptArmed() {
			btn.BorderWidth(1).BorderColor(v.pal.Danger)
		}
		btn.Children(func() {
			icons.Icon(c, "stop").FontSize(14).TextColor(v.pal.OnBrand)
		})
		return
	}
	// 发送按钮：brand 圆形（亮色黑底白箭头）；无模型时禁用（降透明，
	// 悬停原因见 Label）。
	btn := mygo.ButtonBase(c).Size(32, 32).Radius(theme.RadiusRound).Center().
		Label("发送")
	hasModel := s.modelLabel() != ""
	if hasModel {
		btn.Background(v.pal.Brand)
		if btn.Clicked() {
			s.submit()
		}
	} else {
		btn.Label("发送：请先在设置中添加提供商并选择模型")
		btn.Background(v.pal.Brand.Alpha(0.35))
	}
	btn.Children(func() {
		color := v.pal.OnBrand
		if !hasModel {
			color = v.pal.Text4
		}
		icons.Icon(c, "arrow-up").FontSize(14).TextColor(color)
	})
}

// ---------------------------------------------------------------------------
// Composer 的只读派生
// ---------------------------------------------------------------------------

// supportsVision 报告当前会话模型是否支持图片输入；模型目录查不到或
// 没有会话模型时按不支持处理（与 React 版 caps.vision ?? false 一致）。
func (s *Chat) supportsVision() bool {
	ref := s.Thread().ModelID
	if ref == "" {
		return false
	}
	_, info, ok := modelByRef(ref)
	return ok && info.Caps.Vision
}

// contextStats 返回上下文占用的实测值与窗口大小、最近速度：
// 实测优先（thread.contextTokens），没有调用记录时按「后端累计 token +
// 当前正文每 4 字符约 1 token」估算（Composer.tsx 同口径）；窗口大小取
// 模型目录的 contextK，缺省 32k。
func (s *Chat) contextStats() (used, max float64, speed float64) {
	th := s.Thread()
	contextK := 32
	if th.ModelID != "" {
		if _, info, ok := modelByRef(th.ModelID); ok && info.ContextK > 0 {
			contextK = info.ContextK
		}
	}
	if th.ContextTokens > 0 {
		return float64(th.ContextTokens), float64(contextK) * 1000, th.ContextSpeed
	}
	characters := 0
	for _, m := range s.Messages() {
		for i := range m.Parts {
			if m.Parts[i].Kind == PartText {
				characters += len([]rune(strOf(m.Parts[i].Data, "text")))
			}
		}
	}
	return float64(th.Tokens) + float64(characters)/4, float64(contextK) * 1000, th.ContextSpeed
}

// modelLabel 返回当前模型的显示名；未配置时返回空串（发送按钮禁用）。
func (s *Chat) modelLabel() string {
	th := s.Thread()
	if th.ModelName != "" {
		return th.ModelName
	}
	if th.ModelID == "" {
		return ""
	}
	if _, info, ok := modelByRef(th.ModelID); ok {
		return info.Name
	}
	return ""
}

// levelLabel 返回审批档位的中文短标签（设置服务在装配时同步进来）。
func (s *Chat) levelLabel() string { return s.approvalLevel }

// approvalLevelSynced 是档位浮层保存成功后的同步回调（帧内执行）：把新
// 档位同步进输入区只读展示，语义与装配时的 SetApprovalLevel 一致；中文
// 短标签与审计说明共用 service.ApprovalLevelName 一张表。
func (s *Chat) approvalLevelSynced(level string) {
	s.approvalLevel = service.ApprovalLevelName(level)
}

// contextDetail 组装上下文详情快照（contextHint 同口径）：实测优先、估算
// 兜底；速度运行中取实时估算。浮层只展示快照，不回读会话状态。
func (s *Chat) contextDetail(used, max, speed float64) overlays.ContextDetail {
	percent := 0
	if max > 0 {
		percent = int(used * 100 / max)
	}
	return overlays.ContextDetail{
		Used:        int64(used),
		WindowK:     int(max / 1000),
		Percent:     percent,
		Tokens:      s.Thread().Tokens,
		CacheTokens: s.Thread().ContextCacheTokens,
		Speed:       speed,
	}
}
