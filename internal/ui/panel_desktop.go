package ui

// 面板用的 mygo 桌面模块薄封装：剪贴板、文件管理器与废纸篓。错误提示与
// 确认框已并入浮层体系（07 号票）：错误走 overlays.Notify（panelview.go
// reportError），确认走 overlays.Confirm（panel_diff.go / panel_editor.go）。

import (
	"time"

	cwmygo "github.com/egoist/mygo"

	"github.com/yangshifu1024/craftwork/internal/ui/overlays"
)

// panelClock 返回当前毫秒时间戳（保存时间戳等界面反馈用）。
func panelClock() int64 {
	return time.Now().UnixMilli()
}

// paneClipboardWrite 把文本放进系统剪贴板；失败静默（复制是尽力而为的
// 辅助操作，不为此打断操作流）。
func paneClipboardWrite(text string) {
	cwmygo.Clipboard.WriteText(text)
}

// paneShowInFolder 在系统文件管理器里显示文件。
func paneShowInFolder(path string) {
	cwmygo.Shell.ShowItemInFolder(path)
}

// paneTrash 把文件移到废纸篓。
func paneTrash(path string) error {
	return cwmygo.Shell.TrashItem(path)
}

// paneConfirmReload 弹「文件已被外部修改」确认浮层：用户做出选择后 done
// 在主线程执行，ok 表示重新加载（会丢弃当前草稿，安全默认是保留）。
// 生产实现经 overlays.Confirm 发起应用内危险确认框；单测与 GUI 冒烟把它
// 换成直接回决定替身（替身不得在非主线程读写界面状态，见 04 号票坑）。
var paneConfirmReload = func(path string, done func(ok bool)) {
	overlays.Confirm(nil, overlays.ConfirmOpts{
		Title:       "文件已被外部修改",
		Description: "文件已被外部修改，重新加载会丢弃当前未保存的修改。",
		Detail:      path,
		ConfirmText: "重新加载",
		CancelText:  "保留草稿",
		Danger:      true,
	}, done)
}

// paneConfirmApplyAll 弹「应用全部修改」确认浮层：done 的 ok 表示应用。
// 语义对齐 React 版 applyDiff 的 askConfirm（danger 确认、默认取消）。
// 生产实现经 overlays.Confirm（非阻塞，view 帧内可直调——04 号票的
// 原生对话框死锁坑随浮层体系消失）；替身约定同 paneConfirmReload。
var paneConfirmApplyAll = func(path string, done func(ok bool)) {
	overlays.Confirm(nil, overlays.ConfirmOpts{
		Title:       "应用全部修改？",
		Description: "将把差异中的新内容写入文件。写入前会备份原文件。",
		Detail:      path,
		ConfirmText: "应用修改",
		CancelText:  "取消",
		Danger:      true,
	}, done)
}
