package ui

// 面板编辑器：file tab 的内容。对齐 frontend/src/features/panel/Editor.tsx
// 与 workspace store 的 FileSlot 语义，用 mygo ui/editor 承载编辑体验
// （裁剪清单：语法高亮、行号、软换行、查找、撤销；不做多光标 / 折叠 /
// LSP / 正则替换）。
//
// 每个 file tab 拥有独立的数据槽（内容、编辑模式、buffer 草稿、定位行），
// key 是面板 tab 的结构化 id，与 tab 同生共死：关闭 tab 即清槽。真正的
// 写盘与备份走 WorkspaceService 既有链路（WriteFile：备份 + expectedHash
// 外部修改检测），本文件只负责编辑体验与冲突确认。

import (
	"fmt"
	"strings"

	mygo "github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/editor"

	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	cwpanel "github.com/yangshifu1024/craftwork/internal/ui/panel"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// 编辑器模式。
const (
	modeRead = "read"
	modeEdit = "edit"
)

// FileSlot 是一个 file tab 的编辑器数据槽（workspace store 的 FileSlot）。
type FileSlot struct {
	// tabID 是所属面板 tab 的结构化 id。
	tabID string
	// folderID / path 是打开时冻结的目标（保存时显式传回）。
	folderID string
	path     string
	// content 是文件内容与元信息（hash / language / binary 等）。保存
	// 目标以 content 自带的工作区与源文件夹身份为准，不依赖「当前激活
	// 工作区」。
	content *service.FileContent
	// editorMode 是只读或编辑。
	editorMode string
	// buffer 是编辑缓冲（草稿即 buffer 的当前文本，保存成功前不落盘）。
	buffer *editor.Buffer
	// state 是编辑器状态（选区、滚动、查找），跨帧保留。
	state *editor.State
	// targetLine 是跳转定位行（>0 待跳转，1 基）；定位一次后清零。
	targetLine int
	// pendingLine 是内容尚未就绪时的待写入定位行：OpenFileAt 先开 tab
	// 再异步读内容，内容到达时由 applySlotContent 落进 targetLine
	// （避免内容到达把定位行清零的竞态，对齐 React 版 openFileAt 的
	// 「先 openFile 后 set」顺序）。
	pendingLine int
	// loading / errText 是读取状态。
	loading bool
	errText string
}

// loadFileSlot 读取文件内容进指定 tab 的数据槽。
//
// force=true 时无条件重读（文件树点击打开，内容与草稿回到磁盘状态）；
// force=false 时只在槽为空（重开的 tab）时补一次读取（React 渲染层的
// 按需重载）。打开即进入只读模式；正常打开复位定位行，带行号的入口
// （OpenFileAt）在内容就绪后重新设置。
func (p *Pane) loadFileSlot(tabID, folderID, path string, force bool) {
	ws := p.deps.Workspace()
	if ws == nil || p.workspaces() == nil {
		return
	}
	if !force {
		if slot := p.files[tabID]; slot != nil && (slot.loading || slot.content != nil || slot.errText != "") {
			return
		}
	}
	slot, ok := p.files[tabID]
	if !ok {
		slot = &FileSlot{tabID: tabID, folderID: folderID, path: path}
		p.files[tabID] = slot
	}
	slot.loading = true
	slot.errText = ""
	workspaces := p.workspaces()
	workspaceID := ws.ID
	go func() {
		content, err := workspaces.ReadFile(workspaceID, folderID, path)
		p.update(func() {
			slot.loading = false
			if err != nil {
				slot.errText = err.Error()
				return
			}
			p.applySlotContent(slot, content)
		})
	}()
}

// applySlotContent 把读到的文件内容写进数据槽：重建 buffer 与编辑器状态
// （打开与保存回读都走这里，重建比增量同步更稳妥）。
func (p *Pane) applySlotContent(slot *FileSlot, content service.FileContent) {
	slot.content = &content
	slot.editorMode = modeRead
	slot.targetLine = slot.pendingLine
	slot.pendingLine = 0
	if content.Binary {
		// 二进制文件不做文本渲染。
		slot.buffer = nil
		slot.state = nil
		return
	}
	buf := editor.NewBuffer(content.Content)
	buf.SetLanguage(editor.LanguageFor(content.Path))
	slot.buffer = buf
	if slot.state == nil {
		slot.state = &editor.State{Buffer: buf}
	} else {
		// 保留 state（滚动位置等），换掉 buffer。
		slot.state.Buffer = buf
	}
}

// setEditorMode 切换某个 tab 的只读 / 编辑；二进制或内容被截断的文件
// 不允许编辑（会导致写回丢内容）。
func (p *Pane) setEditorMode(tabID, mode string) {
	slot := p.files[tabID]
	if slot == nil || slot.content == nil {
		return
	}
	if mode == modeEdit && (slot.content.Binary || slot.content.Truncated) {
		return
	}
	slot.editorMode = mode
}

// saveSlot 保存某个 tab 的草稿（链路对齐 workspace store 的 saveSlot）：
// 1. 写入目标是打开文件时记录的工作区与源文件夹，不带「当前激活工作区」；
// 2. 带上打开文件时拿到的 hash 写入，后端据此判断文件是否被外部改动；
// 3. 成功后重新读取文件刷新内容与 hash，回到只读模式；
// 4. 后端报「外部修改」时弹确认框，用户选择是否重新加载（丢弃草稿）；
// 5. 其它错误弹提示，草稿保留以便重试。
func (p *Pane) saveSlot(tabID string) {
	slot := p.files[tabID]
	if slot == nil || slot.content == nil || slot.loading || slot.buffer == nil {
		return
	}
	// Buffer 的读方法可从任意 goroutine 调用（rope 快照）。
	text := slot.buffer.String()
	wsID := slot.content.WorkspaceID
	folderID := slot.content.FolderID
	path := slot.content.Path
	hash := slot.content.Hash
	workspaces := p.workspaces()
	go func() {
		err := workspaces.WriteFile(wsID, folderID, path, text, hash)
		if err != nil {
			message := err.Error()
			if strings.Contains(message, "外部修改") {
				// 冲突确认浮层经 update 回主线程发起（overlays 的状态只在
				// 主线程读写）；done 同样在主线程执行，重新加载是读盘 IO，
				// 经 loadFileSlot 自带的后台路径完成。
				p.update(func() {
					paneConfirmReload(path, func(ok bool) {
						if ok {
							p.reloadSlot(tabID, folderID, path)
						}
					})
				})
				return
			}
			p.reportError("保存失败", message)
			return
		}
		refreshed, readErr := workspaces.ReadFile(wsID, folderID, path)
		p.update(func() {
			current := p.files[tabID]
			if current == nil {
				return // tab 已被关闭，丢弃回读
			}
			if readErr != nil {
				p.reportError("保存成功但回读失败", readErr.Error())
				return
			}
			p.applySlotContent(current, refreshed)
			p.savedAt[tabID] = theme.Now()
		})
	}()
}

// reloadSlot 重新读取文件（冲突确认后的「重新加载」；丢弃当前草稿）。
func (p *Pane) reloadSlot(tabID, folderID, path string) {
	if slot := p.files[tabID]; slot != nil {
		slot.content = nil
		slot.errText = ""
	}
	p.loadFileSlot(tabID, folderID, path, true)
}

// editorView 画一个 file tab 的编辑器。
func (p *Pane) editorView(c *mygo.Context, pal *theme.Palette, tab cwpanel.Tab) {
	slot := p.files[tab.ID]
	if slot == nil {
		// 槽被清（重开 / 状态重建）时按需补读一次，渲染层只读状态。
		p.loadFileSlot(tab.ID, tab.FolderID, tab.Path, false)
		slot = p.files[tab.ID]
	}
	if slot == nil || slot.loading || (slot.content == nil && slot.errText == "") {
		mygo.Column(c).Grow(1).Center().Gap(8).Children(func() {
			mygo.Spinner(c)
			mygo.Text(c, "正在打开 "+tab.Title+"…").TextColor(pal.Text2)
		})
		return
	}
	if slot.errText != "" {
		paneEmptyState(c, pal, "alert", "读取文件失败", slot.errText)
		return
	}
	content := slot.content
	if content.Binary {
		p.binaryView(c, pal, tab, slot, content)
		return
	}

	// 定位行：需要跳转时把光标移到该行并居中显示（GoTo 的 revealCenter
	// 语义），行号先夹进有效范围；定位一次即清零。
	if slot.targetLine > 0 && slot.state != nil {
		line := slot.targetLine - 1
		if max := slot.buffer.LineCount() - 1; line > max {
			line = max
		}
		if line < 0 {
			line = 0
		}
		slot.state.GoTo(editor.Pos{Line: line})
		slot.targetLine = 0
	}
	if slot.state != nil {
		// 只读开关按槽的模式每帧同步（State 选项在构建编辑器前生效）。
		slot.state.ReadOnly = slot.editorMode != modeEdit
	}

	mygo.Column(c).Fill().Children(func() {
		p.editorHead(c, pal, tab, slot, content)
		p.editorToolbar(c, pal, tab, slot, content)
		if slot.state != nil {
			editor.View(c, slot.state).Grow(1).FillWidth()
		}
		p.editorNote(c, pal, tab, slot, content)
	})
}

// editorHead 画面包屑 + 状态徽标 + 关闭文件按钮。
func (p *Pane) editorHead(c *mygo.Context, pal *theme.Palette, tab cwpanel.Tab, slot *FileSlot, content *service.FileContent) {
	mygo.Row(c).MinHeight(editorBarH).Padding(6, 10, 6, 10).Gap(8).
		AlignItems(mygo.Center).
		BorderWidth(0, 0, 1, 0).BorderColor(pal.BorderSoft).
		Shrink(0).
		Children(func() {
			// 面包屑显示完整路径，最后一段高亮为当前文件名。
			mygo.Row(c).Grow(1).Gap(3).ClipX().Children(func() {
				crumbs := strings.Split(content.Path, "/")
				for i, crumb := range crumbs {
					if i > 0 {
						mygo.Text(c, "/").FontSize(theme.Fonts.XS()).TextColor(pal.Text4)
					}
					color := pal.Text3
					if i == len(crumbs)-1 {
						color = pal.Text
					}
					mygo.Text(c, crumb).FontSize(theme.Fonts.XS()).TextColor(color)
				}
			})
			// 状态徽标：编辑中 / 只读，以及文件语言。
			badgeBg, badgeText := pal.BgActive, pal.Text2
			if slot.editorMode == modeEdit {
				badgeBg, badgeText = pal.WarnSoft, pal.Warn
			}
			paneBadge(c, pal, paneEditorModeName(slot.editorMode), badgeBg, badgeText)
			paneBadge(c, pal, content.Language, pal.BgActive, pal.Text2)
			if paneIconButton(c, pal, "x", "关闭文件") {
				p.CloseTab(tab.ID)
			}
		})
}

// paneEditorModeName 返回编辑模式的徽标文案。
func paneEditorModeName(mode string) string {
	if mode == modeEdit {
		return "编辑中"
	}
	return "只读"
}

// editorToolbar 画工具条：只读 / 编辑切换、搜索、复制与保存。
func (p *Pane) editorToolbar(c *mygo.Context, pal *theme.Palette, tab cwpanel.Tab, slot *FileSlot, content *service.FileContent) {
	mygo.Row(c).Padding(5, 9, 5, 9).Gap(5).AlignItems(mygo.Center).
		BorderWidth(0, 0, 1, 0).BorderColor(pal.BorderSoft).
		Shrink(0).
		Children(func() {
			if paneToolButton(c, pal, "eye", "只读", slot.editorMode == modeRead) {
				p.setEditorMode(tab.ID, modeRead)
			}
			// 编辑按钮在二进制或超限截断的文件上禁用。
			editable := !content.Binary && !content.Truncated
			if paneToolButtonDisabled(c, pal, "edit", "编辑", slot.editorMode == modeEdit, editable,
				paneEditorEditTip(content)) {
				p.setEditorMode(tab.ID, modeEdit)
			}
			if paneToolButton(c, pal, "search", "搜索", false) && slot.state != nil {
				slot.state.Find("")
			}
			mygo.Spacer(c)
			if paneToolButton(c, pal, "copy", "复制", false) {
				// 复制的是已加载到内存的文本内容（超限文件可能已被截断）。
				// 尽力而为：写剪贴板失败不打断操作流。
				service.ClipboardWriteText(content.Content)
			}
			if slot.editorMode == modeEdit {
				// 只在编辑态显示保存；保存会先备份原文件，并校验外部修改。
				if paneToolButton(c, pal, "save", "保存", true) {
					p.saveSlot(tab.ID)
				}
			}
		})
}

// paneEditorEditTip 返回编辑按钮禁用时的提示。
func paneEditorEditTip(content *service.FileContent) string {
	if content.Binary {
		return "二进制文件不可编辑"
	}
	if content.Truncated {
		return "文件超过 2MB，已截断，编辑已禁用"
	}
	return ""
}

// paneToolButton 画一个工具条按钮（.tool-btn），返回是否被点击。
func paneToolButton(c *mygo.Context, pal *theme.Palette, icon, label string, on bool) bool {
	return paneToolButtonDisabled(c, pal, icon, label, on, true, "")
}

// paneToolButtonDisabled 是带禁用态与提示的工具条按钮。
func paneToolButtonDisabled(c *mygo.Context, pal *theme.Palette, icon, label string, on, enabled bool, tip string) bool {
	el := mygo.ButtonBase(c).Height(toolBtnHeight).Padding(0, 7, 0, 7).Gap(4).
		AlignItems(mygo.Center).Radius(theme.RadiusBadge)
	if tip != "" {
		el = el.Tooltip(tip)
	}
	if !enabled {
		el = el.Disabled(true)
	} else if on {
		el.Background(pal.BgActive)
	} else if el.Hovered() {
		el.Background(pal.BgHover)
	}
	iconColor := pal.Text3
	textColor := pal.Text3
	if on {
		iconColor, textColor = pal.Text, pal.Text
	} else if !enabled {
		iconColor, textColor = pal.Text4, pal.Text4
	}
	el.Children(func() {
		icons.Icon(c, icon).FontSize(11).TextColor(iconColor)
		mygo.Text(c, label).FontSize(theme.Fonts.XS()).TextColor(textColor)
	})
	return enabled && el.Clicked()
}

// binaryView 画二进制文件的信息视图（不尝试文本渲染）。
func (p *Pane) binaryView(c *mygo.Context, pal *theme.Palette, tab cwpanel.Tab, slot *FileSlot, content *service.FileContent) {
	mygo.Column(c).Fill().Children(func() {
		p.editorHead(c, pal, tab, slot, content)
		p.editorToolbar(c, pal, tab, slot, content)
		mygo.Column(c).Grow(1).Center().Gap(8).Padding(16, 16, 16, 16).Children(func() {
			icons.Icon(c, "file").FontSize(24).TextColor(pal.Text3)
			mygo.Text(c, "二进制文件，无法以文本方式预览。").TextColor(pal.Text2)
			mygo.Text(c, "大小："+formatBytes(content.Size)).FontSize(theme.Fonts.SM()).TextColor(pal.Text3)
			mygo.Text(c, "SHA-256："+content.Hash).FontSize(theme.Fonts.SM()).TextColor(pal.Text3)
		})
		p.editorNote(c, pal, tab, slot, content)
	})
}

// editorNote 画底部状态条：左侧是按文件状态切换的说明（含保存反馈），
// 右侧是光标行列、行数、语言与大小（参照 examples/code-editor 的状态条）。
func (p *Pane) editorNote(c *mygo.Context, pal *theme.Palette, tab cwpanel.Tab, slot *FileSlot, content *service.FileContent) {
	note := "只读预览；切换到编辑态后可修改。"
	switch {
	case content.Binary:
		note = "二进制文件不可编辑。"
	case content.Truncated:
		note = "文件超过 2MB，已截断，编辑已禁用；请使用其他方式处理。"
	case slot.editorMode == modeEdit:
		note = "保存前会备份原文件，外部修改会阻止覆盖。"
	}
	if savedAt, ok := p.savedAt[tab.ID]; ok && savedAt > 0 {
		note += " 已保存 " + paneSavedTime(savedAt)
	}
	status := ""
	if slot.state != nil && slot.buffer != nil {
		pos := slot.state.CursorPos()
		status = fmt.Sprintf("Ln %d, Col %d · %d 行 · %s", pos.Line+1, pos.Col+1,
			slot.buffer.LineCount(), content.Language)
	} else {
		status = fmt.Sprintf("%d 行 · %s", content.Lines, content.Language)
	}
	if content.Truncated {
		status += " · 已截断"
	}
	status += " · " + formatBytes(content.Size)
	mygo.Row(c).Padding(6, 10, 6, 10).Gap(6).AlignItems(mygo.Center).
		BorderWidth(1, 0, 0, 0).BorderColor(pal.BorderSoft).
		Shrink(0).
		Children(func() {
			icons.Icon(c, "info").FontSize(12).TextColor(pal.Text4)
			mygo.Text(c, note).FontSize(theme.Fonts.XS()).TextColor(pal.Text3)
			mygo.Spacer(c)
			mygo.Text(c, status).FontSize(theme.Fonts.XS()).TextColor(pal.Text3)
		})
}
