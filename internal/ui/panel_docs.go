package ui

// 面板文档预览：document tab 的内容（Word / Excel / PDF 的文本级预览）。
// 对齐 frontend/src/features/panel/DocumentPanel.tsx 的能力范围：只做
// 文本级能力（Word 仅同格式片段内替换、Excel 不计算公式、PDF 只读文本
// 层），预览数据在面板的单槽 doc 上，切换 document tab 时按需重新请求。

import (
	"time"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	cwpanel "github.com/yangshifu1024/craftwork/internal/ui/panel"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// openDocument 请求文档预览并打开（或聚焦）对应文档 tab。预览成功后在
// 面板里记录单槽，tab 类型取自预览结果的 kind。
func (p *Pane) openDocument(folderID, path string) {
	ws := p.deps.Workspace()
	if ws == nil || p.workspaces() == nil || p.documents() == nil {
		p.reportError("读取文档失败", "当前没有活动工作区")
		return
	}
	if p.docLoading {
		return
	}
	p.docLoading = true
	documents := p.documents()
	workspaceID := ws.ID
	go func() {
		doc, err := documents.Preview(workspaceID, folderID, path)
		p.update(func() {
			p.docLoading = false
			if err != nil {
				p.doc = nil
				p.reportError("读取文档失败", err.Error())
				return
			}
			p.doc = &doc
			p.OpenDocumentTab(folderID, path, baseName(path), doc.Kind)
		})
	}()
}

// docView 画文档预览：数据在单槽上，切换 document tab 时若预览的不是本
// tab 的文件，重新请求预览。
func (p *Pane) docView(c *mygo.Context, pal *theme.Palette, tab cwpanel.Tab) {
	if p.doc == nil || p.doc.Path != tab.Path {
		if !p.docLoading {
			p.openDocument(tab.FolderID, tab.Path)
		}
	}
	if p.docLoading || p.doc == nil || p.doc.Path != tab.Path {
		mygo.Column(c).Grow(1).Center().Gap(8).Children(func() {
			mygo.Spinner(c)
			mygo.Text(c, "正在读取文档…").TextColor(pal.Text2)
		})
		return
	}
	doc := p.doc

	mygo.Column(c).Fill().Children(func() {
		// 头部：文件名、类型徽标、大小与修改时间，以及「重新加载预览」。
		mygo.Row(c).MinHeight(editorBarH).Padding(6, 10, 6, 10).Gap(8).AlignItems(mygo.Center).
			BorderWidth(0, 0, 1, 0).BorderColor(pal.BorderSoft).
			Shrink(0).
			Children(func() {
				icons.Icon(c, "file-text").FontSize(14).TextColor(pal.Text2)
				mygo.Text(c, doc.Name).FontSize(theme.Fonts.SM()).TextColor(pal.Text)
				paneBadge(c, pal, paneUpperKind(doc.Kind), pal.BgActive, pal.Text2)
				mygo.Spacer(c)
				mygo.Text(c, formatBytes(doc.Size)+" · "+paneFormatTime(doc.Modified)).
					FontSize(theme.Fonts.XS()).TextColor(pal.Text3)
				if paneIconButton(c, pal, "refresh", "重新加载预览") {
					p.doc = nil
					p.openDocument(tab.FolderID, tab.Path)
				}
			})
		// 预览能力提示；note 解释无法预览或文本受限的原因。
		mygo.Row(c).Padding(5, 10, 5, 10).Gap(6).AlignItems(mygo.Center).Shrink(0).Children(func() {
			if doc.Supported {
				paneBadge(c, pal, "文本预览", pal.OKSoft, pal.OK)
			} else {
				paneBadge(c, pal, "无法预览", pal.WarnSoft, pal.Warn)
			}
			if doc.Note != "" {
				mygo.Text(c, doc.Note).FontSize(theme.Fonts.XS()).TextColor(pal.Text3)
			}
		})
		if doc.Supported && doc.Text != "" {
			// 提取出的纯文本按原样展示，不做富文本样式还原。
			mygo.Scroll(c).Grow(1).Padding(10, 12, 16, 12).Children(func() {
				mygo.Text(c, doc.Text).FontSize(theme.Fonts.SM()).TextColor(pal.Text2)
			})
			return
		}
		// 无可展示文本时给出图标与原因说明。
		icon := "file-text"
		if doc.Kind == "xlsx" {
			icon = "table"
		}
		note := doc.Note
		if note == "" {
			note = "文档内容为空。"
		}
		paneEmptyState(c, pal, icon, "没有可提取的文本", note)
	})
}

// paneUpperKind 把文档类型徽标转成大写。
func paneUpperKind(kind string) string {
	if kind == "" {
		return "TEXT"
	}
	b := []byte(kind)
	for i := range b {
		if b[i] >= 'a' && b[i] <= 'z' {
			b[i] -= 'a' - 'A'
		}
	}
	return string(b)
}

// paneFormatTime 把毫秒时间戳格式化为短日期时间。
func paneFormatTime(ms int64) string {
	return time.UnixMilli(ms).Format("2006-01-02 15:04")
}
