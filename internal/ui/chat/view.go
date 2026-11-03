// view.go 是聊天列的立即模式 view：会话头部（标题 + 徽章）、消息流
// （ui.List 平滑跟底）、空会话引导与「回到底部」。视觉取值对照
// docs/specs/interface.md 第 4 节与 frontend/src/styles/chat.css（CSS 值
// 在注释里保留对照）。
//
// 槽位纪律：本文件只画聊天列内部；三栏外壳（shell.go）经 Shell 的
// ChatView 字段把自适应列的绘制委托进来。
//
// 范围约定：会话头部的操作按钮（面板开关、重命名、复制、归档）归 08 /
// 侧栏右键菜单；非文本部件的完整卡片归 06 号票，本票画最小占位卡
//（parts.go）保证消息流结构完整。

package chat

import (
	"fmt"
	"strings"
	"time"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// suggestions 是空会话时按模式展示的快捷提示句（ChatPane.tsx 同文案）。
var suggestions = map[theme.Mode][]string{
	theme.ModeCode: {"解释这个仓库的结构", "运行测试并修复失败用例", "为当前分支写一份变更摘要"},
	theme.ModeWork: {"总结这份文档的要点", "把会议纪要整理成待办表", "草拟一封回复邮件"},
}

// viewSide 是 view 帧内的一次性派生：色板、模式强调色与动作入口。
type viewSide struct {
	pal  *theme.Palette
	acc  theme.AccentSet // 当前模式强调色（气泡底、发送按钮、光标）
	mode theme.Mode
	acts *Actions
}

// View 画整个聊天列：头部、消息流、输入区。由 shell.go 的聊天槽位调用，
// pal 来自外壳每帧套用的主题色板。
func (s *Chat) View(c *mygo.Context, pal *theme.Palette) {
	// 异步动作排队的提示在帧开头弹出（c.Toast 只能在 view 帧内调用）。
	if message := s.TakeToast(); message != "" {
		c.Toast(message)
	}

	v := &viewSide{pal: pal, acts: s.acts}
	mode := theme.ModeCode
	if th := s.Thread(); th.Mode == "work" {
		mode = theme.ModeWork
	} else if th.Mode == "" {
		mode = s.fallbackMode
	}
	v.mode = mode
	v.acc = pal.ModeAccent(mode)

	mygo.Column(c).Fill().Background(pal.ChatBg).Children(func() {
		s.chatHead(c, v)
		s.streamArea(c, v)
		s.composerArea(c, v)
	})
	// 附件图片的放大预览浮层（DialogBase 覆盖整窗，画在聊天列槽位内）。
	s.imagePreviewDialog(c, v)
}

// ---------------------------------------------------------------------------
// 会话头部（.chat-head：48px、标题省略、模式与状态徽章）
// ---------------------------------------------------------------------------

// chatHead 画 48px 会话头部。没有会话时画空头部，保证布局高度稳定
// （ChatHeader.tsx 同语义）。背景用 ChatBg 与 BgPanel 的 72% 混合近似
// CSS 的 color-mix；backdrop 模糊省略（下方没有滚动内容穿透的观感差）。
// 14 号票起本行顶到窗口顶，整行是窗口拖拽区（DragWindow，双击缩放）：
// 行内只有标题与徽章、无交互控件，无需 no-drag 划分。
func (s *Chat) chatHead(c *mygo.Context, v *viewSide) {
	head := mygo.Row(c).Height(theme.TopbarHeight).Padding(0, theme.Space6, 0, 18).
		BorderWidth(0, 0, 1, 0).BorderColor(v.pal.Border).
		DragWindow()
	// color-mix(in srgb, chat-bg 72%, bg-panel)：B 占比 28%。
	head.Background(v.pal.ChatBg.Mix(v.pal.BgPanel, 0.28))
	head.Children(func() {
		th := s.Thread()
		if th.ID == "" {
			return
		}
		// 标题：超长省略（定宽行内的 Text 按容器截断）。
		mygo.Text(c, th.Title).FontWeight(600).MaxLines(1).Grow(1)
		// 模式徽章。
		badge(c, v, modeName(v.mode), v.acc.Soft, v.acc.Accent)
		// 状态徽章：只覆盖需要用户注意的三种状态（运行中/等待审批/出错）。
		switch th.Status {
		case ThreadStatusRunning:
			badge(c, v, "运行中", v.acc.Soft, v.acc.Accent)
		case ThreadStatusAwaiting:
			badge(c, v, "等待审批", v.pal.WarnSoft, v.pal.Warn)
		case ThreadStatusErrorStat:
			badge(c, v, "出错", v.pal.DangerSoft, v.pal.Danger)
		}
	})
}

// badge 画一枚小徽章（.badge：软底 + 主色文字）。
func badge(c *mygo.Context, v *viewSide, label string, bg, fg mygo.Color) {
	mygo.Box(c).Height(theme.HeightBadge).Padding(0, theme.Space3).Radius(theme.RadiusBadge).
		Background(bg).Center().
		Children(func() {
			mygo.Text(c, label).FontSize(theme.Fonts.XS()).TextColor(fg).FontWeight(600)
		})
}

// modeName 返回模式徽章文案。
func modeName(m theme.Mode) string {
	if m == theme.ModeWork {
		return "Work"
	}
	return "Code"
}

// ---------------------------------------------------------------------------
// 消息流（.chat-scroll + .chat-stream：正文列居中限宽、22px 上内边距）
// ---------------------------------------------------------------------------

// streamArea 画消息流与「回到底部」浮钮。列表 FollowEnd 平滑跟底；
// 用户上滚停止跟随，此时浮出回底按钮（spec 4.6）。
func (s *Chat) streamArea(c *mygo.Context, v *viewSide) {
	messages := s.Messages()
	s.list.Key = func(i int) any { return messages[i].ID }
	s.list.FollowEnd = true
	s.list.FollowAnimation = &mygo.Smooth

	wrapper := mygo.Box(c).Grow(1).FillWidth().Clip()
	wrapper.Children(func() {
		lst := mygo.List(c, &s.list, len(messages), func(i int) {
			s.messageRow(c, v, messages[i])
		}).Grow(1).Gap(22).Justify(mygo.End).
			Padding(theme.Space7, 0, 26, 0) // chat-stream: 22px 顶 26px 底
		// 空会话引导：标题、说明与按模式区分的快捷提示句（List 无行时的
		// 附加内容，官方文档的空态模式）。
		lst.Children(func() {
			if len(messages) == 0 {
				s.emptyState(c, v)
			}
		})
		// 靠近顶部自动向前翻页（Visible 返回当前可视行区间）。
		if first, _ := s.list.Visible(); first < 5 && s.hasEarlier && !s.loadingEarlier && len(messages) > 0 {
			if s.acts != nil && s.acts.LoadEarlier != nil {
				s.acts.LoadEarlier()
			}
		}
		// 用户翻阅历史时出现回底按钮；点击平滑滚回末尾。绝对定位铺满
		// 底部一行、内容居中（React 版 sticky 底部居中）。
		if !s.list.AtEnd() && len(messages) > 0 {
			overlay := mygo.Box(c).Absolute().Left(0).Right(0).Bottom(14)
			overlay.Children(func() {
				mygo.Row(c).FillWidth().Justify(mygo.Center).Children(func() {
					btn := mygo.ButtonBase(c).Height(theme.HeightBtnSmall).Padding(0, 11).
						Radius(20).
						Background(v.pal.BgFloat).
						BorderWidth(1).BorderColor(v.pal.BorderStrong)
					if btn.Clicked() {
						s.list.ScrollToEnd()
					}
					if btn.Hovered() {
						btn.Background(v.pal.BgHover)
					}
					btn.Children(func() {
						mygo.Row(c).Gap(5).Center().Children(func() {
							icons.Icon(c, "arrow-down").FontSize(12).TextColor(v.pal.Text2)
							mygo.Text(c, "回到底部").FontSize(theme.Fonts.SM()).TextColor(v.pal.Text2)
						})
					})
				})
			})
		}
	})
}

// emptyState 画空会话引导（.chat-empty：模式图标、标题、说明、建议句）。
func (s *Chat) emptyState(c *mygo.Context, v *viewSide) {
	temp := s.tempWorkspace
	title, desc := "Code 模式已就绪", "在当前工作区里读代码、改文件、跑命令、看差异。所有写操作与命令执行都会先经过审批。"
	icon := "code"
	if temp {
		title = "临时会话已就绪"
		desc = "无需选择工作区，直接提问即可；产生的文件会保存在内置临时目录，随时可在侧栏切回工作区会话。"
	} else if v.mode == theme.ModeWork {
		title = "Work 模式已就绪"
		desc = "读取文档、修改 Word 与 Excel、汇总邮件与日历。所有文档写入都会在审批后执行并自动备份。"
		icon = "briefcase"
	}

	mygo.Box(c).Grow(1).FillWidth().Center().Padding(40, 20).Children(func() {
		mygo.Column(c).Gap(theme.Space5).AlignItems(mygo.Center).MaxWidth(380).Children(func() {
			// 模式图标：46px 圆角方块，模式色软底（.empty-mark）。
			mark := mygo.Box(c).Size(46, 46).Radius(13).
				Background(v.acc.Soft).Center()
			mark.BorderWidth(1).BorderColor(v.acc.Line)
			mark.Children(func() {
				icons.Icon(c, icon).FontSize(22).TextColor(v.acc.Accent)
			})
			mygo.Text(c, title).FontSize(theme.Fonts.LG()).FontWeight(600).TextColor(v.pal.Text)
			mygo.Text(c, desc).FontSize(theme.Fonts.SM()).TextColor(v.pal.Text3).TextAlign(mygo.Center)
			mygo.Row(c).Gap(theme.Space3).Justify(mygo.Center).Children(func() {
				for _, suggestion := range suggestions[v.mode] {
					btn := mygo.ButtonBase(c).Height(30).Padding(0, theme.Space6).Radius(theme.RadiusCtl).
						Background(v.pal.BgPanel).BorderWidth(1).BorderColor(v.pal.Border)
					if btn.Clicked() {
						if s.acts != nil && s.acts.Submit != nil {
							s.acts.Submit(suggestion, nil)
						}
					}
					if btn.Hovered() {
						btn.Background(v.acc.Soft).BorderColor(v.acc.Line)
					}
					btn.Children(func() {
						mygo.Text(c, suggestion).FontSize(theme.Fonts.Caption()).TextColor(v.pal.Text2)
					})
				}
			})
		})
	})
}

// ---------------------------------------------------------------------------
// 单条消息（.msg：用户右对齐气泡 / 助手全宽 / 部件顺序渲染）
// ---------------------------------------------------------------------------

// chatColumnWidth 是聊天正文列宽公式，对齐 ZCode conversationLayout.ts:3-7
// 的三档断点（viewport = 窗口宽）：≥1280 内容列 min(1152, 聊天列-384)；
// ≥864 min(896, 聊天列-96)；以下全宽。chatW 是聊天列宽（上一帧实测，首帧
// 调用方用窗口宽近似，第二帧收敛）；保底 theme.ChatColumnMinWidth（Markdown
// 收缩布局塌零宽的兜底）。消息流与 composer 外框共用这条列宽。
func chatColumnWidth(winW, chatW float32) float32 {
	w := chatW
	switch {
	case winW >= 1280:
		w = min(1152, chatW-384)
	case winW >= 864:
		w = min(896, chatW-96)
	}
	if w < theme.ChatColumnMinWidth {
		return theme.ChatColumnMinWidth
	}
	return w
}

// messageRow 画一条消息。正文列先居中限宽（两侧 margin 各 10%、保底见
// chatColumnWidth；显式宽度构建——Markdown 在收缩布局里会塌成零宽，
// 限宽不允许退回纯 MaxWidth 的收缩形态）；列内用户消息右对齐、模式色
// 软底气泡（圆角 12、右下 4、最大宽 78% 相对正文列），助手消息铺满正文
// 列。元信息行常显时间与标签，悬停浮现复制 / 重新生成（悬停区域仍是
// 整条消息行）。
func (s *Chat) messageRow(c *mygo.Context, v *viewSide, m *Message) {
	isUser := m.Role == RoleUser
	row := mygo.Row(c).FillWidth().Justify(mygo.Center)
	// 悬停状态取整条消息（元信息按钮的浮现条件，.msg:hover）。
	hovered := row.Hovered()
	// 消息行铺满聊天列：实测宽是列宽公式的输入（上一帧值）。
	row.Draw(func(_ *mygo.Painter, r mygo.Rect) { s.lastRowW = r.W })
	row.Children(func() {
		// 正文列：宽度 = chatColumnWidth 断点公式（ZCode conversationLayout）；
		// 聊天列宽 = 消息行上一帧实测宽（row 铺满聊天列；首帧以窗口宽近似，
		// 第二帧收敛）。Draw 把实际列宽记给走查断言（s.lastColumnW）。
		winW, _ := c.Size()
		chatW := s.lastRowW
		if chatW <= 0 {
			chatW = winW
		}
		column := mygo.Column(c).Width(chatColumnWidth(winW, chatW)).
			Draw(func(_ *mygo.Painter, r mygo.Rect) { s.lastColumnW = r.W })
		column.Children(func() {
			inner := mygo.Row(c).FillWidth().AlignItems(mygo.Start)
			if isUser {
				inner.Justify(mygo.End)
			}
			inner.Children(func() {
				body := mygo.Column(c)
				if isUser {
					body.MaxWidthPercent(78) // msg.user .msg-body: max-width 78%（相对正文列）
				} else {
					// 助手消息铺满正文列。
					body.FillWidth()
				}
				body.Children(func() {
					s.messageParts(c, v, m, isUser)
					s.messageFoot(c, v, m, isUser, hovered)
				})
			})
		})
	})
}

// messageParts 按顺序画消息部件：用户消息的文本进气泡（附件 chips 在
// 气泡内），助手消息按「过程分组」切段渲染——过程部件（思考 / 工具 /
// 审批 / 待办 / 历史正文）相邻成组，运行中平铺、结束后收进「已工作 N」
// 折叠区；最终正文走 Markdown；错误 / 提示等显性卡片留在折叠区外。
func (s *Chat) messageParts(c *mygo.Context, v *viewSide, m *Message, isUser bool) {
	if isUser {
		for i := range m.Parts {
			part := m.Parts[i]
			switch part.Kind {
			case PartText:
				s.userBubble(c, v, m, strOf(part.Data, "text"))
			case PartNotice:
				noticeStrip(c, v, strOf(part.Data, "text"))
			default:
				partPlaceholder(c, v, part)
			}
			if i < len(m.Parts)-1 {
				mygo.Box(c).Height(theme.Space5)
			}
		}
		if len(m.Parts) == 0 && m.Status == StatusStreaming {
			s.streamCaret(c, v)
		}
		return
	}

	// 助手消息：先切段渲染，再补流式光标。非 work 段里只有最终正文
	//（其余文本部件归过程段）、提示与显性卡片。
	final := finalTextIndex(m)
	for k, seg := range segmentParts(m) {
		if k > 0 {
			partsGap(c)
		}
		if seg.work {
			s.drawSegment(c, v, m, seg)
			continue
		}
		for n, i := range seg.parts {
			if n > 0 {
				partsGap(c)
			}
			part := m.Parts[i]
			switch part.Kind {
			case PartText:
				if i == final {
					if text := strOf(part.Data, "text"); text != "" {
						s.markdownView(c, v.pal, text, m.Status == StatusStreaming)
					}
				}
			case PartNotice:
				noticeStrip(c, v, strOf(part.Data, "text"))
			default:
				s.partCard(c, v, part, false)
			}
		}
	}
	// 生成中：正文末尾闪烁光标（.stream-caret）。
	if m.Status == StatusStreaming {
		s.streamCaret(c, v)
	}
}

// userBubble 画用户消息气泡：模式色软底、1px 模式色描边、圆角 12（右下 4）。
func (s *Chat) userBubble(c *mygo.Context, v *viewSide, m *Message, text string) {
	if text == "" && !hasAttachments(m) {
		return
	}
	bubble := mygo.Column(c).Padding(9, theme.Space6).Radius(12, 12, 4, 12).
		Background(v.acc.Soft)
	bubble.BorderWidth(1).BorderColor(v.acc.Line)
	bubble.Children(func() {
		if text != "" {
			mygo.Text(c, text).FontSize(theme.Fonts.Base())
		}
		if hasAttachments(m) {
			mygo.Box(c).Height(theme.Space2)
			for _, att := range messageAttachments(m) {
				// 图片附件画缩略图（点击放大），其余画文件 chip。
				s.attachItem(c, v, att.Path, att.Name)
			}
		}
	})
}

// messageAttachment 是消息内的一条附件显示信息。
type messageAttachment struct {
	Name string
	Path string
	Size int64
}

// hasAttachments 报告消息的文本部件是否携带附件。
func hasAttachments(m *Message) bool {
	return len(messageAttachments(m)) > 0
}

// messageAttachments 取消息附件的显示名、路径与大小（payload 里可能是
// 路径字符串，也可能是 {name,path,size} 对象，两者都兼容）。
func messageAttachments(m *Message) []messageAttachment {
	var out []messageAttachment
	for i := range m.Parts {
		if m.Parts[i].Kind != PartText {
			continue
		}
		list, ok := m.Parts[i].Data["attachments"].([]any)
		if !ok {
			continue
		}
		for _, item := range list {
			switch a := item.(type) {
			case string:
				out = append(out, messageAttachment{Name: baseName(a), Path: a})
			case map[string]any:
				att := messageAttachment{
					Name: strOf(a, "name"),
					Path: strOf(a, "path"),
					Size: int64(numOf(a, "size")),
				}
				if att.Path != "" {
					att.Name = strOr(att.Name, baseName(att.Path))
				}
				if att.Name != "" {
					out = append(out, att)
				}
			}
		}
	}
	return out
}

func baseName(p string) string {
	if idx := strings.LastIndex(p, "/"); idx >= 0 {
		return p[idx+1:]
	}
	return p
}

func strOr(a, b string) string {
	if a != "" {
		return a
	}
	return b
}

// streamCaret 画流式光标（.stream-caret：6×13 模式色圆角条，1s 闪烁）。
func (s *Chat) streamCaret(c *mygo.Context, v *viewSide) {
	caret := mygo.Box(c).Size(6, 13).Radius(1).Background(v.acc.Accent)
	// 闪烁：Loop 以 1s 周期往复（CSS steps(2,start) 的平滑近似）。
	caret.Opacity(0.35 + 0.65*caret.Loop("caret", time.Second, mygo.Bounce(mygo.EaseInOut)))
}

// noticeStrip 画居中的系统提示细条（「上下文已压缩」等）。
func noticeStrip(c *mygo.Context, v *viewSide, text string) {
	if text == "" {
		return
	}
	mygo.Box(c).FillWidth().Center().Children(func() {
		mygo.Box(c).Padding(3, theme.Space4).Radius(theme.RadiusBadge).
			Background(v.pal.BgHover).
			Children(func() {
				mygo.Text(c, text).FontSize(theme.Fonts.XS()).TextColor(v.pal.Text3)
			})
	})
}

// messageFoot 画元信息行：时间常显、生成中 / 待送达标签、悬停浮现的
// 复制与重新生成（.msg-foot）。
func (s *Chat) messageFoot(c *mygo.Context, v *viewSide, m *Message, isUser bool, hovered bool) {
	foot := mygo.Row(c).Gap(theme.Space3).AlignItems(mygo.Center).Height(theme.Space7)
	if isUser {
		foot.Justify(mygo.End)
	}
	foot.Children(func() {
		mygo.Text(c, formatTime(m.CreatedAt)).FontSize(theme.Fonts.XS()).TextColor(v.pal.Text4)
		if m.Status == StatusStreaming {
			msgTag(c, v, "生成中", v.pal.Border)
		}
		if isUser && m.SteerQueued {
			msgTag(c, v, "待送达", v.acc.Line)
		}
		// 悬停操作：复制（用户与助手都有）与重新生成（仅助手）。
		if hovered {
			if footButton(c, v, "copy", "复制") {
				// 复制只取文本部件，附件与卡片内容不进剪贴板
				//（MessageView.tsx messageText 同语义）。剪贴板要经主循环，
				// 由 client 的 Copy 动作排队执行。
				if s.acts != nil && s.acts.Copy != nil {
					s.acts.Copy(messagePlainText(m))
				}
			}
			if !isUser {
				if footButton(c, v, "refresh", "重新生成") {
					if s.acts != nil && s.acts.Regenerate != nil {
						s.acts.Regenerate()
					}
				}
			}
		}
	})
}

// msgTag 画一枚细描边小标签（.msg-tag：11px、3px 圆角、内边距 4）。
func msgTag(c *mygo.Context, v *viewSide, label string, border mygo.Color) {
	mygo.Box(c).Height(16).Padding(0, theme.Space2).Radius(3).
		BorderWidth(1).BorderColor(border).Center().
		Children(func() {
			mygo.Text(c, label).FontSize(theme.Fonts.XS()).TextColor(v.pal.Text3)
		})
}

// footButton 画元信息行的一个小按钮，返回是否被点击。
func footButton(c *mygo.Context, v *viewSide, icon, label string) bool {
	btn := mygo.ButtonBase(c).Height(theme.HeightMiniBtn).Padding(0, theme.Space3).Radius(theme.RadiusBadge)
	clicked := btn.Clicked()
	if btn.Hovered() {
		btn.Background(v.pal.BgHover)
	}
	btn.Children(func() {
		mygo.Row(c).Gap(theme.Space2).Center().Children(func() {
			icons.Icon(c, icon).FontSize(12).TextColor(v.pal.Text3)
			mygo.Text(c, label).FontSize(theme.Fonts.XS()).TextColor(v.pal.Text3)
		})
	})
	return clicked
}

// messagePlainText 拼接消息全部文本部件（复制按钮的数据源）。
func messagePlainText(m *Message) string {
	var texts []string
	for i := range m.Parts {
		if m.Parts[i].Kind == PartText {
			if t := strOf(m.Parts[i].Data, "text"); t != "" {
				texts = append(texts, t)
			}
		}
	}
	return strings.Join(texts, "\n\n")
}

// formatTime 按前端 formatTime 的口径显示：当天 HH:MM，跨天 MM/DD HH:MM。
func formatTime(ts int64) string {
	if ts <= 0 {
		return ""
	}
	t := time.UnixMilli(ts)
	now := time.Now()
	sameDay := t.Year() == now.Year() && t.Month() == now.Month() && t.Day() == now.Day()
	if sameDay {
		return t.Format("15:04")
	}
	return fmt.Sprintf("%02d/%02d %s", int(t.Month()), t.Day(), t.Format("15:04"))
}
