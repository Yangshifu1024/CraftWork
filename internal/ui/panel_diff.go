package ui

// 面板差异 tab：对齐 frontend/src/features/panel/DiffPanel.tsx 与
// workspace store 的 diff 状态语义，用 mygo ui/editor.DiffView 承载渲染
// （裁剪清单：统一 / 并排、词级高亮、应用全部；不做逐块应用 / 跳过、
// 可编辑新侧）。
//
// 数据槽是全局单份（React 版 workspace store 的 diff 同样是单份）：标题、
// 新旧文本与打开时冻结的工作区身份。聊天审批卡「查看完整差异」经
// OpenDiff 写入数据槽并聚焦差异 tab（06 号票接线）；「应用全部」走
// WorkspaceService.WriteFile 既有链路（备份 + 原子替换 + 审计），不带
// hash 表示允许覆盖（与 React 版 applyDiff 一致）。

import (
	"fmt"

	mygo "github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/editor"

	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// DiffSlot 是差异 singleton tab 的数据槽。
type DiffSlot struct {
	// title 是差异标题，也是应用的写入目标（相对主要目录的路径）。
	title string
	// workspaceID 是打开时冻结的工作区身份：应用写入显式传回，不依赖
	// 「当前激活工作区」。
	workspaceID string
	// oldText / newText 是参与对比的两段文本（newText 是应用写入的内容）。
	oldText, newText string
	// state 是 mygo 的差异视图状态：文本变化时置 nil，渲染时惰性重建
	// （对比、行号、词级高亮全部由内置件自算，不经过后端 ComputeDiff）。
	state *editor.DiffState
	// applying 标记「应用全部」进行中（确认与写入在后台，防重复点击）。
	applying bool
	// appliedAt 是最近一次应用成功的毫秒时间（foot 行展示反馈）。
	appliedAt int64
}

// OpenDiff 打开差异预览：写入数据槽并聚焦差异 singleton tab。标题是
// 相对主要目录的路径（审批差异来自事件载荷，固定按主要目录解析，附加
// 目录文件的差异应用是已知限制，与 React 版一致）。
func (p *Pane) OpenDiff(title, oldText, newText string) {
	p.diff = &DiffSlot{
		title:       title,
		workspaceID: p.scope().WorkspaceID,
		oldText:     oldText,
		newText:     newText,
	}
	p.OpenDiffTab(title)
}

// diffStats 返回差异的增删行数（视图状态惰性重建后读取）。
func (d *DiffSlot) diffStats() (added, deleted int) {
	if d.state == nil {
		return 0, 0
	}
	return d.state.Stats()
}

// ensureDiffState 保证视图状态与文本一致：首次渲染或文本变化时重建
// buffer 与 DiffState。语言按标题扩展名解析（DiffView 的两侧编辑器高亮）。
func (d *DiffSlot) ensureDiffState(sideView bool) {
	if d.state != nil {
		// 并排 / 统一切换只改布局选项，不重建 buffer。
		d.state.Layout = diffLayout(sideView)
		return
	}
	lang := editor.LanguageFor(d.title)
	old := editor.NewBuffer(d.oldText)
	new := editor.NewBuffer(d.newText)
	old.SetLanguage(lang)
	new.SetLanguage(lang)
	d.state = &editor.DiffState{
		Old:    old,
		New:    new,
		Layout: diffLayout(sideView),
		// 词级高亮（相邻单词合并呈现，默认档）与 gutter 条状变更标记。
		Inline:       editor.DiffInlineWordAlt,
		Indicators:   editor.DiffBars,
		ContextLines: 3,
	}
}

// diffLayout 把「并排」开关映射到 mygo 布局枚举；false 是统一视图
// （React 版默认 unified）。
func diffLayout(sideView bool) editor.DiffLayout {
	if sideView {
		return editor.DiffSplit
	}
	return editor.DiffUnified
}

// diffView 画差异 tab：概要行（增删统计 + 标题 + 视图切换）、差异视图、
// foot 行（行数汇总、在编辑器中打开、应用全部）。
func (p *Pane) diffView(c *mygo.Context, pal *theme.Palette) {
	if p.diff == nil {
		// 只有 tab 没有数据（快捷入口打开 / 重开）：空态提示。
		paneEmptyState(c, pal, "diff", "还没有差异预览", "在审批卡片中点击「查看完整差异」，或让 Agent 修改文件。")
		return
	}
	d := p.diff
	side := p.diffViewIdx == 1
	d.ensureDiffState(side)
	added, deleted := d.diffStats()

	mygo.Column(c).Fill().Children(func() {
		// 概要行：+N / −M 徽标、标题、统一 / 并排切换（React 版同款
		// segmented，切换只改布局，保留已算好的差异）。
		mygo.Row(c).MinHeight(editorBarH).Padding(6, 10, 6, 10).Gap(8).
			AlignItems(mygo.Center).
			BorderWidth(0, 0, 1, 0).BorderColor(pal.BorderSoft).
			Shrink(0).
			Children(func() {
				paneBadge(c, pal, "+"+fmt.Sprint(added), pal.OKSoft, pal.OK)
				paneBadge(c, pal, "−"+fmt.Sprint(deleted), pal.DangerSoft, pal.Danger)
				mygo.Text(c, d.title).FontSize(theme.Fonts.SM()).TextColor(pal.Text).Grow(1).ClipX()
				mygo.Segmented(c, &p.diffViewIdx, "统一", "并排").Label("差异视图")
			})
		// 差异视图：内置件自带折叠分隔、词级高亮与滚动。
		editor.DiffView(c, d.state).Grow(1).FillWidth()
		// foot 行：行数汇总 + 应用反馈，右侧打开文件与应用全部。
		mygo.Row(c).Padding(6, 10, 6, 10).Gap(6).AlignItems(mygo.Center).
			BorderWidth(1, 0, 0, 0).BorderColor(pal.BorderSoft).
			Shrink(0).
			Children(func() {
				summary := fmt.Sprintf("%d 行新增 · %d 行删除", added, deleted)
				if d.appliedAt > 0 {
					summary += " · 已应用 " + paneSavedTime(d.appliedAt)
				}
				mygo.Text(c, summary).FontSize(theme.Fonts.XS()).TextColor(pal.Text3)
				mygo.Spacer(c)
				if paneToolButton(c, pal, "file-code", "在编辑器中打开", false) {
					p.OpenFileAt(d.title, 0)
				}
				if d.applying {
					paneToolButtonDisabled(c, pal, "check", "应用中…", true, false, "")
				} else if paneToolButton(c, pal, "check", "应用全部", true) {
					p.applyDiff()
				}
			})
	})
}

// applyDiff 应用全部修改：确认浮层在当前帧发起（overlays.Confirm 非阻塞，
// 04 号票「原生对话框不能进 view 帧」的坑不再适用），用户确认后走既有
// 写文件链路：WriteFile 备份原文件 + 原子替换 + 落审计；不带 hash 表示
// 允许覆盖。成功后在编辑器里打开磁盘最新状态（对齐 React 版 applyDiff
// 的收尾）。done 回调在主线程执行：取消只复位 applying；确认后的写盘
// 是磁盘 IO，放后台 goroutine，结果经 update 回主线程。
func (p *Pane) applyDiff() {
	d := p.diff
	if d == nil || d.applying || d.workspaceID == "" || p.workspaces() == nil {
		return
	}
	d.applying = true
	title, wsID, text := d.title, d.workspaceID, d.newText
	paneConfirmApplyAll(title, func(ok bool) {
		if !ok {
			if p.diff == d {
				d.applying = false
			}
			return
		}
		go func() {
			err := p.workspaces().WriteFile(wsID, "", title, text, "")
			p.update(func() {
				if p.diff == d {
					d.applying = false
					if err == nil {
						d.appliedAt = theme.Now()
						// 打开文件显示磁盘最新状态（对齐 React 版 applyDiff）。
						p.openResolved("", title, 0)
					}
				}
				if err != nil {
					p.reportError("应用失败", err.Error())
				}
			})
		}()
	})
}
