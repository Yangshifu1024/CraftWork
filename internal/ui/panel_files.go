package ui

// 面板文件树：懒加载、过滤平铺、多源文件夹分段、扩展名着色图标、Git
// 状态字母与行右键菜单。对齐 frontend/src/features/panel/FilesPanel.tsx。
//
// 多文件夹：工作区可以挂载多个源文件夹，文件树按文件夹分段渲染，每段
// 顶层是源文件夹根（带「主要」徽标），段内结构与单根时代完全一致。
// 内部缓存的键是「文件夹 ID + 相对路径」的组合键，避免不同根之间的同名
// 相对路径互相覆盖。懒加载：只有展开目录时才请求子节点，List 的按需
// 渲染让大仓库只构建可视行。

import (
	"sort"
	"strconv"
	"strings"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/service"
	cwpanel "github.com/yangshifu1024/craftwork/internal/ui/panel"
)

// flatHit 是过滤平铺的一条命中：文件名匹配关键字，保留所属文件夹。
type flatHit struct {
	folderID string
	node     service.FileNode
}

// flatResults 是过滤模式下的扁平文件列表：遍历所有已加载目录，按文件名
// 包含关键字筛选并去重；返回 nil 表示没有过滤关键字，界面应显示树形
// 结构。上限 flatLimit（200）条。
func flatResults(tree map[string][]service.FileNode, filter string) []flatHit {
	if strings.TrimSpace(filter) == "" {
		return nil
	}
	keyword := strings.ToLower(strings.TrimSpace(filter))
	seen := map[string]bool{}
	var result []flatHit
	// map 遍历顺序不稳定，先排序组合键让结果稳定。
	keys := make([]string, 0, len(tree))
	for key := range tree {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	for _, key := range keys {
		folderID, _ := splitFolderKey(key)
		for _, node := range tree[key] {
			combo := folderKey(folderID, node.Path)
			if node.Type == "file" &&
				strings.Contains(strings.ToLower(node.Name), keyword) &&
				!seen[combo] {
				seen[combo] = true
				result = append(result, flatHit{folderID: folderID, node: node})
			}
		}
	}
	if len(result) > flatLimit {
		result = result[:flatLimit]
	}
	return result
}

// folderKey 是源文件夹的组合键：树缓存与展开状态都以它为键。
// folderID 为空表示主要目录（兼容尚未挂载文件夹列表的工作区）。
func folderKey(folderID, path string) string {
	return folderID + "|" + path
}

// splitFolderKey 从组合键拆出文件夹 ID 与相对路径。组合键中的路径理论上
// 可以包含「|」；这里以第一个「|」为分隔，因为 folderID（wfo_xxx）不含
// 该字符。
func splitFolderKey(key string) (string, string) {
	index := strings.Index(key, "|")
	if index < 0 {
		return "", key
	}
	return key[:index], key[index+1:]
}

// fileIconName 根据文件名后缀选择文件图标；未识别的类型统一使用通用
// 文件图标（FilesPanel.tsx fileIcon）。
func fileIconName(name string) string {
	lower := strings.ToLower(name)
	switch {
	case strings.HasSuffix(lower, ".md"),
		strings.HasSuffix(lower, ".pdf"), strings.HasSuffix(lower, ".docx"):
		return "file-text"
	case strings.HasSuffix(lower, ".go"), strings.HasSuffix(lower, ".ts"),
		strings.HasSuffix(lower, ".tsx"), strings.HasSuffix(lower, ".js"),
		strings.HasSuffix(lower, ".py"), strings.HasSuffix(lower, ".json"),
		strings.HasSuffix(lower, ".yaml"), strings.HasSuffix(lower, ".yml"),
		strings.HasSuffix(lower, ".toml"):
		return "file-code"
	case strings.HasSuffix(lower, ".xlsx"), strings.HasSuffix(lower, ".csv"):
		return "table"
	case strings.HasSuffix(lower, ".png"), strings.HasSuffix(lower, ".jpg"),
		strings.HasSuffix(lower, ".jpeg"), strings.HasSuffix(lower, ".webp"),
		strings.HasSuffix(lower, ".svg"):
		return "image"
	}
	return "file"
}

// fileIconColor 返回文件后缀对应的图标着色（panel.css 的 .f-ico.go/.doc/
// .sheet/.pdf；md 与未识别类型用默认 text-3）。色值收在 theme 色板的
// File*Icon 字段里，深浅模式差异也由色板承担。dir 为 true 时返回目录
// 图标色。
func fileIconColor(pal *Palette, name string, dir bool) mygo.Color {
	if dir {
		return pal.FileDirIcon
	}
	lower := strings.ToLower(name)
	switch {
	case strings.HasSuffix(lower, ".go"):
		return pal.FileCodeIcon
	case strings.HasSuffix(lower, ".docx"):
		return pal.FileDocIcon
	case strings.HasSuffix(lower, ".xlsx"):
		return pal.FileSheetIcon
	case strings.HasSuffix(lower, ".pdf"):
		return pal.FilePdfIcon
	}
	return pal.Text3
}

// gitLetterColor 返回 Git 状态字母的颜色（panel.css .tree-git.M/A/D/U；
// 其余代码继承文字色）。
func gitLetterColor(pal *Palette, code string) mygo.Color {
	if code == "" {
		return pal.Text3
	}
	switch code[0] {
	case 'M':
		return pal.Warn
	case 'A':
		return pal.OK
	case 'D':
		return pal.Danger
	case 'U':
		return pal.Text4
	}
	return pal.Text3
}

// workspaceFolders 返回工作区的全部源文件夹；旧数据没有 folders 时退化
// 为单个主要目录段（React FilesPanel 同款兜底）。
func workspaceFolders(ws *service.WorkspaceView) []service.WorkspaceFolderView {
	if len(ws.Folders) > 0 {
		return ws.Folders
	}
	return []service.WorkspaceFolderView{{ID: "", Path: ws.Path, Primary: true}}
}

// ensureTree 在文件树可见且根尚未加载（或工作区已切换）时加载各源文件
// 夹的根目录与主要目录的 Git 状态；失败静默，下次进入重试。
func (p *Pane) ensureTree() {
	ws := p.deps.Workspace()
	if ws == nil || p.workspaces() == nil {
		return
	}
	if p.treeFor == ws.ID || p.pending["root"] {
		return
	}
	p.pending["root"] = true
	workspaces := p.workspaces()
	workspaceID := ws.ID
	folders := workspaceFolders(ws)
	go func() {
		type rootEntry struct {
			key   string
			nodes []service.FileNode
		}
		entries := make([]rootEntry, 0, len(folders))
		var lastErr error
		for _, folder := range folders {
			nodes, err := workspaces.ListDir(workspaceID, folder.ID, ".")
			if err != nil {
				lastErr = err
				continue
			}
			entries = append(entries, rootEntry{key: folderKey(folder.ID, "."), nodes: nodes})
		}
		git, _ := workspaces.GitStatuses(workspaceID)
		p.update(func() {
			p.pending["root"] = false
			if lastErr != nil && len(entries) == 0 {
				// 全部根读取失败：保持未加载态，用户点刷新或重进面板重试。
				return
			}
			for _, entry := range entries {
				p.tree[entry.key] = entry.nodes
			}
			if git != nil {
				p.git = git
			}
			p.treeFor = workspaceID
		})
	}()
}

// loadDir 按需加载某个源文件夹内目录的子节点（防重入）。
func (p *Pane) loadDir(folderID, path string) {
	ws := p.deps.Workspace()
	if ws == nil || p.workspaces() == nil {
		return
	}
	key := folderKey(folderID, path)
	if p.pending[key] {
		return
	}
	p.pending[key] = true
	workspaces := p.workspaces()
	workspaceID := ws.ID
	go func() {
		nodes, err := workspaces.ListDir(workspaceID, folderID, path)
		p.update(func() {
			p.pending[key] = false
			if err != nil {
				return
			}
			p.tree[key] = nodes
		})
	}()
}

// toggleDir 展开或收起目录；首次展开时才请求子节点，再次展开不会重复
// 请求。
func (p *Pane) toggleDir(folderID, path string) {
	key := folderKey(folderID, path)
	p.expanded[key] = !p.expanded[key]
	if p.expanded[key] && p.tree[key] == nil {
		p.loadDir(folderID, path)
	}
}

// reloadTree 清掉全部树缓存并重新加载根与 Git 状态。
func (p *Pane) reloadTree() {
	p.tree = map[string][]service.FileNode{}
	p.expanded = map[string]bool{}
	p.treeFor = ""
	p.ensureTree()
}

// selectedFilePath 面板里激活 file tab 的路径：文件树的选中高亮跟随它。
// files tab 激活时取最近激活过的 file tab，用户回到树里能看到刚才编辑
// 的文件。
func selectedFilePath(visible []cwpanel.Tab, activeID string) string {
	var latest *cwpanel.Tab
	for i := range visible {
		tab := visible[i]
		if tab.Type != cwpanel.TabFile {
			continue
		}
		if tab.ID == activeID {
			return tab.Path
		}
		if latest == nil || tab.ActivatedAt > latest.ActivatedAt {
			latest = &visible[i]
		}
	}
	if latest != nil {
		return latest.Path
	}
	return ""
}

// filesView 画文件树：过滤框 + 元信息行 + 树形结构（或过滤平铺）。
func (p *Pane) filesView(c *mygo.Context, pal *Palette) {
	p.ensureTree()
	ws := p.deps.Workspace()
	if ws == nil {
		paneEmptyState(c, pal, "folder", "没有活动工作区", "在侧栏添加或选择一个工作区后浏览文件。")
		return
	}
	folders := workspaceFolders(ws)
	flat := flatResults(p.tree, p.filter)

	mygo.Column(c).Fill().Children(func() {
		// 工具栏：过滤框 + 刷新按钮。
		mygo.Row(c).Height(9*2+HeightInput).Padding(0, 9, 0, 9).
			Gap(6).AlignItems(mygo.Center).Shrink(0).Children(func() {
			mygo.Row(c).Grow(1).Height(HeightInput).Padding(0, 8, 0, 8).Gap(6).
				AlignItems(mygo.Center).
				Background(pal.BgInput).
				Radius(RadiusCtl).
				Border(1, pal.Border).
				Children(func() {
					Icon(c, "search").FontSize(13).TextColor(pal.Text3)
					mygo.TextInput(c, &p.filter).Grow(1).Placeholder("过滤文件…")
				})
			if paneIconButton(c, pal, "refresh", "刷新文件树") {
				p.reloadTree()
			}
		})
		// 元信息行：左侧显示当前是过滤结果还是完整工作区，右侧显示 Git
		// 变更文件数。
		mygo.Row(c).Padding(2, 9, 6, 9).Gap(6).Shrink(0).Children(func() {
			label := "工作区文件"
			if flat != nil {
				label = "匹配 " + strconv.Itoa(len(flat)) + " 个文件"
			}
			mygo.Text(c, label).FontSize(Fonts.XS()).TextColor(pal.Text3)
			mygo.Spacer(c)
			mygo.Text(c, strconv.Itoa(len(p.git))+" 个变更").FontSize(Fonts.XS()).TextColor(pal.Text3)
		})

		list := mygo.Scroll(c).Grow(1).Padding(0, 6, 6, 6)
		list.Children(func() {
			if flat != nil {
				// 过滤结果：平铺展示文件名与完整路径，点击直接打开。
				for _, hit := range flat {
					p.flatItem(c, pal, hit)
				}
				return
			}
			// 无过滤关键字：按源文件夹分段，段内从各自根目录开始递归渲染树。
			selected := selectedFilePath(p.currentVisible(), p.currentActiveID())
			for _, folder := range folders {
				if len(folders) > 1 {
					// 多段时显示源文件夹根：目录名 + 主要徽标；单段不加段头。
					mygo.Row(c).Height(treeRowHeight+6).Padding(0, 6, 0, 6).Gap(5).
						AlignItems(mygo.Center).Tooltip(folder.Path).Shrink(0).Children(func() {
						icon := "folder"
						if folder.Primary {
							icon = "folder-open"
						}
						Icon(c, icon).FontSize(tabIconSize).TextColor(pal.Text2)
						mygo.Text(c, folderBase(folder.Path)).FontSize(Fonts.Base()).FontWeight(500).TextColor(pal.Text)
						paneBadge(c, pal, "主要", pal.AccentSoft, pal.Accent)
					})
				}
				// Git 语义只覆盖主要目录（与 React 一致：附加目录无 Git 标记）。
				git := map[string]string{}
				if folder.Primary || folder.ID == "" {
					git = p.git
				}
				p.treeLevel(c, pal, folder.ID, ".", 0, selected, git)
			}
		})
	})
}

// flatItem 画过滤平铺的一条文件项。
func (p *Pane) flatItem(c *mygo.Context, pal *Palette, hit flatHit) {
	el := mygo.ButtonBase(c).FillWidth().MinHeight(fileItemHeight).Padding(7, 8, 7, 8).Gap(7).
		Radius(RadiusRow).AlignItems(mygo.Center)
	if el.Hovered() {
		el.Background(pal.BgHover)
	}
	el.Children(func() {
		Icon(c, fileIconName(hit.node.Name)).FontSize(tabIconSize).TextColor(fileIconColor(pal, hit.node.Name, false))
		mygo.Column(c).Grow(1).Children(func() {
			mygo.Text(c, hit.node.Name).FontSize(Fonts.SM()).TextColor(pal.Text)
			mygo.Text(c, hit.node.Path).FontSize(Fonts.XS()).TextColor(pal.Text4)
		})
		if code := p.git[hit.node.Path]; code != "" && hit.folderID == "" {
			paneGitBadge(c, pal, code)
		}
	})
	if el.Clicked() {
		p.openWorkspaceFile(hit.folderID, hit.node.Path)
	}
}

// treeLevel 递归渲染一层文件树：folderID 是本段所属源文件夹（空值表示
// 主要目录），path 是当前层目录（"." 表示该段根），depth 用于缩进。
// 每层只渲染自己的节点，子目录通过再次渲染自身实现递归；目录未展开时
// 不渲染子节点，减少无用工作量。
func (p *Pane) treeLevel(c *mygo.Context, pal *Palette, folderID, path string, depth int, selected string, git map[string]string) {
	nodes, ok := p.tree[folderKey(folderID, path)]
	if !ok {
		// 该目录内容还没加载时（例如根目录尚未读取），本层不渲染任何内容。
		return
	}
	for _, node := range nodes {
		node := node
		isDir := node.Type == "dir"
		key := folderKey(folderID, node.Path)
		isOpen := p.expanded[key]
		isSelected := selected == node.Path

		row := mygo.Row(c.Key(key)).FillWidth().Height(treeRowHeight).
			Padding(0, 6, 0, 6+float32(depth)*treeIndent).
			Gap(5).AlignItems(mygo.Center).Radius(RadiusBadge)
		if isSelected || row.Hovered() {
			row.Background(pal.BgHover)
		}
		row.Children(func() {
			// 目录行的展开箭头（展开时右转 90°）；文件行是同宽的隐藏占位。
			if isDir {
				mygo.Box(c).Width(12).Children(func() {
					chevron := Icon(c, "chevron-right").FontSize(12).TextColor(pal.Text4)
					if isOpen {
						chevron.Rotate(90)
					}
				})
			} else {
				mygo.Box(c).Width(12)
			}
			iconName := fileIconName(node.Name)
			if isDir {
				iconName = "folder"
				if isOpen {
					iconName = "folder-open"
				}
			}
			Icon(c, iconName).FontSize(13).TextColor(fileIconColor(pal, node.Name, isDir))
			if isDir {
				mygo.Text(c, node.Name).FontSize(Fonts.Base()).FontWeight(500).TextColor(pal.Text)
			} else {
				mygo.Text(c, node.Name).FontSize(Fonts.Base()).TextColor(pal.Text3)
			}
			if code := git[node.Path]; code != "" {
				paneGitBadge(c, pal, code)
			}
		})

		// 行右键菜单：打开 / 复制路径 / 文件管理器 / 废纸篓（features.md
		// F-WS-03「右键菜单」的落地）。
		row.PopoverMenu(func(m *mygo.Menu) {
			if isDir {
				if m.Item(paneDirToggleName(isOpen)).Chosen() {
					p.toggleDir(folderID, node.Path)
				}
			} else if m.Item("打开").Chosen() {
				p.openWorkspaceFile(folderID, node.Path)
			}
			if m.Item("复制路径").Chosen() {
				p.copyPath(folderID, node.Path)
			}
			m.Separator()
			if m.Item("在文件管理器中显示").Chosen() {
				p.revealInFolder(folderID, node.Path)
			}
			if m.Item("移到废纸篓").Chosen() {
				p.trashPath(folderID, node.Path)
			}
		})

		if row.Clicked() {
			// 目录点击只切换展开，文件点击才打开编辑器。
			if isDir {
				p.toggleDir(folderID, node.Path)
			} else {
				p.openWorkspaceFile(folderID, node.Path)
			}
		}

		if isDir && isOpen {
			p.treeLevel(c, pal, folderID, node.Path, depth+1, selected, git)
		}
	}
}

// paneDirToggleName 返回目录行菜单第一项的文案。
func paneDirToggleName(open bool) string {
	if open {
		return "收起"
	}
	return "展开"
}

// openWorkspaceFile 点击文件项的统一入口：Office / PDF 走文档预览（文本
// 级），其余读取文本内容并开 file tab。
func (p *Pane) openWorkspaceFile(folderID, path string) {
	lower := strings.ToLower(path)
	if strings.HasSuffix(lower, ".docx") || strings.HasSuffix(lower, ".xlsx") || strings.HasSuffix(lower, ".pdf") {
		p.openDocument(folderID, path)
		return
	}
	p.OpenFileTab(folderID, path, baseName(path))
}

// copyPath 复制文件的绝对路径到剪贴板。
func (p *Pane) copyPath(folderID, rel string) {
	if abs := p.absPath(folderID, rel); abs != "" {
		paneClipboardWrite(abs)
	}
}

// revealInFolder 在系统文件管理器里显示文件。
func (p *Pane) revealInFolder(folderID, rel string) {
	if abs := p.absPath(folderID, rel); abs != "" {
		paneShowInFolder(abs)
	}
}

// trashPath 把文件移到废纸篓；失败弹错误提示，成功后刷新文件树。
func (p *Pane) trashPath(folderID, rel string) {
	abs := p.absPath(folderID, rel)
	if abs == "" {
		return
	}
	go func() {
		err := paneTrash(abs)
		p.update(func() {
			if err != nil {
				p.reportError("无法移到废纸篓", err.Error())
				return
			}
			p.reloadTree()
		})
	}()
}

// absPath 把「源文件夹 + 相对路径」解析为绝对路径；解析不了返回空串。
func (p *Pane) absPath(folderID, rel string) string {
	ws := p.deps.Workspace()
	if ws == nil {
		return ""
	}
	for _, folder := range workspaceFolders(ws) {
		if folder.ID == folderID || (folderID == "" && folder.Primary) {
			return folder.Path + "/" + rel
		}
	}
	return ""
}

// paneBadge 画一枚小徽章（tree-section-badge 的通用形态）。
func paneBadge(c *mygo.Context, pal *Palette, label string, bg, fg mygo.Color) {
	mygo.Box(c).Padding(1, 5, 1, 5).Radius(RadiusBadge).Background(bg).Children(func() {
		mygo.Text(c, label).FontSize(Fonts.XS()).TextColor(fg)
	})
}

// paneGitBadge 画 Git 状态字母。
func paneGitBadge(c *mygo.Context, pal *Palette, code string) {
	mygo.Box(c).Width(12).Center().Children(func() {
		mygo.Text(c, code).FontSize(Fonts.XS()).TextColor(gitLetterColor(pal, code))
	})
}

// paneEmptyState 画面板空态（panel-empty）。
func paneEmptyState(c *mygo.Context, pal *Palette, icon, title, hint string) {
	mygo.Column(c).Grow(1).Center().Gap(6).Padding(16, 16, 16, 16).Children(func() {
		Icon(c, icon).FontSize(22).TextColor(pal.Text4)
		mygo.Text(c, title).TextColor(pal.Text2)
		mygo.Text(c, hint).FontSize(Fonts.SM()).TextColor(pal.Text3)
	})
}
