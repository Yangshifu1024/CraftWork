package ui

// 面板用的 mygo 桌面模块薄封装：剪贴板、文件管理器、废纸篓、原生对话
// 框与毫秒时钟。对话框出口是可替换的函数变量，便于单测替身与 07 号票
// 浮层体系合入后的替换（替换点收敛在本文件）。

import (
	"time"

	cwmygo "github.com/egoist/mygo"
)

// panelClock 返回当前毫秒时间戳（保存时间戳等界面反馈用）。
func panelClock() int64 {
	return time.Now().UnixMilli()
}

// paneClipboardWrite 把文本放进系统剪贴板；失败静默（复制是尽力而为的
// 辅助操作）。
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

// paneReportError 用原生消息对话框报告错误。
//
// TODO(07): 浮层体系合入后替换为应用内 toast（错误不再打断操作流）；
// 单测把它替换为记录调用，避免起真对话框。
var paneReportError = func(title, detail string) {
	_, _ = cwmygo.Dialog.Message(cwmygo.MessageOptions{
		Type:    cwmygo.MessageWarning,
		Title:   title,
		Message: title,
		Detail:  detail,
	})
}

// paneConfirmReload 弹「文件已被外部修改」确认框：返回是否重新加载
// （重新加载会丢弃当前草稿，安全默认是保留草稿）。
//
// TODO(07): 浮层体系合入后替换为 askConfirm 浮层（danger 样式的确认框）。
var paneConfirmReload = func(path string) bool {
	result, err := cwmygo.Dialog.Message(cwmygo.MessageOptions{
		Type:    cwmygo.MessageQuestion,
		Title:   "文件已被外部修改",
		Message: "文件已被外部修改，重新加载会丢弃当前未保存的修改。",
		Detail:  path,
		Buttons: []string{"重新加载", "保留草稿"},
		// 默认落在安全侧（保留草稿）；「重新加载」标记为破坏性动作。
		DefaultButton:      1,
		DestructiveButtons: []int{0},
	})
	if err != nil {
		return false
	}
	return result.Button == 0
}

// paneConfirmApplyAll 弹「应用全部修改」确认框：返回是否应用。语义对齐
// React 版 applyDiff 的 askConfirm（danger 确认、默认取消）。
//
// 调用方在后台 goroutine 里调（Dialog.Message 阻塞到用户作答为止，
// view 帧内直调会死锁——04 号票已知坑），结果经 Update 回主线程。
//
// TODO(07): 浮层体系合入后替换为 askConfirm 浮层（danger 样式的确认框）。
var paneConfirmApplyAll = func(path string) bool {
	result, err := cwmygo.Dialog.Message(cwmygo.MessageOptions{
		Type:    cwmygo.MessageWarning,
		Title:   "应用全部修改？",
		Message: "将把差异中的新内容写入文件。写入前会备份原文件。",
		Detail:  path,
		Buttons: []string{"取消", "应用修改"},
		// 默认落在安全侧（取消）；「应用修改」标记为破坏性动作。
		DefaultButton:      0,
		CancelButton:       0,
		DestructiveButtons: []int{1},
	})
	if err != nil {
		return false
	}
	return result.Button == 1
}
