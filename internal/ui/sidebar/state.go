// 本文件是侧栏的界面状态：模式、双模式会话列表、工作区分组源、折叠与
// 分页记忆、底部计数与临时浮层。立即模式约定：只在主线程（view 或
// win.Update 回调）里读写，不放锁。
//
// 服务触达的动作集中在 Actions 函数组，由 internal/app 装配注入；
// 字段为 nil 时（ui.Tester 单测路径）按钮只改本地状态。

// 状态机本体在 model.go（纯函数）；这里是随窗口存活的可变状态壳。

package sidebar

// Counts 是底部两行的计数：今日 token 用量与最近审计条数
// （React 版随模式或活动会话变化重新拉取）。
type Counts struct {
	UsageTokens int64
	AuditItems  int
}

// RenameDialog 是重命名浮层的临时状态；Thread 为空表示没有浮层。
type RenameDialog struct {
	Open  bool
	ID    string
	Title string // 输入框内容
}

// Sidebar 是左侧栏的界面状态壳。
type Sidebar struct {
	// Mode 是当前对话模式；启动时从设置恢复。
	Mode Mode
	// threads 是两种模式各自的会话列表（后端顺序）。
	threads map[Mode][]Thread
	// workspaces 是工作区清单（最近打开优先），分组依据。
	workspaces []Workspace
	// activeWorkspace 是设置里恢复的活动工作区 Id（会话新建的归属；
	// 工作区视图归 08 号票，这里只留位）。
	activeWorkspace string
	// active 是两种模式各自选中的会话 Id，切换模式互不影响。
	active map[Mode]string
	// coll 是折叠与分页记忆（跨模式共享，对齐 React 版组件级 state）。
	coll Collapses
	// counts 是底部计数。
	counts Counts
	// rename 是重命名浮层状态。
	rename RenameDialog
	// pendingToast 是待弹的提示：异步动作完成后经 deliver 写入，
	// view 下一帧开头消费（mygo 的 c.Toast 只能在 view 帧内调用）。
	pendingToast string
	// actions 是服务动作入口；NewSidebar 时为 nil，经 SetActions 注入。
	actions *Actions
}

// NewSidebar 创建侧栏状态：Code 模式、空列表、全展开。
func NewSidebar() *Sidebar {
	return &Sidebar{
		Mode:    ModeCode,
		threads: map[Mode][]Thread{ModeCode: {}, ModeWork: {}},
		active:  map[Mode]string{ModeCode: "", ModeWork: ""},
		coll:    NewCollapses(),
	}
}

// SetActions 注入服务动作集合（app.go 装配时调用一次；nil 合法，
// view 全部回退本地行为）。
func (s *Sidebar) SetActions(a *Actions) { s.actions = a }

// Actions 是侧栏触达服务层的动作集合（internal/app 注入；view 对 nil
// 字段回退为本地状态或占位 toast，供单测与接线前使用）。
type Actions struct {
	// SwitchMode 切换模式并持久化到设置，随后刷新该模式列表与计数。
	SwitchMode func(Mode)
	// CreateTempThread 在内置临时目录新建会话并选中。
	CreateTempThread func()
	// CreateThreadIn 先切换到指定项目工作区，再新建绑定它的会话。
	CreateThreadIn func(workspaceID string)
	// CreateProject 打开「创建项目」对话框（07 号票前的占位入口）。
	CreateProject func()
	// CopyThreadID 把会话 Id 写入剪贴板并提示。剪贴板要经主循环，
	// 实现必须排队到帧后执行（app.go 用 win.Update），不能帧内直调。
	CopyThreadID func(id string)
	// SelectThread 选中会话后的联动（快照加载归 05 号票；本票为空实现）。
	SelectThread func(id string)
	// ShowRename 打开重命名浮层（先在本地状态登记目标会话）。
	ShowRename func(t Thread)
	// RenameThread 修改会话标题并刷新列表。
	RenameThread func(id, title string)
	// DeleteThread 弹危险确认后彻底删除会话并刷新列表。
	DeleteThread func(id string)
	// Usage / Audit / Palette / Settings 是底部两行与区头的入口动作
	//（数据浮层与命令面板归 07 号票；nil 时 view 弹占位 toast）。
	Usage    func()
	Audit    func()
	Palette  func()
	Settings func()
	// Boot 启动首拉：恢复模式、工作区、列表与计数（app.go 在窗口
	// 创建后调用一次）。
	Boot func()
}

// ---------------------------------------------------------------------------
// 只读派生（view 每帧调用）
// ---------------------------------------------------------------------------

// Groups 返回当前模式的分组结果（临时组置顶、项目组随后、孤儿垫底）。
func (s *Sidebar) Groups() []Group {
	return GroupThreads(s.threads[s.Mode], s.workspaces)
}

// ActiveID 返回当前模式选中的会话 Id。
func (s *Sidebar) ActiveID() string { return s.active[s.Mode] }

// ActiveInMode 返回指定模式记忆的选中会话 Id（05 号票聊天列恢复选中用）。
func (s *Sidebar) ActiveInMode(m Mode) string { return s.active[NormalizeMode(string(m))] }

// Collapses 返回折叠与分页记忆（view 读取）。
func (s *Sidebar) Collapses() Collapses { return s.coll }

// Counts 返回底部计数。
func (s *Sidebar) Counts() Counts { return s.counts }

// Rename 返回重命名浮层状态。
func (s *Sidebar) Rename() RenameDialog { return s.rename }

// ThreadByID 在当前模式列表里找会话（右键菜单动作的定位用）。
func (s *Sidebar) ThreadByID(id string) (Thread, bool) {
	for _, t := range s.threads[s.Mode] {
		if t.ID == id {
			return t, true
		}
	}
	return Thread{}, false
}

// ---------------------------------------------------------------------------
// 状态转移（view 与 Controller 共用；仅主线程调用）
// ---------------------------------------------------------------------------

// SetMode 写入当前模式（不触达服务层；走设置的切换用 Actions.SwitchMode）。
func (s *Sidebar) SetMode(m Mode) { s.Mode = NormalizeMode(string(m)) }

// ReplaceThreads 整表替换一个模式的会话列表（Client 拉取后投递）。
func (s *Sidebar) ReplaceThreads(m Mode, list []Thread) {
	if s.threads == nil {
		s.threads = map[Mode][]Thread{}
	}
	s.threads[NormalizeMode(string(m))] = list
}

// ReplaceWorkspaces 整表替换工作区清单。
func (s *Sidebar) ReplaceWorkspaces(list []Workspace) { s.workspaces = list }

// Select 选中一个会话（当前模式槽位）。
func (s *Sidebar) Select(id string) { s.active[s.Mode] = id }

// SetActiveWorkspace 记录设置里恢复的活动工作区 Id。
func (s *Sidebar) SetActiveWorkspace(id string) { s.activeWorkspace = id }

// ActiveWorkspace 返回活动工作区 Id。
func (s *Sidebar) ActiveWorkspace() string { return s.activeWorkspace }

// HasThread 报告当前模式列表是否包含指定会话（删除后的选中兜底用）。
func (s *Sidebar) HasThread(id string) bool {
	_, ok := s.ThreadByID(id)
	return ok
}

// SetCounts 写入底部计数。
func (s *Sidebar) SetCounts(c Counts) { s.counts = c }

// QueueToast 排队一条提示，view 下一帧开头弹出（异步动作的反馈通道）。
func (s *Sidebar) QueueToast(message string) { s.pendingToast = message }

// TakeToast 取走排队中的提示；空表示没有待弹的提示。
func (s *Sidebar) TakeToast() string {
	message := s.pendingToast
	s.pendingToast = ""
	return message
}

// ToggleTemp 翻转临时会话组折叠。
func (s *Sidebar) ToggleTemp() { s.coll.Collapsed = s.coll.Toggle(tempGroupKey) }

// ToggleProject 翻转一个项目组折叠。
func (s *Sidebar) ToggleProject(key string) { s.coll.Collapsed = s.coll.Toggle(key) }

// ToggleAllProjects 批量折叠 / 展开所有项目组。
func (s *Sidebar) ToggleAllProjects() { s.coll.Collapsed = s.coll.ToggleAllProjects(s.Groups()) }

// ShowMore 把一个组的分页游标追加一页。
func (s *Sidebar) ShowMore(key string) { s.coll.Limits[key] = NextLimit(s.coll.Limits[key]) }

// Limit 返回一个组的分页游标；未展开过按首页 PageSize 处理
// （React 版 useState 初始值，保证「还有 N 条」从 5 起算）。
func (s *Sidebar) Limit(key string) int {
	if v := s.coll.Limits[key]; v > 0 {
		return v
	}
	return PageSize
}

// OpenRename 打开重命名浮层并预填当前标题。
func (s *Sidebar) OpenRename(t Thread) {
	s.rename = RenameDialog{Open: true, ID: t.ID, Title: t.Title}
}

// CloseRename 关闭重命名浮层。
func (s *Sidebar) CloseRename() { s.rename = RenameDialog{} }
