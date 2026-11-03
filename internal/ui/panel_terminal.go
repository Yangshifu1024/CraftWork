package ui

// 面板终端 tab：plugins/terminal（libghostty 自管 PTY）替代 TerminalService。
// 对齐 frontend/src/features/panel/TerminalPanel.tsx 的语义（裁剪清单：起
// 默认 shell、真彩、scrollback、选区复制粘贴、Ctrl+L、主题跟随应用、标题
// 跟随、退出提示；不做搜索、鼠标报告应用、缩放、多会话镜像）。
//
// 会话管理：
//   - 惰性创建：终端 tab 首次渲染才 terminal.New——窗口与视图都已存在，
//     插件在首帧按视图尺寸起 PTY（网格尺寸协商由插件每次 paint 完成，
//     01 号票观察到的「卡半尺寸」在 v0.7.4 无法复现，见票 Comments）。
//   - 工作区切换重起：会话目录冻结为创建时的工作区根，切换工作区后旧
//     会话回收、按新根重起（React 版 Create 按当前工作区解析 cwd 的语义）。
//   - 进程回收：窗口关闭 / 应用退出经 Pane.Shutdown 关闭终端（插件对
//     程序挂断 SIGHUP，忽略挂断的程序 2 秒后补杀），不留孤儿 shell。
//
// 立即模式纪律：Terminal 的创建 / 回收在后台 goroutine，结果经
// p.update（win.Update）回主线程；回调里以代次号丢弃迟到的旧结果。

import (
	"fmt"
	"time"

	"github.com/egoist/mygo/plugins/terminal"
	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// termSettingsTTL 是终端设置缓存的存活时间（设置项最迟一个 TTL 生效）。
const termSettingsTTL = time.Second

// cwTerminalDarkTheme / cwTerminalLightTheme 是终端的深浅两套配色：背景与
// 前景取 base.css 的 --code-bg 与文本色，16 色板与选区色对齐 React 版
// TerminalPanel 的 terminalTheme（双套主题语义）。构造时同时给插件
// Theme 与 DarkTheme，插件按窗口外观每帧自选，跟随系统深浅色实时切换。
var (
	cwTerminalDarkTheme = &terminal.Theme{
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
	cwTerminalLightTheme = &terminal.Theme{
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

// terminalFontSizeOrDefault 把设置里的终端字号折算成插件字号（DIP）：
// 设置层已钳制在 10–20，脏数据回退默认 12。
func terminalFontSizeOrDefault(v int) float32 {
	if v < 10 || v > 20 {
		return 12
	}
	return float32(v)
}

// terminalTarget 返回当前上下文下的会话目标：工作区 id 与根目录。
// 没有活动工作区时目录为空（插件回退用户主目录），与「无工作区也能开
// 终端」的临时会话语义一致。
func (p *Pane) terminalTarget() (wsID, dir string) {
	if ws := p.deps.Workspace(); ws != nil {
		return ws.ID, ws.Path
	}
	return "", ""
}

// terminalSettings 取终端相关设置（1 秒 TTL 缓存：设置项最迟 1 秒生效，
// 避免终端 tab 每帧都查一次 SQLite）。
func (p *Pane) terminalSettings() service.Settings {
	if p.deps.Settings == nil {
		return service.DefaultSettings()
	}
	if !p.termSettingsAt.IsZero() && time.Since(p.termSettingsAt) < termSettingsTTL {
		return p.termSettings
	}
	p.termSettings = p.deps.Settings()
	p.termSettingsAt = time.Now()
	return p.termSettings
}

// ensureTerminal 保证终端会话与当前上下文一致：首次渲染创建、工作区
// 切换重起、设置字号变化即时应用。只在主线程（view）调用。
func (p *Pane) ensureTerminal() {
	wsID, dir := p.terminalTarget()
	font := p.terminalSettings().TerminalFontSize
	switch {
	case !p.termRequested:
		p.startTerminal(wsID, dir, font)
	case p.term != nil && p.termWsID != wsID:
		// 工作区切换：旧会话回收，按新根目录重起。
		old := p.term
		p.term = nil
		p.termGen++
		go func() { _ = old.Close() }()
		p.startTerminal(wsID, dir, font)
	}
	if p.term != nil && p.termFont != font {
		p.term.SetFont(terminal.Font{Size: terminalFontSizeOrDefault(font)})
		p.termFont = font
	}
}

// startTerminal 发起一次会话创建：New 在后台 goroutine（加载动态库与
// 构造），结果经 update 回主线程；代次不匹配的迟到结果直接关闭丢弃。
// 回调经 update 切回主线程（终端的回调跑在它自己的 goroutine 上）。
func (p *Pane) startTerminal(wsID, dir string, font int) {
	p.termRequested = true
	p.termWsID, p.termDir = wsID, dir
	p.termFont = font
	p.termErr = nil
	p.termExited, p.termExitCode, p.termTitle = false, 0, ""
	gen := p.termGen
	opts := terminal.Options{
		Dir:  dir,
		Font: terminal.Font{Size: terminalFontSizeOrDefault(font)},
		// 深浅两套都给：插件按窗口外观自选，跟随系统实时切换。
		Theme:     cwTerminalLightTheme,
		DarkTheme: cwTerminalDarkTheme,
		OnTitle: func(title string) {
			p.update(func() {
				if p.termGen == gen {
					p.termTitle = title
				}
			})
		},
		OnExit: func(code int) {
			p.update(func() {
				if p.termGen != gen {
					return
				}
				p.termExited, p.termExitCode = true, code
			})
		},
	}
	go func() {
		term, err := terminal.New(opts)
		p.update(func() {
			if p.termGen != gen {
				// 已被重起 / 关闭超越：直接回收这个迟到的会话。
				if err == nil {
					go func() { _ = term.Close() }()
				}
				return
			}
			if err != nil {
				p.termErr = err
				return
			}
			p.term = term
		})
	}()
}

// restartTerminal 用户点「重起」：回收旧会话并按当前上下文重建（下一帧
// ensureTerminal 会看到 termRequested=false 而重新创建）。
func (p *Pane) restartTerminal() {
	if old := p.term; old != nil {
		p.termGen++
		go func() { _ = old.Close() }()
	}
	p.term = nil
	p.termErr = nil
	p.termExited, p.termExitCode, p.termTitle = false, 0, ""
	p.termRequested = false
}

// Shutdown 回收面板持有的长生命周期资源。窗口关闭 / 应用退出时由外壳
// 调用：终端进程被插件挂断（SIGHUP），不留孤儿 shell。
func (p *Pane) Shutdown() {
	if t := p.term; t != nil {
		p.term = nil
		p.termGen++
		go func() { _ = t.Close() }()
	}
}

// terminalView 画终端 tab：头行（标题跟随 + 会话目录 + 运行状态）、
// 终端视图（加载 / 错误态）、foot 行（审批边界说明 + 退出提示与重起）。
func (p *Pane) terminalView(c *mygo.Context, pal *theme.Palette) {
	p.ensureTerminal()
	_, dir := p.terminalTarget()
	if dir == "" {
		dir = "工作区"
	}

	mygo.Column(c).Fill().Children(func() {
		// 头行：标题（跟随 shell 的 OSC 0/2）+ 目录 + 运行状态 + 提示。
		mygo.Row(c).MinHeight(editorBarH).Padding(6, 10, 6, 10).Gap(8).
			AlignItems(mygo.Center).
			BorderWidth(0, 0, 1, 0).BorderColor(pal.BorderSoft).
			Shrink(0).
			Children(func() {
				icons.Icon(c, "terminal").FontSize(13).TextColor(pal.Text2)
				title := p.termTitle
				if title == "" {
					title = "zsh"
				}
				mygo.Text(c, title).FontSize(theme.Fonts.SM()).TextColor(pal.Text).MaxLines(1).ClipX().Shrink(0)
				mygo.Text(c, dir).FontSize(theme.Fonts.XS()).TextColor(pal.Text3).
					MaxLines(1).Grow(1).ClipX()
				badgeBg, badgeText, badgeLabel := pal.BgActive, pal.Text2, "未运行"
				if p.term != nil && !p.termExited {
					badgeBg, badgeText, badgeLabel = pal.OKSoft, pal.OK, "运行中"
				}
				paneBadge(c, pal, badgeLabel, badgeBg, badgeText)
				mygo.Text(c, "点击终端后直接输入 · Ctrl+L 清屏").FontSize(theme.Fonts.XS()).
					TextColor(pal.Text4).MaxLines(1).Shrink(0)
			})
		// 内容：错误 / 加载 / 终端视图。进程退出后插件会在缓冲区里写
		// 「[Process exited]」，视图保留现场，重起入口在 foot 行。
		// 终端视图按插件文档用 Fill() 撑满剩余区域：网格尺寸由插件按
		// 本元素的绘制矩形每帧协商。
		switch {
		case p.termErr != nil:
			// 启动失败（如动态库缺失）：给出错误与重试入口。
			mygo.Column(c).Grow(1).Center().Gap(8).Padding(16, 16, 16, 16).Children(func() {
				icons.Icon(c, "alert").FontSize(22).TextColor(pal.Text4)
				mygo.Text(c, "终端启动失败").TextColor(pal.Text2)
				mygo.Text(c, p.termErr.Error()).FontSize(theme.Fonts.SM()).TextColor(pal.Text3)
				if paneToolButton(c, pal, "refresh", "重试", false) {
					p.restartTerminal()
				}
			})
		case p.term == nil:
			mygo.Column(c).Grow(1).Center().Gap(8).Children(func() {
				mygo.Spinner(c)
				mygo.Text(c, "正在启动终端…").TextColor(pal.Text2)
			})
		default:
			terminal.View(c, p.term).Fill().AutoFocus()
		}
		// foot 行：用户终端与 Agent 执行命令在审批上的差别；退出时补
		// 「进程已退出」提示与重起按钮（React 版把退出行写进缓冲区，
		// 这里语义一致、入口更显眼）。
		mygo.Row(c).Padding(6, 10, 6, 10).Gap(6).AlignItems(mygo.Center).
			BorderWidth(1, 0, 0, 0).BorderColor(pal.BorderSoft).
			Shrink(0).
			Children(func() {
				mygo.Text(c, "用户终端不经过 Agent 审批；Agent 的 shell_exec 走策略引擎。").
					FontSize(theme.Fonts.XS()).TextColor(pal.Text3)
				mygo.Spacer(c)
				if p.termExited {
					mygo.Text(c, fmt.Sprintf("进程已退出 · 退出码 %d", p.termExitCode)).
						FontSize(theme.Fonts.XS()).TextColor(pal.Warn)
					if paneToolButton(c, pal, "refresh", "重起", false) {
						p.restartTerminal()
					}
				}
			})
	})
}
