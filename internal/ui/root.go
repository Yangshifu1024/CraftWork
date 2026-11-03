// 根视图：三栏外壳 + 浮层体系的组合点，以及窗口级快捷键全表
// （AppShell.tsx 的全局快捷键移植）：
//
//	⌘K  命令面板（开关）
//	⌘N  新建当前模式的会话
//	⌘B  面板开合
//	⌘W  关闭面板当前激活 tab（没有 tab 时不消费）
//	⌘,  设置中心（幂等打开）
//	Esc 退出链（一次退一层，见 Shell.HandleWindowEscape）
//
// 装配（internal/app）用 RootView 生成窗口内容。
package ui

import (
	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/overlays"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// RootView 返回组合外壳与浮层的根 view 函数。
//
// 帧序：先套主题画外壳（面板 pane 在此帧内写入焦点守卫状态），再处理
// 窗口级快捷键，最后画浮层——模态压在外壳上，全局对话框压在模态上，
// 通知在最上层。模态打开时 mygo 会让这里声明的窗口快捷键等待
// （context.insideModal），Esc 归浮层（DialogBase 的 OverlayShortcut）
// 所有，退出链天然只处理无模态时的层级。
func RootView(shell *Shell) func(c *mygo.Context) {
	return func(c *mygo.Context) {
		theme.Apply(c)
		shell.View(c)

		// 窗口级快捷键。⌘K / ⌘, 与浮层体系自身的入口同函数（07 号票）。
		if c.Shortcut(mygo.Cmd, mygo.KeyK) {
			overlays.TogglePalette()
		}
		if c.Shortcut(mygo.Cmd, mygo.KeyComma) {
			overlays.OpenSettings("")
		}
		// ⌘N 新建会话：入口与命令面板、侧栏 + 同一条侧栏编排路径。
		if c.Shortcut(mygo.Cmd, mygo.KeyN) && shell.NewThread != nil {
			shell.NewThread()
		}
		// ⌘B 面板开合：窄窗时 TogglePanel 只改偏好，面板仍按 panelShown
		// 自动收起（与 React 版面板 toggle 同语义）。
		if c.Shortcut(mygo.Cmd, mygo.KeyB) {
			shell.TogglePanel()
		}
		// ⌘W 关闭面板当前激活 tab；没有激活 tab 时不消费
		//（React 版透传系统语义的等价：无菜单接管，按键落空）。
		if c.Shortcut(mygo.Cmd, mygo.KeyW) && shell.PanelHasActiveTab() {
			shell.ClosePanelActiveTab()
		}
		// Esc 退出链。焦点在面板内（终端 / 编辑器 / 过滤框把 Esc 当输入）
		// 时不注册窗口级 Esc：mygo 对已注册快捷键的按键不再投给焦点元素
		// （engine.claimed），不注册才能让 vim 的模式切换这类 Esc 到达终端；
		// 焦点状态是上一帧的，极快速切换焦点时与 React 的逐事件判断同样
		// 存在一帧竞态，属可接受近似。
		if !shell.PanelHasFocus() && c.Shortcut(0, mygo.KeyEscape) {
			shell.HandleWindowEscape()
		}

		overlays.View(c)
	}
}
