// Package sidebar 是原生 UI 的左侧栏（268 固定列）：Code / Work 模式切换、
// 会话分组列表与底部入口两行。
//
// 本文件是列表状态的纯函数状态机，语义移植 frontend/src/stores/threads.ts 的
// 列表部分与 frontend/src/features/sidebar/Sidebar.tsx 的分组规则：
//
//   - 会话按归属分组：内置临时目录（purpose = conversation）的会话置顶成
//     「临时会话」组；其余按项目工作区（最近打开优先）分组；工作区已被删除
//     的会话归入末尾「其他」组，避免数据凭空消失。
//   - 组内分页：每组默认展示最近 PageSize 条，「显示更多」每次追加同数条；
//     展开条数是分组维度的记忆，折叠再展开不重置（React 版组件本地 state）。
//   - 折叠持久化：临时组与各项目组各自记忆折叠与否；React 版写 localStorage，
//     原生版本期保存在内存（结构即持久化位，后续票接设置存储即可延续）。
//
// 所有函数不触碰 mygo 与服务层，便于单测覆盖状态转移。
package sidebar

import (
	"sort"
	"strconv"
	"time"
)

// PageSize 是每个会话分组默认展示的条数，也是「显示更多」每次追加的条数
// （Sidebar.tsx 的 PROJECT_GROUP_PAGE_SIZE）。
const PageSize = 5

// Mode 是对话模式（settings.Mode）。
type Mode string

// 两种模式；空值与未知值一律按 Code 处理（NormalizeMode）。
const (
	ModeCode Mode = "code"
	ModeWork Mode = "work"
)

// NormalizeMode 把后端模式字符串收敛为已知模式；React 版以 code 为兜底。
func NormalizeMode(s string) Mode {
	if Mode(s) == ModeWork {
		return ModeWork
	}
	return ModeCode
}

// Thread 是侧栏列表需要的会话字段（service.ThreadView 的列表子集）。
type Thread struct {
	ID          string
	Title       string
	WorkspaceID string
	// Status 是 idle / running / awaiting / error（store.Thread.Status）。
	Status    string
	UpdatedAt int64 // 毫秒时间戳
}

// Workspace 是分组需要的工作区字段（service.WorkspaceView 的列表子集）。
type Workspace struct {
	ID   string
	Name string
	// Purpose 是 project 或 conversation（内置临时会话目录）。
	Purpose string
}

// GroupKind 区分三种分组。
type GroupKind int

const (
	// GroupTemp 是置顶的「临时会话」组（绑定内置临时目录的会话）。
	GroupTemp GroupKind = iota
	// GroupProject 是一个项目工作区的会话组。
	GroupProject
	// GroupOrphan 是「其他」组：所属工作区已从列表移除的会话。
	GroupOrphan
)

// Group 是分组后的一个组：组头信息与其会话（后端顺序，最近更新优先）。
type Group struct {
	Kind GroupKind
	// Key 是折叠与分页状态的键：临时组固定为 "temp"，项目组为工作区 Id，
	// 孤儿组固定为 "orphan"。
	Key string
	// Name 是组头展示名；项目组为工作区名，孤儿组为「其他」。
	Name string
	// WorkspaceID 是项目组的工作区 Id；其余组为空。
	WorkspaceID string
	Items       []Thread
}

// tempGroupKey / orphanGroupKey 是非项目组在折叠与分页表里的固定键。
const (
	tempGroupKey   = "temp"
	orphanGroupKey = "orphan"
)

// GroupThreads 把一个模式的会话列表按工作区归属分组。
//
// 顺序规则对齐 Sidebar.tsx：临时组置顶；项目组按工作区切片顺序（最近打开
// 优先，由服务层保证）；孤儿组垫底。空组的项目也保留组头（React 版行为：
// 项目下还没有会话时组头仍然可见，+ 号可新建）。
func GroupThreads(threads []Thread, workspaces []Workspace) []Group {
	conversation := make(map[string]bool, len(workspaces))
	projectOrder := make([]Workspace, 0, len(workspaces))
	for _, w := range workspaces {
		if w.Purpose == "conversation" {
			conversation[w.ID] = true
			continue
		}
		projectOrder = append(projectOrder, w)
	}

	byWorkspace := make(map[string][]Thread, len(workspaces))
	var orphans []Thread
	for _, t := range threads {
		switch {
		case conversation[t.WorkspaceID]:
			byWorkspace[tempGroupKey] = append(byWorkspace[tempGroupKey], t)
		case hasWorkspace(projectOrder, t.WorkspaceID):
			byWorkspace[t.WorkspaceID] = append(byWorkspace[t.WorkspaceID], t)
		default:
			// 绑定了已删除工作区的会话：仍有归属组，不凭空消失。
			orphans = append(orphans, t)
		}
	}

	groups := make([]Group, 0, len(projectOrder)+2)
	groups = append(groups, Group{
		Kind:  GroupTemp,
		Key:   tempGroupKey,
		Name:  "临时会话",
		Items: byWorkspace[tempGroupKey],
	})
	for _, w := range projectOrder {
		groups = append(groups, Group{
			Kind:        GroupProject,
			Key:         w.ID,
			Name:        w.Name,
			WorkspaceID: w.ID,
			Items:       byWorkspace[w.ID],
		})
	}
	if len(orphans) > 0 {
		groups = append(groups, Group{
			Kind:  GroupOrphan,
			Key:   orphanGroupKey,
			Name:  "其他",
			Items: orphans,
		})
	}
	return groups
}

// hasWorkspace 报告 id 是否在工作区切片中（项目分组归置用）。
func hasWorkspace(workspaces []Workspace, id string) bool {
	for _, w := range workspaces {
		if w.ID == id {
			return true
		}
	}
	return false
}

// ProjectThreadCount 汇总项目分组的会话总数，是「项目会话」区头数量徽章
// 的值（React 版只数项目组；孤儿组数量在「其他」组头单独展示）。
func ProjectThreadCount(groups []Group) int {
	sum := 0
	for _, g := range groups {
		if g.Kind == GroupProject {
			sum += len(g.Items)
		}
	}
	return sum
}

// VisibleItems 按已展开条数截取组内可见会话，并给出是否还有剩余。
// limit <= 0 视为初始页；超过长度的 limit 按全长处理。
func VisibleItems(items []Thread, limit int) (visible []Thread, hasMore bool) {
	if limit <= 0 {
		limit = PageSize
	}
	if limit > len(items) {
		limit = len(items)
	}
	return items[:limit], len(items) > limit
}

// NextLimit 把分页游标追加一页。未登记的组按首页 PageSize 起算
// （React 版初始 limit = PageSize，首次「显示更多」到 2×PageSize）；
// 分页计数是分组维度的记忆，折叠再展开与切换其他分组都不重置。
func NextLimit(current int) int {
	if current < PageSize {
		current = PageSize
	}
	return current + PageSize
}

// Collapses 记录各分组的折叠状态与分页游标（React 版的 collapsedProjects /
// collapsedTemp / 组内 limit）。原生版本期保存在内存；字段即持久化位，
// 后续接设置存储时整表落盘即可。
type Collapses struct {
	// Collapsed 是折叠表：键为分组 Key，true 表示折叠；缺省为展开。
	Collapsed map[string]bool
	// Limits 是分页游标：键为分组 Key，值是该组已展开的条数；缺省为首页。
	Limits map[string]int
}

// NewCollapses 返回全展开、全部首页的初始折叠状态。
func NewCollapses() Collapses {
	return Collapses{
		Collapsed: make(map[string]bool),
		Limits:    make(map[string]int),
	}
}

// IsCollapsed 报告一个分组是否折叠；未登记的分组按展开处理。
func (c Collapses) IsCollapsed(key string) bool {
	return c.Collapsed[key]
}

// Toggle 翻转一个分组的折叠状态，返回新表（纯函数，不改原表）。
func (c Collapses) Toggle(key string) map[string]bool {
	next := cloneCollapsed(c.Collapsed)
	next[key] = !next[key]
	return next
}

// AllProjectsCollapsed 报告「项目会话」区是否所有项目组都已折叠：
// React 版语义——存在项目组且每一组都折叠才算全部折叠。
func (c Collapses) AllProjectsCollapsed(groups []Group) bool {
	found := false
	for _, g := range groups {
		if g.Kind != GroupProject {
			continue
		}
		if !c.IsCollapsed(g.Key) {
			return false
		}
		found = true
	}
	return found
}

// ToggleAllProjects 批量折叠 / 展开所有项目组，返回新表：已全部折叠时
// 清空折叠表（全展开），否则逐个置 true。
func (c Collapses) ToggleAllProjects(groups []Group) map[string]bool {
	next := cloneCollapsed(c.Collapsed)
	if c.AllProjectsCollapsed(groups) {
		for key := range next {
			if isProjectKey(groups, key) {
				delete(next, key)
			}
		}
		return next
	}
	for _, g := range groups {
		if g.Kind == GroupProject {
			next[g.Key] = true
		}
	}
	return next
}

// cloneCollapsed 复制折叠表，nil 视为空表。
func cloneCollapsed(m map[string]bool) map[string]bool {
	next := make(map[string]bool, len(m)+1)
	for k, v := range m {
		next[k] = v
	}
	return next
}

// isProjectKey 报告键是否属于某个项目组（批量展开只清项目组的登记）。
func isProjectKey(groups []Group, key string) bool {
	for _, g := range groups {
		if g.Kind == GroupProject && g.Key == key {
			return true
		}
	}
	return false
}

// Status 是会话条目的展示状态（线程行右侧状态点）。
type Status int

const (
	// StatusIdle 是已完成（其余状态之外的一律灰点无脉冲）。
	StatusIdle Status = iota
	// StatusRunning 是运行中（模式强调色脉冲点）。
	StatusRunning
	// StatusAwaiting 是等待审批（黄点）。
	StatusAwaiting
	// StatusError 是错误（红点）。
	StatusError
)

// StatusOf 把后端状态字符串映射为展示状态；未知值按已完成处理。
func StatusOf(status string) Status {
	switch status {
	case "running":
		return StatusRunning
	case "awaiting":
		return StatusAwaiting
	case "error":
		return StatusError
	default:
		return StatusIdle
	}
}

// FormatTime 把毫秒时间戳格式化为列表时间列（utils.formatTime 的移植）：
// 当天显示 HH:MM，否则 MM/DD HH:MM；零值显示空。时区与夏令时交给
// 标准库 time 的本地时区处理。
func FormatTime(millis int64, now int64) string {
	if millis <= 0 {
		return ""
	}
	t := time.UnixMilli(millis)
	n := time.UnixMilli(now)
	clock := t.Format("15:04")
	if t.Year() == n.Year() && t.YearDay() == n.YearDay() && t.Location() == n.Location() {
		return clock
	}
	return t.Format("01/02") + " " + clock
}

// FormatTokens 把 token 数格式化为紧凑展示（utils.formatTokens 的移植）：
// 1000 以下原样；10000 以下一位小数的 k；更大四舍五入到整数 k。
func FormatTokens(v int64) string {
	switch {
	case v <= 0:
		return "0"
	case v < 1000:
		return strconv.FormatInt(v, 10)
	case v < 10000:
		return strconv.FormatInt(v/1000, 10) + "." +
			strconv.FormatInt((v%1000)/100, 10) + "k"
	default:
		return strconv.FormatInt((v+500)/1000, 10) + "k"
	}
}

// SortByUpdated 把会话按更新时间降序排序（后端 List 已保证顺序，
// 这里兜底给测试与非后端来源用，不改变稳定性语义）。
func SortByUpdated(threads []Thread) {
	sort.SliceStable(threads, func(i, j int) bool {
		return threads[i].UpdatedAt > threads[j].UpdatedAt
	})
}
