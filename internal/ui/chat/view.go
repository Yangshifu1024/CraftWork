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

	"github.com/yangshifu1024/craftwork/internal/ui"
)

// chatStreamWidth 是消息流内容的最大宽度（interface.md：聊天区 880px）。
const chatStreamWidth = float32(880)

// suggestions 是空会话时按模式展示的快捷提示句（ChatPane.tsx 同文案）。
var suggestions = map[ui.Mode][]string{
	ui.ModeCode: {"解释这个仓库的结构", "运行测试并修复失败用例", "为当前分支写一份变更摘要"},
	ui.ModeWork: {"总结这份文档的要点", "把会议纪要整理成待办表", "草拟一封回复邮件"},
}

// viewSide 是 view 帧内的一次性派生：色板、模式强调色与动作入口。
type viewSide struct {
	pal  *ui.Palette
	acc  ui.AccentSet // 当前模式强调色（气泡底、发送按钮、光标）
	mode ui.Mode
	acts *Actions
}

// View 画整个聊天列：头部、消息流、输入区。由 shell.go 的聊天槽位调用，
// pal 来自外壳每帧套用的主题色板。
func (s *Chat) View(c *mygo.Context, pal *ui.Palette) {
	// 异步动作排队的提示在帧开头弹出（c.Toast 只能在 view 帧内调用）。
	if message := s.TakeToast(); message != "" {
		c.Toast(message)
	}

	v := &viewSide{pal: pal, acts: s.acts}
	mode := ui.ModeCode
	if th := s.Thread(); th.Mode == "work" {
		mode = ui.ModeWork
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
}

// ---------------------------------------------------------------------------
// 会话头部（.chat-head：48px、标题省略、模式与状态徽章）
// ---------------------------------------------------------------------------

// chatHead 画 48px 会话头部。没有会话时画空头部，保证布局高度稳定
// （ChatHeader.tsx 同语义）。背景用 ChatBg 与 BgPanel 的 72% 混合近似
// CSS 的 color-mix；backdrop 模糊省略（下方没有滚动内容穿透的观感差）。
func (s *Chat) chatHead(c *mygo.Context, v *viewSide) {
	head := mygo.Row(c).Height(ui.TopbarHeight).Padding(0, ui.Space6, 0, 18).
		BorderWidth(0, 0, 1, 0).BorderColor(v.pal.Border)
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
	mygo.Box(c).Height(ui.HeightBadge).Padding(0, ui.Space3).Radius(ui.RadiusBadge).
		Background(bg).Center().
		Children(func() {
			mygo.Text(c, label).FontSize(ui.Fonts.XS()).TextColor(fg).FontWeight(600)
		})
}

// modeName 返回模式徽章文案。
func modeName(m ui.Mode) string {
	if m == ui.ModeWork {
		return "Work"
	}
	return "Code"
}

// ---------------------------------------------------------------------------
// 消息流（.chat-scroll + .chat-stream：880px 居中、22px 上内边距）
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
			Padding(ui.Space7, 0, 26, 0) // chat-stream: 22px 顶 26px 底
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
					btn := mygo.ButtonBase(c).Height(ui.HeightBtnSmall).Padding(0, 11).
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
							ui.Icon(c, "arrow-down").FontSize(12).TextColor(v.pal.Text2)
							mygo.Text(c, "回到底部").FontSize(ui.Fonts.SM()).TextColor(v.pal.Text2)
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
	} else if v.mode == ui.ModeWork {
		title = "Work 模式已就绪"
		desc = "读取文档、修改 Word 与 Excel、汇总邮件与日历。所有文档写入都会在审批后执行并自动备份。"
		icon = "briefcase"
	}

	mygo.Box(c).Grow(1).FillWidth().Center().Padding(40, 20).Children(func() {
		mygo.Column(c).Gap(ui.Space5).AlignItems(mygo.Center).MaxWidth(380).Children(func() {
			// 模式图标：46px 圆角方块，模式色软底（.empty-mark）。
			mark := mygo.Box(c).Size(46, 46).Radius(13).
				Background(v.acc.Soft).Center()
			mark.BorderWidth(1).BorderColor(v.acc.Line)
			mark.Children(func() {
				ui.Icon(c, icon).FontSize(22).TextColor(v.acc.Accent)
			})
			mygo.Text(c, title).FontSize(ui.Fonts.LG()).FontWeight(600).TextColor(v.pal.Text)
			mygo.Text(c, desc).FontSize(ui.Fonts.SM()).TextColor(v.pal.Text3).TextAlign(mygo.Center)
			mygo.Row(c).Gap(ui.Space3).Justify(mygo.Center).Children(func() {
				for _, suggestion := range suggestions[v.mode] {
					btn := mygo.ButtonBase(c).Height(30).Padding(0, ui.Space6).Radius(ui.RadiusCtl).
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
						mygo.Text(c, suggestion).FontSize(ui.Fonts.Caption()).TextColor(v.pal.Text2)
					})
				}
			})
		})
	})
}

// ---------------------------------------------------------------------------
// 单条消息（.msg：用户右对齐气泡 / 助手全宽 / 部件顺序渲染）
// ---------------------------------------------------------------------------

// messageRow 画一条消息。用户消息：右对齐、模式色软底气泡（圆角 12、
// 右下 4、最大宽 78%）；助手消息：全宽内容区。元信息行常显时间与标签，
// 悬停浮现复制 / 重新生成。
func (s *Chat) messageRow(c *mygo.Context, v *viewSide, m *Message) {
	isUser := m.Role == RoleUser
	row := mygo.Row(c).FillWidth().AlignItems(mygo.Start)
	if isUser {
		row.Justify(mygo.End)
	}
	// 悬停状态取整条消息（元信息按钮的浮现条件，.msg:hover）。
	hovered := row.Hovered()
	row.Children(func() {
		body := mygo.Column(c)
		if isUser {
			body.MaxWidthPercent(78) // msg.user .msg-body: max-width 78%
		} else {
			// 助手消息全宽：Markdown 在收缩布局里会塌成零宽，显式铺满。
			body.FillWidth()
		}
		body.Children(func() {
			s.messageParts(c, v, m, isUser)
			s.messageFoot(c, v, m, isUser, hovered)
		})
	})
}

// messageParts 按顺序画消息部件：用户消息的文本进气泡，助手消息的正文
// 走 Markdown，非文本部件画最小占位卡（完整卡片归 06 号票）。
func (s *Chat) messageParts(c *mygo.Context, v *viewSide, m *Message, isUser bool) {
	for i := range m.Parts {
		part := m.Parts[i]
		switch part.Kind {
		case PartText:
			text := strOf(part.Data, "text")
			if isUser {
				s.userBubble(c, v, m, text)
			} else if text != "" {
				s.markdownView(c, v.pal, text, m.Status == StatusStreaming)
				// 生成中：正文末尾闪烁光标（.stream-caret）。
				if m.Status == StatusStreaming {
					s.streamCaret(c, v)
				}
			}
		case PartNotice:
			// 系统提示：居中细条（完整样式归 06 号票）。
			noticeStrip(c, v, strOf(part.Data, "text"))
		default:
			// 工具 / 审批 / 思考 / 待办 / 错误的最小占位卡（06 号票替换）。
			partPlaceholder(c, v, part)
		}
		// 部件间距：msg-body > .blk + .blk { margin-top: 10px }。
		if i < len(m.Parts)-1 {
			mygo.Box(c).Height(ui.Space5)
		}
	}
	// 消息没有任何可见部件时（流式刚开始），给出与生成中一致的光标。
	if len(m.Parts) == 0 && m.Status == StatusStreaming {
		s.streamCaret(c, v)
	}
}

// userBubble 画用户消息气泡：模式色软底、1px 模式色描边、圆角 12（右下 4）。
func (s *Chat) userBubble(c *mygo.Context, v *viewSide, m *Message, text string) {
	if text == "" && !hasAttachments(m) {
		return
	}
	bubble := mygo.Column(c).Padding(9, ui.Space6).Radius(12, 12, 4, 12).
		Background(v.acc.Soft)
	bubble.BorderWidth(1).BorderColor(v.acc.Line)
	bubble.Children(func() {
		if text != "" {
			mygo.Text(c, text).FontSize(ui.Fonts.Base())
		}
		if hasAttachments(m) {
			mygo.Box(c).Height(ui.Space2)
			for _, att := range messageAttachments(m) {
				attachChip(c, v, att.Name, formatSize(att.Size))
			}
		}
	})
}

// messageAttachment 是消息内的一条附件显示信息。
type messageAttachment struct {
	Name string
	Size int64
}

// hasAttachments 报告消息的文本部件是否携带附件。
func hasAttachments(m *Message) bool {
	return len(messageAttachments(m)) > 0
}

// messageAttachments 取消息附件的显示名与大小（payload 里可能是路径
// 字符串，也可能是 {name,path,size} 对象，两者都兼容）。
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
				out = append(out, messageAttachment{Name: baseName(a)})
			case map[string]any:
				att := messageAttachment{Name: strOf(a, "name"), Size: int64(numOf(a, "size"))}
				if p := strOf(a, "path"); p != "" {
					att.Name = strOr(att.Name, baseName(p))
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

// formatSize 把字节数显示成 KB / MB；0 表示未知，返回空串。
func formatSize(size int64) string {
	switch {
	case size <= 0:
		return ""
	case size < 1024:
		return fmt.Sprintf("%d B", size)
	case size < 1024*1024:
		return fmt.Sprintf("%.1f KB", float64(size)/1024)
	default:
		return fmt.Sprintf("%.1f MB", float64(size)/(1024*1024))
	}
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
		mygo.Box(c).Padding(3, ui.Space4).Radius(ui.RadiusBadge).
			Background(v.pal.BgHover).
			Children(func() {
				mygo.Text(c, text).FontSize(ui.Fonts.XS()).TextColor(v.pal.Text3)
			})
	})
}

// messageFoot 画元信息行：时间常显、生成中 / 待送达标签、悬停浮现的
// 复制与重新生成（.msg-foot）。
func (s *Chat) messageFoot(c *mygo.Context, v *viewSide, m *Message, isUser bool, hovered bool) {
	foot := mygo.Row(c).Gap(ui.Space3).AlignItems(mygo.Center).Height(ui.Space7)
	if isUser {
		foot.Justify(mygo.End)
	}
	foot.Children(func() {
		mygo.Text(c, formatTime(m.CreatedAt)).FontSize(ui.Fonts.XS()).TextColor(v.pal.Text4)
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
	mygo.Box(c).Height(16).Padding(0, ui.Space2).Radius(3).
		BorderWidth(1).BorderColor(border).Center().
		Children(func() {
			mygo.Text(c, label).FontSize(ui.Fonts.XS()).TextColor(v.pal.Text3)
		})
}

// footButton 画元信息行的一个小按钮，返回是否被点击。
func footButton(c *mygo.Context, v *viewSide, icon, label string) bool {
	btn := mygo.ButtonBase(c).Height(ui.HeightMiniBtn).Padding(0, ui.Space3).Radius(ui.RadiusBadge)
	clicked := btn.Clicked()
	if btn.Hovered() {
		btn.Background(v.pal.BgHover)
	}
	btn.Children(func() {
		mygo.Row(c).Gap(ui.Space2).Center().Children(func() {
			ui.Icon(c, icon).FontSize(12).TextColor(v.pal.Text3)
			mygo.Text(c, label).FontSize(ui.Fonts.XS()).TextColor(v.pal.Text3)
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
