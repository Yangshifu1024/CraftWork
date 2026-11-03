// 根视图：三栏外壳 + 浮层体系的组合点，以及窗口级快捷键（⌘K 命令面板、
// ⌘, 设置中心）。装配（internal/app）用 RootView 生成窗口内容。
package ui

import (
	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/overlays"
)

// RootView 返回组合外壳与浮层的根 view 函数。
//
// 帧序：先套主题画外壳，再处理窗口级快捷键，最后画浮层——模态压在外壳
// 上，全局对话框压在模态上，通知在最上层。对话框打开时 mygo 会让这里声
// 明的快捷键等待（对话框外的窗口快捷键不抢键盘），Esc 归对话框所有。
func RootView(shell *Shell) func(c *mygo.Context) {
	return func(c *mygo.Context) {
		Apply(c)
		shell.View(c)

		// 窗口级快捷键：⌘K 命令面板（开关），⌘, 设置中心（幂等打开）。
		if c.Shortcut(mygo.Cmd, mygo.KeyK) {
			overlays.TogglePalette()
		}
		if c.Shortcut(mygo.Cmd, mygo.KeyComma) {
			overlays.OpenSettings("")
		}

		overlays.View(c)
	}
}
