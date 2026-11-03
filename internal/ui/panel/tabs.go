// Package panel 是右侧面板：多标签页状态机（tabs.go）与面板 view。
//
// tabs.go 逐行移植 frontend/src/lib/sidePane.ts（519 行纯函数）：只提供
// 类型与不可变转移函数，不依赖 view 与外壳；面板的编排层（panel.go）是
// 这些函数的唯一写出口，view 层只读状态、从不直接修改 tabs。
//
// 核心设计（与 TS 版一致）：
//  1. 结构化 id = 幂等打开。singleton 类型（diff / terminal）的 id 是固定
//     字面量，内容类型（file / document）的 id 携带工作区、文件夹与路径，
//     重复打开天然命中同一条 tab，无需额外的 singleton 标记位。
//  2. 一份 tabs 数组 + 可见性过滤。所有线程与工作区共享同一个数组，归属
//     （workspaceId + threadId）在打开时冻结，切换上下文只是过滤视图，
//     隐藏的 tab 不销毁。
//  3. 归属冻结后绝不漂移：盖戳函数只处理「未认领」的 tab，已带归属的
//     永远不被覆盖。
//
// TS → Go 的表示约定：
//   - threadId 的 null（未认领）映射为空字符串；已认领的归属经
//     NormalizeOwner 归一化（草稿为 DraftOwner），永不为空串。
//   - activeTabId 的 null 映射为空字符串。
//   - openedAt / activatedAt 是毫秒时间戳，缺省值统一取 theme.Now()。
package panel

import (
	"cmp"
	"slices"
	"strings"

	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// SidePaneTabType 面板 tab 的类型集合。会话派生与浏览器类类型属于路线图，
// 暂不进入本集合。
type SidePaneTabType string

// 五类 tab 类型（与 TS 的 SidePaneTabType 联合一一对应）。
const (
	TabFiles    SidePaneTabType = "files"
	TabFile     SidePaneTabType = "file"
	TabDiff     SidePaneTabType = "diff"
	TabTerminal SidePaneTabType = "terminal"
	TabDocument SidePaneTabType = "document"
)

// singleton tab 的固定 id：重复打开只聚焦并刷新内容。
const (
	// DiffTabID 是差异 tab 的固定 id。
	DiffTabID = "diff"
	// TerminalTabID 是终端 tab 的固定 id：singleton，且是跨工作区、
	// 跨线程的公共 tab。
	TerminalTabID = "terminal"
	// FilesTabID 是文件树 tab 的固定 id：singleton，工作区公共（跨线程
	// 可见、随工作区隔离）。文件树是导航器 tab：file tab 激活时编辑器
	// 占据内容区，用户经「文件」tab 回到树再打开其他文件。
	FilesTabID = "files"
)

// DraftOwner 是草稿线程（尚未发送首条消息）的归属标识。
const DraftOwner = "draft"

// 最近关闭栈的长度上限。
const RecentClosedLimit = 8

// FileTabLimit 是 file tab 的数量上限：超出时自动关闭最久未激活的
// （不入最近关闭栈）。
const FileTabLimit = 8

// Tab 是面板 tab：五类类型共用一个结构，file / document 独有的字段在
// 其他类型上保持零值（与 TS 的可辨识联合相比少了类型收窄，换来纯值
// 语义；类型分派用 Type 字段 switch）。
type Tab struct {
	// ID 是结构化 id，幂等键；singleton 类型为固定字面量。
	ID string
	// Type 是 tab 类型。
	Type SidePaneTabType
	// Title 是 tab 条上显示的标题。
	Title string
	// WorkspaceID 是打开时冻结的所属工作区（分桶键）。空字符串表示
	// 全局公共（跨工作区可见），当前只有终端使用。
	WorkspaceID string
	// ThreadID 是打开时冻结的归属线程；空字符串表示尚未认领（等待盖戳）。
	// 注意这与「草稿」不同：草稿是已归一化的归属值 DraftOwner。
	ThreadID string
	// OpenedAt 是打开时间戳（毫秒），用于展示与淘汰排序兜底。
	OpenedAt int64
	// ActivatedAt 是最近一次激活的时间戳；淘汰 file tab 时按它挑最久
	// 未看的。零值回退 OpenedAt。
	ActivatedAt int64

	// FolderID 是所属源文件夹（file / document）；空字符串表示主要目录。
	FolderID string
	// Path 是相对所属源文件夹的路径（file / document）。
	Path string
	// Kind 是文档类型（docx / xlsx / pdf / text），来自后端预览结果的
	// kind（document）。
	Kind string
}

// State 是面板的多 tab 状态：一份共享数组 + 激活项 + 最近关闭栈。
type State struct {
	// Tabs 是全部 tab（含当前线程不可见的隐藏 tab）。
	Tabs []Tab
	// ActiveTabID 是激活 tab 的 id。允许指向当前线程不可见的 tab（切走
	// 时保留别人的激活项），渲染层用 VisibleActiveTabID 兜底。空串表示
	// 没有激活 tab。
	ActiveTabID string
	// RecentClosed 是最近关闭栈，新的在前，上限 RecentClosedLimit。
	RecentClosed []Tab
}

// Scope 是可见性判定的工作区与线程上下文；ThreadID 应已归一化
// （草稿为 DraftOwner）。
type Scope struct {
	WorkspaceID string
	ThreadID    string
}

// Input 是各 Make 构造函数的入参；OpenedAt 为零值时取当前时间。
// ThreadID 传空串表示「未认领，等待盖戳」（TS 的 null）。
type Input struct {
	WorkspaceID string
	ThreadID    string
	FolderID    string
	Path        string
	Title       string
	Kind        string
	OpenedAt    int64
}

// clock 返回当前毫秒时间戳，作 openedAt / activatedAt 的缺省值；统一
// 走 theme.Now（此前本包自带原子可换时钟，测试固定时间用显式传
// Input.OpenedAt 已足够，注入机制无人使用，随时钟收敛一并删除）。
func clock() int64 { return theme.Now() }

// NormalizeOwner 把线程 id 归一化为归属值：空值归一为草稿。
func NormalizeOwner(threadID string) string {
	trimmed := strings.TrimSpace(threadID)
	if trimmed == "" {
		return DraftOwner
	}
	return trimmed
}

// FileTabID 构造 file tab 的结构化 id：工作区 + 文件夹 + 路径共同保证唯一。
func FileTabID(workspaceID, folderID, path string) string {
	return "file:" + workspaceID + ":" + folderID + ":" + path
}

// DocumentTabID 构造 document tab 的结构化 id。
func DocumentTabID(workspaceID, folderID, path string) string {
	return "document:" + workspaceID + ":" + folderID + ":" + path
}

// MakeFileTab 构造一个 file tab；归属由调用方给出（未知的线程传空串
// 等待盖戳）。
func MakeFileTab(input Input) Tab {
	openedAt := input.OpenedAt
	if openedAt == 0 {
		openedAt = clock()
	}
	return Tab{
		ID:          FileTabID(input.WorkspaceID, input.FolderID, input.Path),
		Type:        TabFile,
		Title:       input.Title,
		WorkspaceID: input.WorkspaceID,
		ThreadID:    input.ThreadID,
		OpenedAt:    openedAt,
		ActivatedAt: openedAt,
		FolderID:    input.FolderID,
		Path:        input.Path,
	}
}

// MakeDocumentTab 构造一个 document tab。
func MakeDocumentTab(input Input) Tab {
	openedAt := input.OpenedAt
	if openedAt == 0 {
		openedAt = clock()
	}
	return Tab{
		ID:          DocumentTabID(input.WorkspaceID, input.FolderID, input.Path),
		Type:        TabDocument,
		Title:       input.Title,
		WorkspaceID: input.WorkspaceID,
		ThreadID:    input.ThreadID,
		OpenedAt:    openedAt,
		ActivatedAt: openedAt,
		FolderID:    input.FolderID,
		Path:        input.Path,
		Kind:        input.Kind,
	}
}

// MakeDiffTab 构造差异 singleton tab；WorkspaceID / ThreadID 记录打开时
// 的上下文。Title 为空时取「差异」。
func MakeDiffTab(input Input) Tab {
	openedAt := input.OpenedAt
	if openedAt == 0 {
		openedAt = clock()
	}
	title := input.Title
	if title == "" {
		title = "差异"
	}
	return Tab{
		ID:          DiffTabID,
		Type:        TabDiff,
		Title:       title,
		WorkspaceID: input.WorkspaceID,
		ThreadID:    input.ThreadID,
		OpenedAt:    openedAt,
		ActivatedAt: openedAt,
	}
}

// MakeTerminalTab 构造终端 singleton tab；WorkspaceID 为空表示全局公共。
func MakeTerminalTab(input Input) Tab {
	openedAt := input.OpenedAt
	if openedAt == 0 {
		openedAt = clock()
	}
	return Tab{
		ID:          TerminalTabID,
		Type:        TabTerminal,
		Title:       "终端",
		WorkspaceID: "",
		ThreadID:    input.ThreadID,
		OpenedAt:    openedAt,
		ActivatedAt: openedAt,
	}
}

// MakeFilesTab 构造文件树 singleton tab；工作区公共，归属当前工作区
// （跨线程可见）。
func MakeFilesTab(input Input) Tab {
	openedAt := input.OpenedAt
	if openedAt == 0 {
		openedAt = clock()
	}
	return Tab{
		ID:          FilesTabID,
		Type:        TabFiles,
		Title:       "文件",
		WorkspaceID: input.WorkspaceID,
		ThreadID:    input.ThreadID,
		OpenedAt:    openedAt,
		ActivatedAt: openedAt,
	}
}

// EmptySidePaneState 返回空的面板状态。
func EmptySidePaneState() State {
	return State{}
}

// publicTabTypes 是跨线程可见的工作区公共 tab 类型集合；终端另外跨
// 工作区（WorkspaceID 为空）。
func publicTabType(t SidePaneTabType) bool {
	return t == TabFiles || t == TabTerminal
}

// IsTabVisibleInScope 判断 tab 在给定工作区与线程上下文下是否可见。
//
// 规则（spec 第 3 节）：
// 1. WorkspaceID 非空时必须与当前工作区一致（空 = 全局公共）；
// 2. 公共类型（terminal）直接可见；
// 3. 其余类型要求归属线程与当前线程一致（草稿与草稿互相匹配）。
func IsTabVisibleInScope(tab Tab, scope Scope) bool {
	if tab.WorkspaceID != "" && tab.WorkspaceID != scope.WorkspaceID {
		return false
	}
	if publicTabType(tab.Type) {
		return true
	}
	return NormalizeOwner(tab.ThreadID) == NormalizeOwner(scope.ThreadID)
}

// VisibleTabs 过滤出当前上下文可见的 tab 子集（保持原顺序）。
func VisibleTabs(state State, scope Scope) []Tab {
	out := make([]Tab, 0, len(state.Tabs))
	for _, tab := range state.Tabs {
		if IsTabVisibleInScope(tab, scope) {
			out = append(out, tab)
		}
	}
	return out
}

// VisibleActiveTabID 解析当前上下文实际应显示的激活 tab。
//
// ActiveTabID 指向可见 tab 时直接使用；指向隐藏 tab（切走前的遗留）时
// 回落到最后一个可见 tab；没有任何可见 tab 时为空串。
func VisibleActiveTabID(state State, scope Scope) string {
	if state.ActiveTabID != "" {
		for _, tab := range state.Tabs {
			if tab.ID == state.ActiveTabID && IsTabVisibleInScope(tab, scope) {
				return tab.ID
			}
		}
	}
	visible := VisibleTabs(state, scope)
	if len(visible) == 0 {
		return ""
	}
	return visible[len(visible)-1].ID
}

// MarkTabsOwnership 给未认领的 tab 盖上归属戳（打开时冻结工作区与线程）。
//
// 只处理 ThreadID 为空串的 tab；已带归属的永远不覆盖——归属是 tab 的
// 身份属性，随上下文漂移会让「切回去还看得到」的承诺失效。
// 没有可盖戳的 tab 时返回原状态（no-op 约定）。
func MarkTabsOwnership(state State, scope Scope) State {
	changed := false
	tabs := slices.Clone(state.Tabs)
	for i, tab := range tabs {
		if tab.ThreadID != "" {
			continue
		}
		changed = true
		tabs[i].ThreadID = NormalizeOwner(scope.ThreadID)
	}
	if !changed {
		return state
	}
	state.Tabs = tabs
	return state
}

// OpenTab 打开一个 tab（幂等）。
//
// 已存在同 id 的 tab：激活并合并可刷新字段（标题等展示信息以新值为准），
// tab 数组顺序不变；不存在：盖戳后追加到末尾并激活。
// 两种情况都刷新激活时间，供 file tab 淘汰排序使用。
func OpenTab(state State, tab Tab, scope Scope) State {
	index := slices.IndexFunc(state.Tabs, func(existing Tab) bool { return existing.ID == tab.ID })
	owned := tab
	if owned.ThreadID == "" {
		owned.ThreadID = NormalizeOwner(scope.ThreadID)
	}
	now := clock()
	if index >= 0 {
		tabs := slices.Clone(state.Tabs)
		merged := owned
		merged.OpenedAt = tabs[index].OpenedAt
		merged.ActivatedAt = now
		// 已存在的 tab 若尚未认领（旧数据），借打开时机补盖归属。
		if merged.ThreadID == "" {
			merged.ThreadID = NormalizeOwner(scope.ThreadID)
		}
		tabs[index] = merged
		state.Tabs = tabs
		state.ActiveTabID = tab.ID
		return state
	}
	state.Tabs = append(slices.Clone(state.Tabs), owned)
	state.ActiveTabID = owned.ID
	return state
}

// ActivateTab 激活指定 tab；id 不存在时返回原状态。激活会刷新该 tab 的
// 激活时间。
func ActivateTab(state State, tabID string) State {
	index := slices.IndexFunc(state.Tabs, func(tab Tab) bool { return tab.ID == tabID })
	if index < 0 {
		return state
	}
	tabs := slices.Clone(state.Tabs)
	tabs[index].ActivatedAt = clock()
	state.Tabs = tabs
	state.ActiveTabID = tabID
	return state
}

// removeTabsByIDs 从数组中删除一批 tab 的公共实现：返回删除的 tab 列表
// 与新状态。
func removeTabsByIDs(state State, ids map[string]bool) (State, []Tab) {
	if len(ids) == 0 {
		return state, nil
	}
	removed := make([]Tab, 0, len(ids))
	tabs := make([]Tab, 0, len(state.Tabs))
	for _, tab := range state.Tabs {
		if ids[tab.ID] {
			removed = append(removed, tab)
			continue
		}
		tabs = append(tabs, tab)
	}
	if len(removed) == 0 {
		return state, nil
	}
	activeTabID := state.ActiveTabID
	if activeTabID != "" && ids[activeTabID] {
		// 激活项被删除：激活它在旧数组中位置之后的第一个存活 tab（跨过
		// 同批被删的），后面没有存活 tab 则取前面最后一个；全部删光则
		// 为空串。
		anchor := slices.IndexFunc(state.Tabs, func(tab Tab) bool { return tab.ID == activeTabID })
		var next Tab
		for i := anchor + 1; i < len(state.Tabs) && next.ID == ""; i++ {
			if !ids[state.Tabs[i].ID] {
				next = state.Tabs[i]
			}
		}
		if next.ID == "" && len(tabs) > 0 {
			next = tabs[len(tabs)-1]
		}
		activeTabID = next.ID
	}
	state.Tabs = tabs
	state.ActiveTabID = activeTabID
	return state, removed
}

// CloseTab 关闭一个 tab。
//
// 关闭激活项时激活原位置右侧的第一个存活 tab（邻居选择），与主流编辑器
// 的关闭行为一致；返回被删除的 tab 供调用方入最近关闭栈。
func CloseTab(state State, tabID string) (State, []Tab) {
	return removeTabsByIDs(state, map[string]bool{tabID: true})
}

// CloseOtherTabs 关闭当前上下文可见 tab 中除 keepTabID 之外的全部
// （「关闭其他」）。
//
// 隐藏 tab（其他线程 / 其他工作区的）不属于当前上下文，不在关闭范围；
// 完成后 keepTabID 若可见则被激活。
func CloseOtherTabs(state State, keepTabID string, scope Scope) (State, []Tab) {
	keepIndex := slices.IndexFunc(state.Tabs, func(tab Tab) bool { return tab.ID == keepTabID })
	if keepIndex < 0 || !IsTabVisibleInScope(state.Tabs[keepIndex], scope) {
		return state, nil
	}
	ids := map[string]bool{}
	for _, tab := range VisibleTabs(state, scope) {
		if tab.ID != keepTabID {
			ids[tab.ID] = true
		}
	}
	next, removed := removeTabsByIDs(state, ids)
	if next.ActiveTabID != keepTabID {
		next = ActivateTab(next, keepTabID)
	}
	return next, removed
}

// CloseAllVisibleTabs 关闭当前上下文可见的全部 tab（「关闭全部」）；
// 隐藏 tab 不受影响。
func CloseAllVisibleTabs(state State, scope Scope) (State, []Tab) {
	ids := map[string]bool{}
	for _, tab := range VisibleTabs(state, scope) {
		ids[tab.ID] = true
	}
	return removeTabsByIDs(state, ids)
}

// PushRecentClosed 把关闭的 tab 压入最近关闭栈。
//
// 新的在前；同 id 的旧记录先移除（重开再关不产生重复项）；超过上限从
// 栈尾淘汰。singleton 与普通 tab 一视同仁。
func PushRecentClosed(state State, closed []Tab) State {
	if len(closed) == 0 {
		return state
	}
	closedIDs := map[string]bool{}
	for _, tab := range closed {
		closedIDs[tab.ID] = true
	}
	merged := make([]Tab, 0, len(closed)+len(state.RecentClosed))
	merged = append(merged, closed...)
	for _, tab := range state.RecentClosed {
		if !closedIDs[tab.ID] {
			merged = append(merged, tab)
		}
	}
	if len(merged) > RecentClosedLimit {
		merged = merged[:RecentClosedLimit]
	}
	state.RecentClosed = merged
	return state
}

// TakeRecentClosed 从最近关闭栈取出一条记录准备重开。
//
// 校验该记录在当前上下文可见（防止跨工作区、跨线程泄漏别的线程的 tab）；
// 校验通过则从栈中移除并返回该 tab，由调用方按打开流程重建内容；不可见
// 或不存在时 ok 为 false 且状态不变。
func TakeRecentClosed(state State, tabID string, scope Scope) (State, Tab, bool) {
	index := slices.IndexFunc(state.RecentClosed, func(tab Tab) bool { return tab.ID == tabID })
	if index < 0 || !IsTabVisibleInScope(state.RecentClosed[index], scope) {
		return state, Tab{}, false
	}
	tab := state.RecentClosed[index]
	state.RecentClosed = slices.Delete(slices.Clone(state.RecentClosed), index, index+1)
	return state, tab, true
}

// PruneThreadTabs 删除某个线程名下的全部 tab（线程删除联动）；公共类型
// 保留，不入最近关闭栈。
func PruneThreadTabs(state State, threadID string) (State, []Tab) {
	owner := NormalizeOwner(threadID)
	ids := map[string]bool{}
	for _, tab := range state.Tabs {
		if !publicTabType(tab.Type) && NormalizeOwner(tab.ThreadID) == owner {
			ids[tab.ID] = true
		}
	}
	return removeTabsByIDs(state, ids)
}

// ReorderTabs 拖拽排序：把 activeTabID 移动到 overTabID 的位置。
//
// 只改变数组顺序，不改变激活项（拖拽结束留在原激活 tab 上，与 ZCode
// 一致）；任一 id 不存在或相同位置时返回原状态。
func ReorderTabs(state State, activeTabID, overTabID string) State {
	if activeTabID == overTabID {
		return state
	}
	from := slices.IndexFunc(state.Tabs, func(tab Tab) bool { return tab.ID == activeTabID })
	to := slices.IndexFunc(state.Tabs, func(tab Tab) bool { return tab.ID == overTabID })
	if from < 0 || to < 0 {
		return state
	}
	tabs := slices.Clone(state.Tabs)
	moved := tabs[from]
	tabs = slices.Delete(tabs, from, from+1)
	tabs = slices.Insert(tabs, to, moved)
	state.Tabs = tabs
	return state
}

// EnforceFileTabLimit 执行 file tab 数量上限（spec 第 6 节）：超出时淘汰
// 最久未激活的 file tab。
//
// 淘汰走真正的删除（返回 evicted 供调用方清理数据槽），但按约定**不入**
// 最近关闭栈——自动清理不是用户动作，把被清理的文件塞进恢复列表反而让
// 人困惑。其他类型不受上限约束。
func EnforceFileTabLimit(state State, limit int) (State, []Tab) {
	var files []Tab
	for _, tab := range state.Tabs {
		if tab.Type == TabFile {
			files = append(files, tab)
		}
	}
	if len(files) <= limit {
		return state, nil
	}
	overflow := len(files) - limit
	sorted := slices.Clone(files)
	slices.SortStableFunc(sorted, func(a, b Tab) int {
		return cmp.Compare(a.lastActive(), b.lastActive())
	})
	evicted := sorted[:overflow]
	ids := map[string]bool{}
	for _, tab := range evicted {
		ids[tab.ID] = true
	}
	next, _ := removeTabsByIDs(state, ids)
	return next, evicted
}

// lastActive 返回淘汰排序用的激活时间；未激活过回退打开时间。
func (t Tab) lastActive() int64 {
	if t.ActivatedAt != 0 {
		return t.ActivatedAt
	}
	return t.OpenedAt
}
