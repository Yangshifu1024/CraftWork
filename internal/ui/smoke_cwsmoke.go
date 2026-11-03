//go:build cwsmoke

package ui

// GUI 冒烟种子（-tags cwsmoke，不进正常构建）：本仓库的 GUI 走查无法
// 依赖键鼠自动化（osascript 无辅助功能授权，点击与按键都送不进窗口），
// 用环境变量把界面状态预置到待截图形态——打开哪个 tab、差异槽里放什么
// 文本、窗口固定位置，并支持延时触发「应用全部」与关窗回收。
//
//	CWSMOKE=terminal          打开终端 tab
//	CWSMOKE=diff              打开差异 tab，内容取 CWSMOKE_DIFF 指定的
//	                          工作区相对路径文件（旧文 = 磁盘内容，
//	                          新文 = 首行改写 + 追加行，含增删改混合）
//	CWSMOKE=editor            打开 CWSMOKE_DIFF 文件的编辑器 tab
//	CWSMOKE_APPLY=1           diff 模式下 8 秒后自动触发 applyDiff；
//	                          确认框替换为自动允许（链路与真实点击一致）
//	CWSMOKE_CLOSE=<秒>        N 秒后自动关窗（验证 OnClosed→终端回收）
//
// 全部模式都把窗口固定到 (60,60) 并置顶，配合 `screencapture -x -R`
// 做区域截图。入口经 shell.go 的 smokeSeed 变量（正常构建为 nil）。

import (
	"os"
	"strings"
	"time"

	"github.com/yangshifu1024/craftwork/internal/ui/overlays"
)

func init() {
	if os.Getenv("CWSMOKE_APPLY") == "1" {
		// 自动允许确认浮层：与真实用户点击「应用修改」走同一条 applyDiff
		// 链路，只是把浮层等待这一步替掉（与单测的替身同一出口）。
		paneConfirmApplyAll = func(_ string, done func(ok bool)) { done(true) }
	}
	smokeSeed = func(s *Shell) {
		if s.win == nil {
			return
		}
		// 固定窗口位置并置顶：截图区域稳定，窗口不被其他应用遮挡。
		s.win.SetPosition(60, 60)
		s.win.SetAlwaysOnTop(true)
		mode := os.Getenv("CWSMOKE")
		target := os.Getenv("CWSMOKE_DIFF")
		switch mode {
		case "terminal":
			s.pane.OpenTerminalTab()
		case "composer-model":
		// 模型选择浮层自动打开（15 号票返工的形态走查；正式数据目录跑
		// 可见真实模型清单）。
		overlays.OpenComposerModels("", nil)
	case "editor":
			if ws := s.activeWorkspace(); ws != nil && target != "" {
				s.pane.OpenFileTab("", target, baseName(target))
			}
		case "diff":
			ws := s.activeWorkspace()
			if ws == nil || target == "" || s.pane.workspaces() == nil {
				return
			}
			content, err := s.pane.workspaces().ReadFile(ws.ID, "", target)
			if err != nil {
				return
			}
			// 新文本：首行改写（删 + 增）、行内改词（词级高亮可见）与
			// 追加行（增），保证差异视图里同时有上下文、删除、新增与
			// 行内词级变更。
			new := content.Content
			new = strings.Replace(new, "package main", "// cwsmoke 首行改写 👍\npackage main", 1)
			new = strings.Replace(new, `return "hello"`, `return "hello world 👍"`, 1)
			new += "// cwsmoke 追加行\n"
			s.pane.OpenDiff(target, content.Content, new)
			if os.Getenv("CWSMOKE_APPLY") == "1" {
				pane := s.pane
				win := s.win
				go func() {
					time.Sleep(8 * time.Second)
					win.Update(func() { pane.applyDiff() })
				}()
			}
		}
		if n := closeAfterSeconds(); n > 0 {
			win := s.win
			go func() {
				time.Sleep(time.Duration(n) * time.Second)
				win.Close()
			}()
		}
	}
}

// closeAfterSeconds 读 CWSMOKE_CLOSE（秒），无或非法时返回 0。
func closeAfterSeconds() int {
	n := 0
	for _, c := range os.Getenv("CWSMOKE_CLOSE") {
		if c < '0' || c > '9' {
			return 0
		}
		n = n*10 + int(c-'0')
	}
	return n
}
