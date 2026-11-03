// fold.go 是「已工作 N」折叠容器（WorkHistoryGroup.tsx 的移植）：把一轮
// 回复里最终总结之前的过程部件（思考、工具、审批、待办、过程正文）按
// 相邻关系分组；运行中保持平铺，结束后收拢为可展开的折叠头，头部显示
// 本轮运行的耗时（后端持久化事实，中文两段格式化）与终止原因。
//
// 分组归属与 React 版 partRegistry 的 work 字段单源一致：
//   - work：reasoning / tool / todo / approval / history-text（非最终正文
//     的文本部件）；
//   - 非 work：最终正文、error / draft / callout / notice（显性可见，
//     不进折叠区）。

package chat

import (
	"strconv"

	mygo "github.com/egoist/mygo/ui"

	cwui "github.com/yangshifu1024/craftwork/internal/ui"
)

// formatWorkDuration 把毫秒时长格式化为「最多两段非零单位」的中文写法
// （WorkHistoryGroup.tsx 同规则）：秒四舍五入取整、至少 1 秒；天 / 时 /
// 分 / 秒里只保留非零单位、最多两段。空串表示时长缺失，调用方省略时长段。
func formatWorkDuration(durationMs int64) string {
	if durationMs <= 0 {
		return ""
	}
	totalSeconds := (durationMs + 500) / 1000
	if totalSeconds < 1 {
		totalSeconds = 1
	}
	days := totalSeconds / 86400
	hours := totalSeconds % 86400 / 3600
	minutes := totalSeconds % 3600 / 60
	seconds := totalSeconds % 60
	var parts []string
	if days > 0 {
		parts = append(parts, strconv.FormatInt(days, 10)+" 天")
	}
	if hours > 0 {
		parts = append(parts, strconv.FormatInt(hours, 10)+" 小时")
	}
	if minutes > 0 {
		parts = append(parts, strconv.FormatInt(minutes, 10)+" 分")
	}
	if seconds > 0 || len(parts) == 0 {
		parts = append(parts, strconv.FormatInt(seconds, 10)+" 秒")
	}
	if len(parts) > 2 {
		parts = parts[:2]
	}
	out := ""
	for i, p := range parts {
		if i > 0 {
			out += " "
		}
		out += p
	}
	return out
}

// isWorkPart 报告部件是否属于「工作过程」（partRegistry 的 work 列）。
// assistant 最终正文之前的文本部件是 history-text，同样算过程。
func isWorkPart(m *Message, index int) bool {
	part := m.Parts[index]
	switch part.Kind {
	case PartReasoning, PartTool, PartTodo, PartApproval:
		return true
	case PartText:
		// 助手消息里最后一个正文部件是最终总结，其余正文是过程说明。
		return m.Role == RoleAssistant && index != finalTextIndex(m)
	default:
		return false
	}
}

// finalTextIndex 返回助手消息里最终总结（最后一个正文部件）的下标；
// 没有正文时返回 -1。
func finalTextIndex(m *Message) int {
	for i := len(m.Parts) - 1; i >= 0; i-- {
		if m.Parts[i].Kind == PartText {
			return i
		}
	}
	return -1
}

// segment 是消息部件的一段连续渲染区：work 为真时整段是过程部件，
// 结束后可收进折叠区。
type segment struct {
	work  bool
	parts []int
}

// segmentParts 把消息部件按相邻 work 归属切段（GroupedParts 的 groupBy
// 等价物）：相邻 work 部件并为一段，非 work 部件各自成段。
func segmentParts(m *Message) []segment {
	var segs []segment
	for i := range m.Parts {
		work := isWorkPart(m, i)
		if len(segs) > 0 && segs[len(segs)-1].work == work {
			segs[len(segs)-1].parts = append(segs[len(segs)-1].parts, i)
			continue
		}
		segs = append(segs, segment{work: work, parts: []int{i}})
	}
	return segs
}

// groupRunning 报告一个过程组是否仍在进行：消息还在生成，或组内仍有
// 运行中的工具 / 等待决定的审批（GroupedParts 的 running 同语义），
// 此时保持平铺，结束后才折叠。
func groupRunning(m *Message, indexes []int) bool {
	if m.Status == StatusStreaming {
		return true
	}
	for _, i := range indexes {
		p := m.Parts[i]
		if p.Kind == PartTool && strOf(p.Data, "status") == "running" {
			return true
		}
		if p.Kind == PartApproval && strOf(p.Data, "status") == "waiting" {
			return true
		}
	}
	return false
}

// drawSegment 画一个部件段：过程段在运行中平铺、结束后收进折叠区。
func (s *Chat) drawSegment(c *mygo.Context, v *viewSide, m *Message, seg segment) {
	if groupRunning(m, seg.parts) {
		for n, i := range seg.parts {
			if n > 0 {
				partsGap(c)
			}
			s.drawWorkPart(c, v, m, i)
		}
		return
	}
	// 终态收拢：「已工作 N」折叠容器，展开态按消息 + 段起点记在 Chat 上。
	key := m.ID + ":" + strconv.Itoa(seg.parts[0])
	duration := formatWorkDuration(m.WorkDurationMs)
	tag := ""
	switch m.Status {
	case StatusIncomplete:
		tag = "已停止"
	case StatusError:
		tag = "出错"
	}
	open := s.workOpen(key)
	head := mygo.ButtonBase(c).FillWidth().AlignItems(mygo.Center).Gap(6).
		Padding(5, 2)
	clicked := head.Clicked()
	if head.Hovered() {
		head.Background(v.pal.BgHover)
	}
	head.Children(func() {
		label := "已工作"
		if duration != "" {
			label += " " + duration
		}
		mygo.Text(c, label).FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text3)
		if tag != "" {
			mygo.Text(c, "·").FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text4)
			mygo.Text(c, tag).FontSize(cwui.Fonts.SM()).TextColor(v.pal.Warn)
		}
		chev := cwui.Icon(c, "chevron-right").FontSize(12).TextColor(v.pal.Text4)
		if open {
			chev.Rotate(90)
		}
	})
	if clicked {
		s.setWorkOpen(key, !open)
	}
	if open {
		// 折叠体：左缘 2px 描边的竖排内容（.work-history-body）。
		mygo.Column(c).FillWidth().Padding(2, 0, 8, 12).Children(func() {
			body := mygo.Column(c).FillWidth().Gap(8)
			body.BorderWidth(0, 0, 0, 2).BorderColor(v.pal.Border)
			body.Children(func() {
				for n, i := range seg.parts {
					if n > 0 {
						mygo.Box(c).Height(8)
					}
					s.drawWorkPart(c, v, m, i)
				}
			})
		})
	}
}

// drawWorkPart 画组内的一个过程部件：正文（历史说明）走 Markdown，
// 其余画完整卡片。
func (s *Chat) drawWorkPart(c *mygo.Context, v *viewSide, m *Message, i int) {
	part := m.Parts[i]
	if part.Kind == PartText {
		// 过程正文：工具间隙的说明文字（HistoryText → MarkdownBlock）。
		if text := strOf(part.Data, "text"); text != "" {
			s.markdownView(c, v.pal, text, false)
		}
		return
	}
	streaming := m.Status == StatusStreaming && i == len(m.Parts)-1
	s.partCard(c, v, part, streaming)
}

// partsGap 是部件之间的统一间距（msg-body > .blk + .blk 的 10px）。
func partsGap(c *mygo.Context) {
	mygo.Box(c).Height(10)
}
