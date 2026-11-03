// markdown.go 是聊天正文的 Markdown 渲染接线：流式用 markdown.Stream、
// 完成态用 markdown.View（同一位置，解析状态跨帧续用）；主题色每帧从
// CraftWork 色板同步（跟随系统深浅切换）；链接统一走 OnLink：
//   - craftwork://path:line（mdref.go 预处理产物）→ OnOpenFile 打开文件；
//   - http/https/mailto → 外链确认（Actions.ConfirmExternal，goroutine），
//     同意后经 mygo.Shell.OpenExternal 交给系统浏览器；
//   - 其余 scheme（javascript: 等）忽略，与 React 版安全策略一致。
//
// 代码块由 markdown 默认渲染（语言标签 + 复制按钮 + 高亮），不需要自建。
// 图片不配置 LoadImage：远程图片保持「描述文本」占位，与 React 版不加载
// 远程图一致。

package chat

import (
	mygo "github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"

	"github.com/yangshifu1024/craftwork/internal/ui"
)

// markdownView 渲染一段正文：运行中的消息用 Stream（词级 reveal、平滑
// 渐入），完成的消息用 View。mdOpts 与 mdTheme 都在 Chat 状态里，跨帧
// 保持解析缓存。
func (s *Chat) markdownView(c *mygo.Context, pal *ui.Palette, text string, streaming bool) {
	opts := s.markdownOptions(c, pal)
	if streaming {
		markdown.Stream(c, text, opts)
		return
	}
	markdown.View(c, text, opts)
}

// markdownOptions 返回共享的渲染选项，并把主题色同步为当前色板。
// Options 含解析缓存与图片表，必须整个应用共用一份（官方文档约定），
// 因此挂在 Chat 状态上；Theme 是值拷贝的目标，每帧覆盖即可跟随换肤。
func (s *Chat) markdownOptions(c *mygo.Context, pal *ui.Palette) *markdown.Options {
	s.mdTheme = markdown.Theme{
		FontSize:   ui.BaseFontSize,
		LineHeight: 1.68, // chat.css .md 的行高
		Muted:      pal.Text3,
		Link:       pal.Accent,
		Strong:     pal.Text,

		Code:           pal.Text,
		CodeBackground: pal.BgActive,
		CodeSize:       0.875,

		BlockSpacing: 0.75,
		ListIndent:   1.6,

		QuoteBar:  pal.Border,
		QuoteText: pal.Text2,
		Rule:      pal.Border,

		CodeBlockBackground: pal.CodeBg,
		CodeBlockBorder:     pal.Border,
		CodeBlockRadius:     ui.RadiusBtn + 2,

		TableBorder: pal.Border,
		TableHeader: pal.BgActive,

		Checkbox: pal.OK,
	}
	s.mdOpts.Theme = &s.mdTheme
	// 流式词级 reveal：默认 ByWord + 渐入，显式声明以固定行为。
	s.mdOpts.Reveal = &markdown.Reveal{Unit: markdown.ByWord}
	return &s.mdOpts
}

// handleLink 是 OnLink 的实现（view 帧内调用）：
//   - 内部引用直接进 OnOpenFile 动作（实现方自行排队到帧后，见 client.go）；
//   - 外链走 ConfirmExternal（goroutine + 原生确认框，不阻塞帧）；
//   - 页内锚点与其他 scheme 忽略。
func (s *Chat) handleLink(url string) {
	if ref, ok := ParseRefLink(url); ok {
		if s.acts != nil && s.acts.OpenFile != nil {
			s.acts.OpenFile(ref.Path, ref.Line)
		}
		return
	}
	switch schemeOf(url) {
	case "http", "https", "mailto":
		if s.acts != nil && s.acts.ConfirmExternal != nil {
			s.acts.ConfirmExternal(url)
		}
	}
}

// schemeOf 返回小写化的 URL scheme；没有 scheme 返回空串。
func schemeOf(url string) string {
	for i := 0; i < len(url); i++ {
		b := url[i]
		switch {
		case b >= 'a' && b <= 'z' || b >= 'A' && b <= 'Z' || b >= '0' && b <= '9' ||
			b == '+' || b == '-' || b == '.':
			continue
		case b == ':':
			return lower(url[:i])
		default:
			return ""
		}
	}
	return ""
}

func lower(s string) string {
	out := []byte(s)
	for i, b := range out {
		if b >= 'A' && b <= 'Z' {
			out[i] = b + 'a' - 'A'
		}
	}
	return string(out)
}
