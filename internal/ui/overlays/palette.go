// 命令面板（⌘K）：命令、会话与文件三类入口，按关键字过滤，↑↓ 循环选择，
// 回车执行，Esc / 遮罩关闭。12 条命令对齐 React 版；会话与文件只在有
// 关键字时出现，各取前 8 条。
//
// 会话列表在打开时读取；文件索引在打开时后台遍历活动工作区（有界：深度
// 与条数封顶，跳过依赖目录），完成后经 win.Update 触发下一帧。

package overlays

import (
	"path/filepath"
	"strings"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// paletteState 是命令面板的界面状态。
type paletteState struct {
	// open 是面板是否打开。
	open bool
	// query 是搜索关键字。
	query string
	// index 是当前选中的结果下标（↑↓ 循环）。
	index int
	// lastQuery 用于检测关键字变化（变化时选中项回到第一行）。
	lastQuery string

	// threads 是打开时缓存的会话列表（按当前默认模式）。
	threads []service.ThreadView
	// files 是打开时后台遍历的文件索引；filesLoading 期间为空。
	files        []paletteFile
	filesLoading bool
}

// paletteFile 是文件索引的一条记录。
type paletteFile struct {
	workspaceID string
	folderID    string
	path        string
	name        string
}

// paletteItem 是面板里的一行结果。
type paletteItem struct {
	id       string
	group    string // 命令 / 会话 / 文件
	title    string
	sub      string
	icon     string
	disabled bool
	run      func()
}

// togglePalette 打开 / 关闭命令面板；打开时重置查询并预载会话与文件索引。
func (s *Store) togglePalette() {
	st := &s.palette
	st.open = !st.open
	if st.open {
		st.query = ""
		st.index = 0
		st.lastQuery = ""
		s.loadPaletteThreads()
		s.startFileIndex()
	}
}

// loadPaletteThreads 按当前默认模式读取会话列表。
func (s *Store) loadPaletteThreads() {
	if s.be == nil || s.be.Threads == nil {
		return
	}
	mode := s.settingsSnapshot().Mode
	if threads, err := s.be.Threads.List(mode, false); err == nil {
		s.palette.threads = threads
	}
}

// junkDirs 是文件索引跳过的目录名（依赖与构建产物）。
var junkDirs = map[string]bool{
	"node_modules": true, ".git": true, "dist": true, "build": true,
	"target": true, ".next": true, "vendor": true, "__pycache__": true,
	"bin": true, "obj": true, ".venv": true,
}

// paletteFileCap 是文件索引的条数与深度上限（有界遍历，避免卡主线程外
// 的 goroutine 太久）。
const (
	paletteFileCap       = 400
	paletteFileMaxDepth  = 6
	paletteFilePerFolder = 200
)

// startFileIndex 在后台遍历活动工作区的全部源文件夹，构建文件索引。
func (s *Store) startFileIndex() {
	st := &s.palette
	st.files = nil
	st.filesLoading = true
	if s.be == nil || s.be.Workspace == nil {
		st.filesLoading = false
		return
	}
	s.runAsync(func() func() {
		files := []paletteFile{}
		active, err := s.be.Workspace.Active()
		if err == nil && active != nil {
			for _, folder := range active.Folders {
				s.walkPaletteFolder(active.ID, folder.ID, "", 0, &files)
				if len(files) >= paletteFileCap {
					break
				}
			}
		}
		return func() {
			st := &s.palette
			st.files = files
			st.filesLoading = false
		}
	})
}

// walkPaletteFolder 递归遍历一个源文件夹，深度与条数有界。
func (s *Store) walkPaletteFolder(workspaceID, folderID, rel string, depth int, out *[]paletteFile) {
	if depth > paletteFileMaxDepth || len(*out) >= paletteFileCap {
		return
	}
	nodes, err := s.be.Workspace.ListDir(workspaceID, folderID, rel)
	if err != nil {
		return
	}
	count := 0
	for _, node := range nodes {
		if count >= paletteFilePerFolder || len(*out) >= paletteFileCap {
			return
		}
		switch node.Type {
		case "file":
			*out = append(*out, paletteFile{
				workspaceID: workspaceID,
				folderID:    folderID,
				path:        node.Path,
				name:        node.Name,
			})
			count++
		case "dir":
			base := filepath.Base(node.Path)
			if junkDirs[strings.ToLower(base)] || strings.HasPrefix(base, ".") {
				continue
			}
			s.walkPaletteFolder(workspaceID, folderID, node.Path, depth+1, out)
		}
	}
}

// drawPalette 画命令面板。
func (s *Store) drawPalette(c *mygo.Context) {
	st := &s.palette
	if !st.open {
		return
	}
	p := pal(c)

	// 组装结果：命令始终参与；会话与文件只在有关键字时按标题 / 路径过滤。
	items := s.paletteItems(c, st)
	// 关键字变化后选中项回到第一行。
	if st.lastQuery != st.query {
		st.lastQuery = st.query
		st.index = 0
	}
	if st.index >= len(items) {
		st.index = len(items) - 1
	}
	if st.index < 0 {
		st.index = 0
	}

	panel := mygo.DialogBase(c, &st.open, func(backdrop, panel mygo.Element) {
		styleBackdrop(backdrop)
		stylePanel(c, panel)
		placePanel(c, backdrop, panel, 640, 0.09)
		panel.Radius(13).Gap(0).Clip().Children(func() {
			// 输入行。
			mygo.Row(c).Height(48).Padding(0, theme.Space7).AlignItems(mygo.Center).Gap(theme.Space5).
				BorderWidth(0, 0, 1, 0).BorderColor(p.Border).Children(func() {
				icons.Icon(c, "command").Size(16, 16).TextColor(p.Text3)
				input := mygo.TextInput(c, &st.query).Grow(1).
					Background(p.BgFloat).Label("面板搜索").
					Placeholder("搜索命令、会话或文件…").AutoFocus()
				// ↑↓ 循环选择（单行输入不消费上下键）；回车执行走 Submitted。
				if c.Shortcut(0, mygo.KeyDown) && len(items) > 0 {
					st.index = (st.index + 1) % len(items)
				}
				if c.Shortcut(0, mygo.KeyUp) && len(items) > 0 {
					st.index = (st.index - 1 + len(items)) % len(items)
				}
				if input.Submitted() && len(items) > 0 && !items[st.index].disabled {
					st.open = false
					if run := items[st.index].run; run != nil {
						run()
					}
				}
				kbd(c, "Esc")
			})

			// 结果列表。
			list := mygo.Scroll(c).Grow(1).MaxHeight(420).Padding(theme.Space2, 0)
			list.Children(func() {
				if len(items) == 0 {
					mygo.Box(c).Fill().Center().Height(120).Children(func() {
						mutedText(c, "没有匹配结果")
					})
					return
				}
				lastGroup := ""
				for i, item := range items {
					i, item := i, item
					if item.group != lastGroup {
						lastGroup = item.group
						mygo.Text(c, item.group).FontSize(theme.Fonts.XS()).TextColor(p.Text4).
							Padding(theme.Space5, theme.Space6, theme.Space2, theme.Space6)
					}
					row := mygo.ButtonBase(c)
					row.Fill().Height(44).Padding(0, theme.Space6).
						AlignItems(mygo.Center).Gap(theme.Space5)
					if i == st.index {
						row.Background(p.BgActive)
					}
					row.Children(func() {
						mygo.Box(c).Width(28).Height(28).Radius(theme.RadiusCtl).Center().Background(p.BgHover).Children(func() {
							icons.Icon(c, item.icon).Size(15, 15).TextColor(p.Text2)
						})
						mygo.Column(c).Grow(1).Gap(1).Children(func() {
							mygo.Text(c, item.title).SingleLine()
							if item.sub != "" && item.group != "命令" {
								mygo.Text(c, item.sub).FontSize(theme.Fonts.SM()).TextColor(p.Text4).SingleLine()
							}
						})
						if item.sub != "" && item.group == "命令" {
							kbd(c, item.sub)
						}
					})
					if row.Clicked() && !item.disabled {
						st.open = false
						if item.run != nil {
							item.run()
						}
					}
				}
				if st.filesLoading {
					mutedText(c, "正在索引文件…").Padding(theme.Space4, theme.Space6)
				}
			})

			// 底部提示。
			mygo.Row(c).Height(34).Padding(0, theme.Space7).AlignItems(mygo.Center).Gap(theme.Space6).
				BorderWidth(1, 0, 0, 0).BorderColor(p.Border).Children(func() {
				mutedText(c, "↑↓ 选择 · ↵ 执行 · Esc 关闭").Grow(1)
				mutedText(c, "命令 / 会话 / 文件")
			})
		})
	})
	if panel.Dismissed() {
		st.open = false
	}
}

// paletteItems 组装面板结果列表。
func (s *Store) paletteItems(c *mygo.Context, st *paletteState) []paletteItem {
	items := make([]paletteItem, 0, 16)
	// 12 条命令对齐 React 版；未接线的钩子置灰并在子文案标注接入票。
	themeLabel, themeTarget := "切换到深色主题", "dark"
	if theme.IsDark() {
		themeLabel, themeTarget = "切换到浅色主题", "light"
	}
	panelOpen := s.hooks.PanelOpen == nil || s.hooks.PanelOpen()
	togglePanel, togglePanelIcon := "展开右侧面板", "panel-open"
	if panelOpen {
		togglePanel, togglePanelIcon = "收起右侧面板", "panel-close"
	}
	commands := []paletteItem{
		{id: "new-thread", group: "命令", title: "新建会话", sub: "⌘N", icon: "plus", disabled: s.hooks.NewThread == nil, run: s.hooks.NewThread},
		{id: "new-temp-thread", group: "命令", title: "新建临时会话", sub: "无需工作区", icon: "zap", disabled: s.hooks.NewTempThread == nil, run: s.hooks.NewTempThread},
		{id: "settings", group: "命令", title: "打开设置", sub: "⌘,", icon: "settings", run: func() { OpenSettings("") }},
		{id: "toggle-theme", group: "命令", title: themeLabel, icon: "sun", run: func() {
			s.save(c, func(x *service.Settings) { x.Theme = themeTarget })
		}},
		{id: "toggle-panel", group: "命令", title: togglePanel, icon: togglePanelIcon, disabled: s.hooks.TogglePanel == nil, run: s.hooks.TogglePanel},
		{id: "maximize-panel", group: "命令", title: "全屏右侧面板", icon: "maximize", disabled: s.hooks.MaximizePanel == nil, run: s.hooks.MaximizePanel},
		{id: "close-panel-tab", group: "命令", title: "关闭面板当前标签页", sub: "⌘W", icon: "x", disabled: s.hooks.ClosePanelTab == nil, run: s.hooks.ClosePanelTab},
		{id: "usage", group: "命令", title: "打开用量统计", icon: "zap", run: func() { OpenData(TabUsage) }},
		{id: "audit", group: "命令", title: "打开审计记录", icon: "shield-check", run: func() { OpenData(TabAudit) }},
		{id: "backup", group: "命令", title: "打开备份列表", icon: "archive", run: func() { OpenData(TabBackup) }},
		{id: "mode-code", group: "命令", title: "切换到 Code 模式", icon: "code", disabled: s.hooks.SetMode == nil, run: func() { s.hooks.SetMode("code") }},
		{id: "mode-work", group: "命令", title: "切换到 Work 模式", icon: "briefcase", disabled: s.hooks.SetMode == nil, run: func() { s.hooks.SetMode("work") }},
	}
	for i := range commands {
		cmd := &commands[i]
		if cmd.disabled {
			cmd.sub = strings.TrimSpace(cmd.sub + " 待接线")
		}
	}

	keyword := strings.ToLower(strings.TrimSpace(st.query))
	if keyword == "" {
		return commands
	}
	match := func(title, sub string) bool {
		return strings.Contains(strings.ToLower(title), keyword) ||
			strings.Contains(strings.ToLower(sub), keyword)
	}
	for _, cmd := range commands {
		if match(cmd.title, cmd.sub) {
			items = append(items, cmd)
		}
	}
	// 会话：标题匹配，最多 8 条。
	shown := 0
	for _, t := range st.threads {
		if shown >= 8 {
			break
		}
		if !match(t.Title, t.ModelName) {
			continue
		}
		items = append(items, paletteItem{
			id: "thread-" + t.ID, group: "会话", title: t.Title,
			sub:      t.ModelName,
			icon:     "message",
			disabled: s.hooks.OpenThread == nil,
			run:      func() { s.hooks.OpenThread(t.ID) },
		})
		shown++
	}
	// 文件：路径匹配，最多 8 条。
	shown = 0
	for _, f := range st.files {
		if shown >= 8 {
			break
		}
		if !match(f.name, f.path) {
			continue
		}
		items = append(items, paletteItem{
			id: "file-" + f.path, group: "文件", title: f.name,
			sub:      f.path,
			icon:     "file-code",
			disabled: s.hooks.OpenFile == nil,
			run:      func() { s.hooks.OpenFile(f.workspaceID, f.path) },
		})
		shown++
	}
	return items
}
