// 终端配色：终端插件的深浅两套 ANSI 16 色板与前景 / 背景 / 光标 / 选区，
// 值对齐 React 版 TerminalPanel 的 terminalTheme（前景背景取 base.css 的
// --code-bg 与文本色，双套主题语义），构造时同时给插件 Theme 与
// DarkTheme，插件按窗口外观每帧自选，跟随系统深浅色实时切换。
//
// ANSI 16 色是 VT 仿真约定而非 UI token，但色值同样只在 theme 出口
// 管理（AGENTS.md 6.4：hex 不出主题包）；终端领域属性由 Terminal 前缀
// 与本文件声明，不与界面色板（Palette）混用。
package theme

import (
	mygo "github.com/egoist/mygo/ui"

	"github.com/egoist/mygo/plugins/terminal"
)

// TerminalDarkTheme / TerminalLightTheme 是终端插件的深浅两套配色。
var (
	TerminalDarkTheme = &terminal.Theme{
		Foreground: mygo.Hex("#e8ebf1"),
		Background: mygo.Hex("#0c0f14"),
		Cursor:     mygo.Hex("#5b9dff"),
		Selection:  mygo.RGBA(91, 157, 255, 0.28),
		Palette: [16]mygo.Color{
			mygo.Hex("#0b0d11"), mygo.Hex("#ef7070"), mygo.Hex("#43c08d"), mygo.Hex("#e8b04a"),
			mygo.Hex("#5b9dff"), mygo.Hex("#a78bfa"), mygo.Hex("#4ec9d4"), mygo.Hex("#e8ebf1"),
			mygo.Hex("#4d5563"), mygo.Hex("#f08a8a"), mygo.Hex("#5ad3a2"), mygo.Hex("#f0c46a"),
			mygo.Hex("#7cb2ff"), mygo.Hex("#bfa8ff"), mygo.Hex("#6fdbe4"), mygo.Hex("#ffffff"),
		},
	}
	TerminalLightTheme = &terminal.Theme{
		Foreground: mygo.Hex("#10141b"),
		Background: mygo.Hex("#f7f9fc"),
		Cursor:     mygo.Hex("#2563eb"),
		Selection:  mygo.RGBA(37, 99, 235, 0.22),
		Palette: [16]mygo.Color{
			mygo.Hex("#10141b"), mygo.Hex("#d64545"), mygo.Hex("#0f9d6e"), mygo.Hex("#b7791f"),
			mygo.Hex("#2563eb"), mygo.Hex("#7c5cf0"), mygo.Hex("#0d8f9c"), mygo.Hex("#e0e5ee"),
			mygo.Hex("#7d8798"), mygo.Hex("#c53c3c"), mygo.Hex("#0f9d6e"), mygo.Hex("#b7791f"),
			mygo.Hex("#1d4ed8"), mygo.Hex("#7c5cf0"), mygo.Hex("#0d8f9c"), mygo.Hex("#ffffff"),
		},
	}
)
