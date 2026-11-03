// Package overlays 是 CraftWork 的浮层体系：通用浮层基建（确认 / 输入 /
// 自由档位确认三个全局对话框、通知）与设置中心、数据面板、命令面板、
// 创建项目对话框（07 号票）。
//
// 调用模型对齐 React 版的 Promise 语义，只是落点从 await 换成回调：view
// 代码里 `overlays.Confirm(c, opts, func(ok bool) { ... })` 发起请求后立即
// 返回，用户在浮层里做出选择时回调在主线程执行。请求由本包单例持有，
// 同类请求同时最多一个，后发起的覆盖前一个（被覆盖的回调按取消收尾）。
//
// 线程约定与外壳一致：浮层状态只在主线程（view 或 win.Update 回调）读写；
// 服务调用若可能慢（网络、MCP 连接），用 runAsync 在后台 goroutine 执行，
// 结果经 dispatch 投回主线程后再写状态。
//
// 本包不做业务逻辑：设置读写、提供商管理、审计查询都直调 internal/service
// 的方法（02 号票后服务是普通领域包），校验与错误转换以服务为准。
package overlays

import (
	mygoapp "github.com/egoist/mygo"
	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/service"
)

// Backend 是浮层需要的后端门面：view 直调服务方法，不在此再包一层。
// 装配（internal/app）构造后经 Bind 注入；字段非空的职责由装配保证。
type Backend struct {
	Settings  *service.SettingsService
	Providers *service.ProviderService
	Audit     *service.AuditService
	MCP       *service.MCPService
	Workspace *service.WorkspaceService
	Threads   *service.ThreadService
	App       *service.AppService
}

// Hooks 是浮层触达其他视图状态的回调槽：命令面板与设置入口要切换外壳、
// 侧栏、面板里的状态，那些状态归各票所有。未接线的钩子保持 nil，对应的
// 命令行置灰展示（不做「看起来能点但无效果」的控件）；04 / 05 / 08 号票
// 合入后在装配处补齐。
type Hooks struct {
	// NewThread 新建会话（05 号票接线）。
	NewThread func()
	// OpenThread 切换到指定会话（04 / 05 号票接线）。
	OpenThread func(threadID string)
	// NewTempThread 新建临时会话（04 号票接线）。
	NewTempThread func()
	// SetMode 切换 Code / Work 模式（04 号票接线），参数 "code" / "work"。
	SetMode func(mode string)
	// TogglePanel 展开 / 收起右侧面板（外壳状态，装配时接线）。
	TogglePanel func()
	// PanelOpen 报告面板当前是否展开（命令标题用动作方向）；未接线时视为展开。
	PanelOpen func() bool
	// MaximizePanel 全屏 / 还原右侧面板（08 号票接线）。
	MaximizePanel func()
	// ClosePanelTab 关闭面板当前标签页（08 号票接线）。
	ClosePanelTab func()
	// OpenFile 在面板打开文件并定位行（08 号票接线，OpenFileAt）。
	OpenFile func(path string, line int)
	// ProjectCreated 创建项目成功后刷新侧栏并新建会话（04 号票接线）。
	ProjectCreated func(workspaceID string)
}

// Store 持有全部浮层的状态。立即模式约定：只在主线程读写，不放锁；
// 后台结果经 dispatch 投回主线程后再写。
type Store struct {
	be    *Backend
	hooks Hooks
	win   *mygoapp.Window

	// toastQueue 是待展示的通知：Notify 排队，通知层在下一帧消费。
	toastQueue []queuedToast
	// logos 是目录提供商 logo 的解码缓存（catalogID → 位图 / 失败标记）。
	logos map[string]logoEntry

	confirm  confirmState
	prompt   promptState
	free     freeState
	composer composerState

	settings settingsState
	data     dataState
	palette  paletteState
	project  projectState
}

// store 是浮层的进程级单例：浮层入口是包级 API（供任何视图包调用），
// 状态收敛在一个实例里，装配经 Bind 注入服务与钩子。
var store = &Store{}

// Bind 注入后端门面；必须在窗口首帧前调用（装配阶段）。
func Bind(b *Backend) { store.be = b }

// BindHooks 注入跨视图回调；04 / 05 / 08 号票合入后补齐缺省的 nil 槽。
func BindHooks(h Hooks) { store.hooks = h }

// BindWindow 记录主窗口，供后台结果投回主线程；窗口创建后调用。
func BindWindow(win *mygoapp.Window) { store.win = win }

// View 画全部浮层。根 view 在外壳之后调用：模态（设置 / 数据 / 创建项目 /
// 命令面板 / 输入区三浮层）压在外壳上，全局对话框压在模态上，通知在最上层。
func View(c *mygo.Context) {
	store.drawSettings(c)
	store.drawData(c)
	store.drawProject(c)
	store.drawPalette(c)
	store.drawComposer(c)
	store.drawConfirm(c)
	store.drawPrompt(c)
	store.drawFree(c)
	drawToastViewport(c)
}

// runAsync 在后台 goroutine 执行 work；work 返回的收尾回调投递到主线程
// 执行（改浮层状态并触发下一帧）。窗口未绑定（单测或装配早期）时收尾
// 被丢弃，测试只走同步路径。
func (s *Store) runAsync(work func() func()) {
	go func() {
		done := work()
		s.dispatch(done)
	}()
}

// dispatch 把收尾回调投递到主线程。
func (s *Store) dispatch(done func()) {
	if s.win == nil {
		return
	}
	s.win.Update(func() { done() })
}

// ---------------------------------------------------------------------------
// 打开 / 关闭入口（包级 API）
// ---------------------------------------------------------------------------

// SettingsTab 是设置中心的分区标识，对齐 React 版 SettingsTab。
type SettingsTab string

// 设置中心的七个分组。
const (
	TabGeneral    SettingsTab = "general"
	TabMode       SettingsTab = "mode"
	TabModels     SettingsTab = "models"
	TabTools      SettingsTab = "tools"
	TabMCP        SettingsTab = "mcp"
	TabAppearance SettingsTab = "appearance"
	TabAdvanced   SettingsTab = "advanced"
)

// OpenSettings 打开设置中心；tab 非空时跳到该分组，空串保持上次停留分组。
func OpenSettings(tab SettingsTab) { store.openSettings(tab) }

// CloseSettings 关闭设置中心。
func CloseSettings() { store.settings.open = false }

// DataTab 是数据面板的分区标识：审计、用量、备份。
type DataTab string

// 数据面板的三个分区。
const (
	TabAudit  DataTab = "audit"
	TabUsage  DataTab = "usage"
	TabBackup DataTab = "backup"
)

// OpenData 打开数据面板；tab 为空时回到审计分区（对齐 React openData）。
func OpenData(tab DataTab) { store.openData(tab) }

// CloseData 关闭数据面板。
func CloseData() { store.data.open = false }

// TogglePalette 打开 / 关闭命令面板（⌘K）。
func TogglePalette() { store.togglePalette() }

// ClosePalette 关闭命令面板。
func ClosePalette() { store.palette.open = false }

// OpenCreateProject 打开「创建项目」对话框。
func OpenCreateProject() { store.project.open = true }

// CloseCreateProject 关闭「创建项目」对话框。
func CloseCreateProject() { store.project.open = false }

// ---------------------------------------------------------------------------
// 设置变更的立即生效
// ---------------------------------------------------------------------------

// settingsSnapshot 返回当前设置；服务未注入时回退默认值（纯 UI 预览安全）。
func (s *Store) settingsSnapshot() service.Settings {
	if s.be == nil || s.be.Settings == nil {
		return service.DefaultSettings()
	}
	return s.be.Settings.Get()
}
