package ui

// 面板的编排层与 view：tab 条 + 内容区分派。
//
// 状态机（幂等 id、singleton、工作区分桶、归属过滤、最近关闭栈）逐行
// 移植在 internal/ui/panel/tabs.go（纯函数，单测对齐 sidePane.test.ts）；
// 本文件对齐 frontend/src/stores/panel.ts（编排者）与
// features/panel/Panel.tsx（tab 条渲染）：面板编排是多 tab 状态的唯一
// 写出口——打开 / 关闭 / 激活入口都经 commit 落账到「工作区记忆桶」，
// view 层只读状态。文件内容、文档预览等业务数据在数据槽（panel_editor.go /
// panel_docs.go），tab 只记录「显示什么」。
//
// 分桶（对齐 ZCode Side Pane）：每个工作区一个记忆桶（tabs + 最近关闭 +
// 按线程记住的激活项），切换工作区整桶切换，互不串台；桶只在内存（LRU
// 上限 50），tab 列表不持久化（重启后按入口重建，避免恢复失效路径）。
// 终端 singleton 跨工作区：打开时同步补进所有已存在的桶，新建桶时自带。

import (
	"fmt"
	"strings"
	"time"

	"github.com/egoist/mygo/plugins/terminal"
	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/service"
	cwpanel "github.com/yangshifu1024/craftwork/internal/ui/panel"
)

// 桶数量上限：防止工作区数量增长后内存无限上涨（对齐 panel store）。
const bucketLimit = 50

// 面板布局尺寸（panel.css 的 .panel-tabs / .panel-tab / .icon-btn.sm 等）。
const (
	tabBarHeight   = float32(48)
	tabIdealWidth  = float32(156)
	tabMinWidth    = float32(60)
	tabHeight      = float32(27)
	tabIconSize    = float32(14)
	tabCloseSize   = float32(16)
	tabActionBtn   = float32(28) // .icon-btn.sm
	actionIconSize = float32(15)
	editorBarH     = float32(31) // .editor-head min-height
	toolBtnHeight  = float32(22) // .tool-btn
	treeRowHeight  = float32(23)
	treeIndent     = float32(13) // 每层缩进
	fileItemHeight = float32(38)
	flatLimit      = 200 // 过滤平铺的条数上限
)

// PaneDeps 是面板对外部世界的最小依赖接口（由外壳装配注入）。
//
// Workspace / Thread 决定可见性 scope；Workspaces / Documents 是后端
// 服务（view 直调约定）；Update 把后台 goroutine 的结果切回主线程；
// Collapse 由外壳接线面板开合。
type PaneDeps struct {
	// Workspace 返回当前活动工作区；没有时返回 nil。
	Workspace func() *service.WorkspaceView
	// Thread 返回当前线程的原始 id（空值会归一为草稿归属）。05 号票接线。
	Thread func() string
	// WorkspacesSvc 返回工作区服务（文件树、读写与 Git 状态）；用 provider
	// 而不是值，让外壳可以在窗口装配期注入（NewPane 之后才拿到服务实例）。
	WorkspacesSvc func() *service.WorkspaceService
	// DocumentsSvc 返回文档预览服务（Word / Excel / PDF 文本级）。
	DocumentsSvc func() *service.DocumentService
	// Settings 返回当前设置快照（终端字号等）；nil 时用默认值。
	Settings func() service.Settings
	// Update 把回调切到主线程执行（窗口 win.Update）；为 nil 时同步执行。
	Update func(fn func())
	// Collapse 请求外壳收起面板（末 tab 关闭 / 关闭全部时）。
	Collapse func()
}

// paneBucket 是一个工作区的记忆桶：tab 状态与按线程记住的激活项。
type paneBucket struct {
	state             cwpanel.State
	activeTabByThread map[string]string
}

// Pane 持有右侧面板的全部界面状态。立即模式约定：只在主线程（view 或
// deps.Update 回调）里读写，不放锁。
type Pane struct {
	deps PaneDeps

	// 所有工作区的记忆桶；order 是 LRU 顺序（末尾最新）。
	buckets map[string]*paneBucket
	order   []string
	// terminalOpen 是终端 singleton 的全局开关：终端是跨工作区、跨线程
	// 的公共 tab，打开时同步补进所有桶，关闭时从其他桶静默移除。
	terminalOpen bool

	// file tab 的编辑器数据槽，key 是 tab id；槽与 tab 同生共死。
	files map[string]*FileSlot
	// 文档（Word / Excel / PDF）预览单槽与加载标记。
	doc        *service.DocumentPreview
	docLoading bool

	// 差异 singleton tab 的数据槽（单份全局，对齐 React 版 workspace
	// store 的 diff）与视图方式记忆（0 统一 / 1 并排，跨打开保留）。
	diff        *DiffSlot
	diffViewIdx int
	// 终端会话状态（plugins/terminal 自管 PTY）：见 panel_terminal.go。
	termRequested  bool
	term           *terminal.Terminal
	termErr        error
	termTitle      string
	termExited     bool
	termExitCode   int
	termGen        int
	termWsID       string
	termDir        string
	termFont       int
	termSettings   service.Settings
	termSettingsAt time.Time

	// 文件树状态：组合键（folderID|path）到子节点、展开状态、过滤关键字、
	// Git 字母与加载防重入标记。
	tree     map[string][]service.FileNode
	expanded map[string]bool
	filter   string
	git      map[string]string
	pending  map[string]bool
	// treeFor 是已完成根加载的工作区 id：切换工作区后重新加载根。
	treeFor string

	// savedAt 记录各 tab 最近一次保存成功的毫秒时间（07 号票 toast 合入
	// 前在编辑器状态条展示保存反馈）。
	savedAt map[string]int64
}

// NewPane 创建面板状态。
func NewPane(deps PaneDeps) *Pane {
	return &Pane{
		deps:     deps,
		buckets:  map[string]*paneBucket{},
		files:    map[string]*FileSlot{},
		tree:     map[string][]service.FileNode{},
		expanded: map[string]bool{},
		git:      map[string]string{},
		pending:  map[string]bool{},
		savedAt:  map[string]int64{},
	}
}

// update 在主线程执行 fn：后台 goroutine 的统一出口。
func (p *Pane) update(fn func()) {
	if p.deps.Update == nil {
		fn()
		return
	}
	p.deps.Update(fn)
}

// collapse 请求外壳收起面板；外壳未接线时忽略。
func (p *Pane) collapse() {
	if p.deps.Collapse != nil {
		p.deps.Collapse()
	}
}

// scope 取当前的面板作用域（工作区 + 归一化线程）。
func (p *Pane) scope() cwpanel.Scope {
	var id string
	if p.deps.Workspace != nil {
		if ws := p.deps.Workspace(); ws != nil {
			id = ws.ID
		}
	}
	thread := ""
	if p.deps.Thread != nil {
		thread = p.deps.Thread()
	}
	return cwpanel.Scope{WorkspaceID: id, ThreadID: cwpanel.NormalizeOwner(thread)}
}

// workspaces 返回工作区服务；未注入时返回 nil（面板各入口都会判空）。
func (p *Pane) workspaces() *service.WorkspaceService {
	if p.deps.WorkspacesSvc == nil {
		return nil
	}
	return p.deps.WorkspacesSvc()
}

// documents 返回文档预览服务；未注入时返回 nil。
func (p *Pane) documents() *service.DocumentService {
	if p.deps.DocumentsSvc == nil {
		return nil
	}
	return p.deps.DocumentsSvc()
}

// bucketFor 取工作区的记忆桶，读写都刷新 LRU 顺序；超过上限时淘汰最旧
// 的桶。新建桶时按终端全局开关自带终端 tab。
func (p *Pane) bucketFor(workspaceID string) *paneBucket {
	if existing, ok := p.buckets[workspaceID]; ok {
		p.touchBucket(workspaceID)
		return existing
	}
	created := &paneBucket{state: cwpanel.EmptySidePaneState(), activeTabByThread: map[string]string{}}
	if p.terminalOpen {
		created.state = cwpanel.OpenTab(created.state, cwpanel.MakeTerminalTab(cwpanel.Input{}), cwpanel.Scope{})
	}
	p.buckets[workspaceID] = created
	p.order = append(p.order, workspaceID)
	if len(p.order) > bucketLimit {
		delete(p.buckets, p.order[0])
		p.order = p.order[1:]
	}
	return created
}

// touchBucket 把工作区桶移到 LRU 末尾。
func (p *Pane) touchBucket(workspaceID string) {
	for i, id := range p.order {
		if id == workspaceID {
			rest := append([]string(nil), p.order[i+1:]...)
			p.order = append(p.order[:i:i], rest...)
			p.order = append(p.order, workspaceID)
			return
		}
	}
}

// ownerKey 是激活项记忆的键：工作区 + 线程。
func ownerKey(scope cwpanel.Scope) string {
	return scope.WorkspaceID + "::" + scope.ThreadID
}

// commitOption 控制 commit 的面板开合联动。
type commitOption struct {
	// collapseOnEmpty 在「当前上下文可见 tab 清空」时收起面板。
	collapseOnEmpty bool
}

// commit 是 tab 写出口的统一编排骨架：拿到当前桶 → 用纯函数算出新状态
// → 写回桶并记住激活项。mutate 返回 changed=false 表示没有变化，整次
// 提交作废。
func (p *Pane) commit(opts commitOption, mutate func(bucket *paneBucket, scope cwpanel.Scope) (cwpanel.State, bool)) {
	scope := p.scope()
	bucket := p.bucketFor(scope.WorkspaceID)
	next, changed := mutate(bucket, scope)
	if !changed {
		return
	}
	bucket.state = next
	if next.ActiveTabID != "" {
		bucket.activeTabByThread[ownerKey(scope)] = next.ActiveTabID
	}
	if opts.collapseOnEmpty && len(cwpanel.VisibleTabs(next, scope)) == 0 {
		// 「当前线程可见 tab 清空」时面板自动收起。
		p.collapse()
	}
}

// syncScope 工作区 / 线程切换后的恢复：按记忆恢复该上下文的激活 tab。
// tab 数据留在各自桶里；没有记忆时不改变激活项。
func (p *Pane) syncScope(bucket *paneBucket, scope cwpanel.Scope) {
	remembered := bucket.activeTabByThread[ownerKey(scope)]
	exists := func(id string) bool {
		if id == "" {
			return false
		}
		for _, tab := range bucket.state.Tabs {
			if tab.ID == id {
				return true
			}
		}
		return false
	}
	if remembered != "" && exists(remembered) {
		bucket.state.ActiveTabID = remembered
	} else if !exists(bucket.state.ActiveTabID) {
		bucket.state.ActiveTabID = ""
	}
}

// ---------------------------------------------------------------------------
// 打开类入口
// ---------------------------------------------------------------------------

// OpenFileTab 打开（或聚焦）一个文件 tab 并读取内容进数据槽。
func (p *Pane) OpenFileTab(folderID, path, title string) {
	workspaceID := p.scope().WorkspaceID
	if workspaceID == "" {
		return
	}
	p.commit(commitOption{}, func(bucket *paneBucket, scope cwpanel.Scope) (cwpanel.State, bool) {
		tab := cwpanel.MakeFileTab(cwpanel.Input{WorkspaceID: scope.WorkspaceID, ThreadID: scope.ThreadID, FolderID: folderID, Path: path, Title: title})
		next := cwpanel.OpenTab(bucket.state, tab, scope)
		next, _ = cwpanel.EnforceFileTabLimit(next, cwpanel.FileTabLimit)
		return next, true
	})
	p.loadFileSlot(cwpanel.FileTabID(workspaceID, folderID, path), folderID, path, true)
}

// OpenDocumentTab 打开（或聚焦）一个文档 tab。
func (p *Pane) OpenDocumentTab(folderID, path, title, kind string) {
	p.commit(commitOption{}, func(bucket *paneBucket, scope cwpanel.Scope) (cwpanel.State, bool) {
		tab := cwpanel.MakeDocumentTab(cwpanel.Input{WorkspaceID: scope.WorkspaceID, ThreadID: scope.ThreadID, FolderID: folderID, Path: path, Title: title, Kind: kind})
		next := cwpanel.OpenTab(bucket.state, tab, scope)
		next, _ = cwpanel.EnforceFileTabLimit(next, cwpanel.FileTabLimit)
		return next, true
	})
}

// OpenDiffTab 打开（或聚焦）差异 singleton tab。差异内容由 09 号票写入。
func (p *Pane) OpenDiffTab(title string) {
	p.commit(commitOption{}, func(bucket *paneBucket, scope cwpanel.Scope) (cwpanel.State, bool) {
		tab := cwpanel.MakeDiffTab(cwpanel.Input{WorkspaceID: scope.WorkspaceID, ThreadID: scope.ThreadID, Title: title})
		return cwpanel.OpenTab(bucket.state, tab, scope), true
	})
}

// OpenFilesTab 打开（或聚焦）文件树 singleton tab（工作区公共导航器）。
func (p *Pane) OpenFilesTab() {
	p.commit(commitOption{}, func(bucket *paneBucket, scope cwpanel.Scope) (cwpanel.State, bool) {
		tab := cwpanel.MakeFilesTab(cwpanel.Input{WorkspaceID: scope.WorkspaceID, ThreadID: scope.ThreadID})
		return cwpanel.OpenTab(bucket.state, tab, scope), true
	})
}

// OpenTerminalTab 打开（或聚焦）终端 singleton tab（全局公共）：置全局
// 开关并同步补进所有桶，当前桶聚焦。
func (p *Pane) OpenTerminalTab() {
	p.terminalOpen = true
	for _, id := range p.order {
		bucket := p.buckets[id]
		has := false
		for _, tab := range bucket.state.Tabs {
			if tab.ID == cwpanel.TerminalTabID {
				has = true
			}
		}
		if !has {
			bucket.state = cwpanel.OpenTab(bucket.state, cwpanel.MakeTerminalTab(cwpanel.Input{}), cwpanel.Scope{})
		}
	}
	p.commit(commitOption{}, func(bucket *paneBucket, _ cwpanel.Scope) (cwpanel.State, bool) {
		return cwpanel.ActivateTab(bucket.state, cwpanel.TerminalTabID), true
	})
}

// ---------------------------------------------------------------------------
// 关闭与激活入口
// ---------------------------------------------------------------------------

// ActivateTab 激活指定 tab 并记住它为当前上下文的激活项。
func (p *Pane) ActivateTab(tabID string) {
	p.commit(commitOption{}, func(bucket *paneBucket, _ cwpanel.Scope) (cwpanel.State, bool) {
		return cwpanel.ActivateTab(bucket.state, tabID), true
	})
}

// CloseTab 关闭一个 tab：入最近关闭栈；关闭的是激活项时自动激活邻居，
// 可见集清空时收起面板。file tab 的数据槽与 tab 同生共死。
func (p *Pane) CloseTab(tabID string) {
	// 关闭终端 tab：全局开关复位，其他工作区的桶静默移除；当前桶交给
	// 下面的统一提交（删除 + 入最近关闭栈）。终端会话不随 tab 关闭：
	// 重开 tab 重新接上（会话随窗口关闭 / 工作区切换 / 用户重起回收）。
	if tabID == cwpanel.TerminalTabID && p.terminalOpen {
		p.terminalOpen = false
		current := p.scope().WorkspaceID
		for id, bucket := range p.buckets {
			if id != current {
				next, removed := cwpanel.CloseTab(bucket.state, cwpanel.TerminalTabID)
				if len(removed) > 0 {
					bucket.state = next
				}
			}
		}
	}
	p.commit(commitOption{collapseOnEmpty: true}, func(bucket *paneBucket, scope cwpanel.Scope) (cwpanel.State, bool) {
		next, removed := cwpanel.CloseTab(bucket.state, tabID)
		if len(removed) == 0 {
			return bucket.state, false
		}
		return cwpanel.PushRecentClosed(next, removed), true
	})
	// 关闭 file tab 时同时清掉它的编辑器数据槽（内容、草稿、定位行），
	// 槽与 tab 同生共死。
	delete(p.files, tabID)
	delete(p.savedAt, tabID)
}

// CloseOtherTabs 关闭当前上下文可见 tab 中除目标之外的全部（隐藏 tab
// 不受影响）。
func (p *Pane) CloseOtherTabs(tabID string) {
	p.commit(commitOption{}, func(bucket *paneBucket, scope cwpanel.Scope) (cwpanel.State, bool) {
		next, removed := cwpanel.CloseOtherTabs(bucket.state, tabID, scope)
		if len(removed) == 0 {
			return bucket.state, false
		}
		return next, true
	})
}

// CloseAllTabs 关闭当前上下文可见的全部 tab，可见集清空时面板收起。
func (p *Pane) CloseAllTabs() {
	p.commit(commitOption{collapseOnEmpty: true}, func(bucket *paneBucket, scope cwpanel.Scope) (cwpanel.State, bool) {
		next, removed := cwpanel.CloseAllVisibleTabs(bucket.state, scope)
		if len(removed) == 0 {
			return bucket.state, false
		}
		return next, true
	})
}

// ReopenRecentClosed 从最近关闭栈重开一个 tab：校验当前上下文可见，
// 归属重置为当前线程，内容由渲染层按需重新加载。
func (p *Pane) ReopenRecentClosed(tabID string) {
	workspaceID := p.scope().WorkspaceID
	p.commit(commitOption{}, func(bucket *paneBucket, scope cwpanel.Scope) (cwpanel.State, bool) {
		taken, tab, ok := cwpanel.TakeRecentClosed(bucket.state, tabID, scope)
		if !ok {
			return bucket.state, false
		}
		tab.ThreadID = cwpanel.NormalizeOwner(scope.ThreadID)
		next := cwpanel.OpenTab(taken, tab, scope)
		next, _ = cwpanel.EnforceFileTabLimit(next, cwpanel.FileTabLimit)
		return next, true
	})
	// 重开的 file tab 内容已清（槽随关闭释放），按需重读。
	if tab := p.findTab(workspaceID, tabID); tab != nil && tab.Type == cwpanel.TabFile {
		p.loadFileSlot(tab.ID, tab.FolderID, tab.Path, false)
	}
}

// findTab 在指定工作区桶里找 tab；找不到返回 nil。
func (p *Pane) findTab(workspaceID, tabID string) *cwpanel.Tab {
	bucket, ok := p.buckets[workspaceID]
	if !ok {
		return nil
	}
	for i, tab := range bucket.state.Tabs {
		if tab.ID == tabID {
			return &bucket.state.Tabs[i]
		}
	}
	return nil
}

// PruneThread 清理某个线程名下的全部 tab（线程删除联动）；公共 tab 保留，
// 不入最近关闭栈；可见集清空时面板收起。
func (p *Pane) PruneThread(threadID string) {
	p.commit(commitOption{collapseOnEmpty: true}, func(bucket *paneBucket, _ cwpanel.Scope) (cwpanel.State, bool) {
		next, removed := cwpanel.PruneThreadTabs(bucket.state, threadID)
		if len(removed) == 0 {
			return bucket.state, false
		}
		return next, true
	})
}

// OpenFileAt 打开文件并定位到指定行（line<=0 表示不定位）。
//
// 路径先按主要目录解析，失败时跨源文件夹唯一匹配（LocateFile）。供聊天
// 正文引用与审批卡「查看完整差异」跳转使用（05 / 06 号票接线）。
func (p *Pane) OpenFileAt(path string, line int) {
	ws := p.deps.Workspace()
	if ws == nil || p.workspaces() == nil {
		p.reportError("无法打开引用的文件", "当前没有活动工作区")
		return
	}
	// 读取与解析是磁盘 IO，放后台做，不打断当前帧。
	go func() {
		folderID := ""
		if _, err := p.workspaces().ReadFile(ws.ID, "", path); err != nil {
			located, locateErr := p.workspaces().LocateFile(ws.ID, path)
			if locateErr != nil {
				p.reportError("无法打开引用的文件", "在工作区内找不到 "+path+"，请确认路径是否正确")
				return
			}
			folderID = located.FolderID
			path = located.Path
		}
		p.update(func() {
			p.openResolved(folderID, path, line)
		})
	}()
}

// openResolved 在主线程里打开已解析的文件并写入定位行。
func (p *Pane) openResolved(folderID, path string, line int) {
	title := baseName(path)
	p.OpenFileTab(folderID, path, title)
	if line > 0 {
		if slot := p.files[cwpanel.FileTabID(p.scope().WorkspaceID, folderID, path)]; slot != nil {
			slot.pendingLine = line
		}
	}
}

// reportError 弹一条错误提示。07 号票浮层体系合入前用原生对话框兜底
// （panel_desktop.go 的 paneReportError，单测可替换），届时替换为应用内
// toast。
func (p *Pane) reportError(title, detail string) {
	paneReportError(title, detail)
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

// View 画整个面板：tab 条 + 内容区。panelW 是面板列宽（外壳传入，用于
// 均分 tab 宽度）。
func (p *Pane) View(c *mygo.Context, pal *Palette, panelW float32) {
	scope := p.scope()
	bucket := p.bucketFor(scope.WorkspaceID)
	p.syncScope(bucket, scope)
	visible := cwpanel.VisibleTabs(bucket.state, scope)
	activeID := cwpanel.VisibleActiveTabID(bucket.state, scope)

	mygo.Column(c).Fill().
		Background(pal.SidebarBg).
		Children(func() {
			p.tabBar(c, pal, visible, activeID, bucket.state, scope, panelW)
			p.content(c, pal, visible, activeID)
		})
}

// currentVisible 与 currentActiveID 是渲染辅助：取当前上下文的可见 tab
// 与兜底激活项（文件树的高亮等只读场景用）。
func (p *Pane) currentVisible() []cwpanel.Tab {
	scope := p.scope()
	return cwpanel.VisibleTabs(p.bucketFor(scope.WorkspaceID).state, scope)
}

func (p *Pane) currentActiveID() string {
	scope := p.scope()
	bucket := p.bucketFor(scope.WorkspaceID)
	p.syncScope(bucket, scope)
	return cwpanel.VisibleActiveTabID(bucket.state, scope)
}

// tabIconName 返回 tab 类型对应的登记图标名（Panel.tsx tabIcon）。
func tabIconName(tab cwpanel.Tab) string {
	switch tab.Type {
	case cwpanel.TabFiles:
		return "folder-open"
	case cwpanel.TabFile:
		return "file-code"
	case cwpanel.TabDiff:
		return "diff"
	case cwpanel.TabTerminal:
		return "terminal"
	case cwpanel.TabDocument:
		return "file-text"
	}
	return "file"
}

// tabWidth 均分 tab 条的可用宽度：理想 156，不足时全体收缩到下限 60
// （panel.css：flex-basis 156 + min-width 60 + max-width 156）。
func tabWidth(panelW float32, actionsW float32, n int) float32 {
	if n <= 0 {
		return tabIdealWidth
	}
	avail := panelW - 12 - actionsW - float32(n) - 1 // 左右内边距 6*2、tab 间距 1、右缘余量
	if avail < 0 {
		avail = 0
	}
	w := avail / float32(n)
	if w > tabIdealWidth {
		return tabIdealWidth
	}
	if w < tabMinWidth {
		return tabMinWidth
	}
	return w
}

// tabBar 画 tab 条：当前上下文可见的 tab（点击切换、中键或关闭按钮关闭、
// 右键弹菜单）+ singleton 快捷入口（还没有对应 tab 时才显示）。
func (p *Pane) tabBar(c *mygo.Context, pal *Palette, visible []cwpanel.Tab, activeID string, state cwpanel.State, scope cwpanel.Scope, panelW float32) {
	hasFiles, hasTerminal, hasDiff := false, false, false
	for _, tab := range visible {
		switch tab.Type {
		case cwpanel.TabFiles:
			hasFiles = true
		case cwpanel.TabTerminal:
			hasTerminal = true
		case cwpanel.TabDiff:
			hasDiff = true
		}
	}
	actions := 0
	if !hasFiles {
		actions++
	}
	if !hasTerminal {
		actions++
	}
	if !hasDiff {
		actions++
	}
	actionsW := float32(actions) * tabActionBtn
	width := tabWidth(panelW, actionsW, len(visible))

	// 右键菜单要用的重开列表：当前上下文可恢复的最近关闭 tab。
	reopenable := make([]cwpanel.Tab, 0, len(state.RecentClosed))
	for _, tab := range state.RecentClosed {
		if cwpanel.IsTabVisibleInScope(tab, scope) {
			reopenable = append(reopenable, tab)
		}
	}

	mygo.Row(c).Height(tabBarHeight).
		Padding(0, 6, 0, 6).
		Gap(1).
		AlignItems(mygo.Center).
		Background(pal.SidebarBg).
		BorderWidth(0, 0, 1, 0).
		BorderColor(pal.Border).
		Shrink(0).
		Children(func() {
			for _, tab := range visible {
				p.tabItem(c, pal, tab, tab.ID == activeID, width, reopenable)
			}
			mygo.Spacer(c)
			p.shortcutButtons(c, pal, hasFiles, hasTerminal, hasDiff)
		})
}

// shortcutButtons 画 singleton 快捷入口（React：对应 tab 缺失时才显示）。
func (p *Pane) shortcutButtons(c *mygo.Context, pal *Palette, hasFiles, hasTerminal, hasDiff bool) {
	if !hasFiles {
		if paneIconButton(c, pal, "folder-open", "打开文件树") {
			p.OpenFilesTab()
		}
	}
	if !hasTerminal {
		if paneIconButton(c, pal, "terminal", "打开终端") {
			p.OpenTerminalTab()
		}
	}
	if !hasDiff {
		if paneIconButton(c, pal, "diff", "打开差异") {
			p.OpenDiffTab("差异")
		}
	}
}

// paneIconButton 画一个 28×28 的 icon 按钮（.icon-btn.sm），返回是否被点击。
func paneIconButton(c *mygo.Context, pal *Palette, name, tip string) bool {
	el := mygo.ButtonBase(c).Size(tabActionBtn, tabActionBtn).Radius(RadiusCtl).
		Tooltip(tip)
	if el.Hovered() {
		el.Background(pal.BgHover)
	}
	el.Children(func() {
		Icon(c, name).FontSize(actionIconSize).TextColor(pal.Text2)
	})
	return el.Clicked()
}

// tabItem 画 tab 条上的单个 tab：主按钮（图标 + 标题）+ 关闭按钮；
// 中键关闭；右键弹菜单。宽度 156→60 收缩带 Size 过渡动画。
func (p *Pane) tabItem(c *mygo.Context, pal *Palette, tab cwpanel.Tab, active bool, width float32, reopenable []cwpanel.Tab) {
	row := mygo.Row(c.Key(tab.ID)).
		Width(width).
		Height(tabHeight).
		Padding(0, 4, 0, 9).
		Gap(5).
		AlignItems(mygo.Center).
		Radius(RadiusRow).
		Shrink(0).
		ClipX().
		// 宽度收缩 / 底色的过渡动画（面板 spec：宽度 156→60 收缩）。
		Transition(mygo.ElementTransition{
			Size:     true,
			Colors:   true,
			Duration: Duration,
			Ease:     Ease,
		})

	hover := row.Hovered()
	if active || hover {
		row.Background(pal.BgHover)
	}
	textColor := pal.Text3
	if active || hover {
		textColor = pal.Text
	}
	icoColor := textColor
	if active {
		// 激活 tab 的图标用模式强调色（Code 模式为主题 accent；Work 模式
		// 的琥珀强调色选择归 04 号票的模式接线）。
		icoColor = pal.Accent
	}

	// 中键（button == 2）在按下阶段直接关闭，不激活 tab、不产生 click
	// （React 的 auxClick 语义）。其余事件放行。
	row.HandleInput(func(ev mygo.InputEvent) bool {
		if ev.Kind == mygo.InputPointerDown && ev.Button == 2 {
			p.CloseTab(tab.ID)
			return true
		}
		return false
	})

	// tab 右键菜单：关闭类操作 + 当前上下文可恢复的最近关闭列表。
	row.PopoverMenu(func(m *mygo.Menu) {
		if m.Item("重新打开关闭的标签页").Disabled(true).Chosen() {
			// 分区标题，不可选。
			return
		}
		if m.Item("关闭").Chosen() {
			p.CloseTab(tab.ID)
		}
		if m.Item("关闭其他").Chosen() {
			p.CloseOtherTabs(tab.ID)
		}
		if m.Item("关闭全部").Chosen() {
			p.CloseAllTabs()
		}
		m.Separator()
		if len(reopenable) == 0 {
			m.Item("没有可恢复的标签页").Disabled(true)
		} else {
			for _, closed := range reopenable {
				if m.Item(closed.Title).Chosen() {
					p.ReopenRecentClosed(closed.ID)
				}
			}
		}
	})

	row.Children(func() {
		// 主按钮：激活 tab。
		main := mygo.ButtonBase(c).Grow(1).Height(tabHeight).Gap(5).
			AlignItems(mygo.Center).Tooltip(tab.Title)
		main.Children(func() {
			Icon(c, tabIconName(tab)).FontSize(tabIconSize).TextColor(icoColor)
			mygo.Text(c, tab.Title).FontSize(Fonts.Base()).TextColor(textColor)
		})
		if main.Clicked() {
			p.ActivateTab(tab.ID)
		}
		// 关闭按钮：激活 tab 或 hover 时显示（React 的 opacity 渐入在
		// 立即模式下简化为条件构建）。
		if active || hover {
			closeBtn := mygo.ButtonBase(c).Size(tabCloseSize, tabCloseSize).
				Radius(RadiusBadge).Tooltip("关闭 " + tab.Title)
			if closeBtn.Hovered() {
				closeBtn.Background(pal.BgHover)
			}
			closeBtn.Children(func() {
				Icon(c, "x").FontSize(10).TextColor(pal.Text4)
			})
			if closeBtn.Clicked() {
				p.CloseTab(tab.ID)
			}
		} else {
			mygo.Box(c).Width(tabCloseSize)
		}
	})
}

// content 画内容区：按激活 tab 分派；没有激活 tab 时显示文件树兜底视图。
func (p *Pane) content(c *mygo.Context, pal *Palette, visible []cwpanel.Tab, activeID string) {
	var active *cwpanel.Tab
	for i := range visible {
		if visible[i].ID == activeID {
			active = &visible[i]
			break
		}
	}
	if active == nil {
		// 兜底文件树：没有激活 tab 时显示。
		p.filesView(c, pal)
		return
	}
	switch active.Type {
	case cwpanel.TabFiles:
		p.filesView(c, pal)
	case cwpanel.TabFile:
		p.editorView(c, pal, *active)
	case cwpanel.TabDiff:
		// 差异 tab：数据槽为空时由 diffView 自显空态。
		p.diffView(c, pal)
	case cwpanel.TabTerminal:
		// 终端 tab：会话惰性创建与重起都在 terminalView 里保证。
		p.terminalView(c, pal)
	case cwpanel.TabDocument:
		p.docView(c, pal, *active)
	default:
		p.filesView(c, pal)
	}
}

// baseName 取路径的最后一段作为展示名。
func baseName(path string) string {
	if index := strings.LastIndex(path, "/"); index >= 0 {
		return path[index+1:]
	}
	return path
}

// folderBase 取路径的最后一段作为段头展示名；空路径原样返回。
func folderBase(path string) string {
	trimmed := strings.TrimRight(path, "/")
	if trimmed == "" {
		return path
	}
	return baseName(trimmed)
}

// formatBytes 把字节数转成可读大小（utils.formatBytes 的 Go 版）。
func formatBytes(n int64) string {
	switch {
	case n < 1024:
		return fmt.Sprintf("%d B", n)
	case n < 1024*1024:
		return fmt.Sprintf("%.1f KB", float64(n)/1024)
	default:
		return fmt.Sprintf("%.1f MB", float64(n)/1024/1024)
	}
}

// paneSavedTime 把保存时间格式化为 HH:MM。
func paneSavedTime(ms int64) string {
	return time.UnixMilli(ms).Format("15:04")
}

// CloseActiveTab 关闭当前工作区桶的激活 tab（命令面板「关闭面板当前
// 标签页」用）；没有激活 tab 时无事发生。
func (p *Pane) CloseActiveTab() {
	b := p.bucketFor(p.scope().WorkspaceID)
	if b == nil || b.state.ActiveTabID == "" {
		return
	}
	p.CloseTab(b.state.ActiveTabID)
}
