// 本文件是侧栏的服务接线（Client）：把 ThreadService / WorkspaceService /
// SettingsService / AuditService 的调用编排在后台 goroutine 里完成，结果
// 经 deliver（win.Update）回主线程写进 Sidebar 状态。
//
// 线程约定：Sidebar 状态只在主线程（view 或 deliver 回调）读写；动作
// 入口在主线程先快照模式等需要的字段，再进 goroutine。
//
// 失败约定（AGENTS.md 6.6）：读取失败写日志并保留旧列表继续可用
//（threads.refresh 的语义）；改动失败用 toast 提示。
//
// TODO(07): 删除确认与占位入口的浮层统一换成应用内浮层体系（对照
// React 版 askConfirm / openPalette / openSettings / openData）。

package sidebar

import (
	"fmt"
	"log/slog"

	mygoapp "github.com/egoist/mygo"

	"github.com/yangshifu1024/craftwork/internal/service"
)

// Deliver 把一个函数调度到主线程执行（app.go 注入 win.Update 的包装）。
type Deliver func(fn func())

// Toast 在主线程弹提示的入口（view 的 c.Toast 只能在 view 帧内用，
// 异步回调完成后经它补一条提示）。
type Toast func(message string)

// ConfirmFn 弹危险确认框，结果经 onDone 回主线程。生产装配用
// goroutine + mygo.Dialog.Message 实现。
type ConfirmFn func(title, message, detail string, onDone func(ok bool))

// Client 是侧栏的服务接线：持有服务引用与投递通道，实现 Actions 里的
// 各个动作。经 internal/app 装配注入。
type Client struct {
	// sb 是侧栏界面状态（主线程读写）。
	sb *Sidebar
	// threads 是会话服务（列表 / 新建 / 重命名 / 删除）。
	threads *service.ThreadService
	// workspaces 是工作区服务（清单与激活）。
	workspaces *service.WorkspaceService
	// settings 是设置服务（模式与活动工作区持久化）。
	settings *service.SettingsService
	// audit 是审计服务（今日用量与条数计数）。
	audit *service.AuditService
	// deliver 把状态写入调度回主线程。
	deliver Deliver
	// toast 在主线程弹提示（异步动作完成后的用户反馈）。
	toast Toast
	// confirmDelete 弹危险确认框；为空时删除不做确认（仅 Tester 路径）。
	confirmDelete ConfirmFn
}

// NewClient 创建侧栏服务接线。
func NewClient(
	sb *Sidebar,
	threads *service.ThreadService,
	workspaces *service.WorkspaceService,
	settings *service.SettingsService,
	audit *service.AuditService,
	deliver Deliver,
	toast Toast,
	confirmDelete ConfirmFn,
) *Client {
	return &Client{
		sb:            sb,
		threads:       threads,
		workspaces:    workspaces,
		settings:      settings,
		audit:         audit,
		deliver:       deliver,
		toast:         toast,
		confirmDelete: confirmDelete,
	}
}

// Actions 返回注入侧栏状态的动作集合；app.go 在装配时调用：
//
//	sb.SetActions(client.Actions())
func (cl *Client) Actions() *Actions {
	return &Actions{
		SwitchMode:       cl.SwitchMode,
		CreateTempThread: cl.CreateTempThread,
		CreateThreadIn:   cl.CreateThreadIn,
		CopyThreadID:     cl.CopyThreadID,
		SelectThread:     func(string) {}, // 快照加载归 05 号票，这里只留接线位
		ShowRename:       cl.sb.OpenRename,
		RenameThread:     cl.RenameThread,
		DeleteThread:     cl.DeleteThread,
		Usage:            func() { cl.toast("用量视图" + stubNote) },
		Audit:            func() { cl.toast("审计视图" + stubNote) },
		Palette:          func() { cl.toast("命令面板" + stubNote) },
		Settings:         func() { cl.toast("设置" + stubNote) },
		Boot:             cl.Boot,
	}
}

// Boot 启动首拉：恢复上次模式、工作区清单、当前模式列表与底部计数。
// 在窗口创建后由 app.go 调用一次。
func (cl *Client) Boot() {
	go func() {
		mode := ModeCode
		var activeWorkspace string
		if cl.settings != nil {
			settings := cl.settings.Get()
			mode = NormalizeMode(settings.Mode)
			activeWorkspace = settings.ActiveWorkspace
		}
		workspaces, wsErr := cl.listWorkspaces()
		threads, thErr := cl.listThreads(mode)
		counts, cntErr := cl.loadCounts()
		cl.deliver(func() {
			cl.sb.SetMode(mode)
			cl.sb.SetActiveWorkspace(activeWorkspace)
			if wsErr != nil {
				slog.Warn("读取工作区清单失败", "err", wsErr)
			} else {
				cl.sb.ReplaceWorkspaces(workspaces)
			}
			if thErr != nil {
				slog.Warn("读取会话列表失败", "err", thErr)
			} else {
				cl.sb.ReplaceThreads(mode, threads)
			}
			if cntErr == nil {
				cl.sb.SetCounts(counts)
			}
		})
	}()
}

// SwitchMode 切换模式（主线程入口）：先写设置，再拉新列表与计数。
func (cl *Client) SwitchMode(mode Mode) {
	go func() {
		if cl.settings != nil {
			settings := cl.settings.Get()
			settings.Mode = string(mode)
			if _, err := cl.settings.Save(settings); err != nil {
				slog.Warn("保存模式设置失败", "err", err)
			}
		}
		threads, err := cl.listThreads(mode)
		counts, cntErr := cl.loadCounts()
		cl.deliver(func() {
			cl.sb.SetMode(mode)
			if err != nil {
				slog.Warn("读取会话列表失败", "err", err)
			} else {
				cl.sb.ReplaceThreads(mode, threads)
			}
			if cntErr == nil {
				cl.sb.SetCounts(counts)
			}
		})
	}()
}

// CreateTempThread 新建临时会话（主线程入口）：切到内置临时目录
// （如不在）再新建。对齐 workspaceFlow.createTempThread：目录缺失时
// 提示并保持现状。
func (cl *Client) CreateTempThread() {
	mode := cl.sb.Mode
	go func() {
		conversation, err := cl.conversationWorkspace()
		if err != nil {
			cl.deliver(func() {
				cl.toast("临时会话不可用：内置临时目录尚未初始化，请重启应用后重试")
			})
			return
		}
		cl.createAsync(mode, conversation, "")
	}()
}

// CreateThreadIn 在指定项目下新建会话（主线程入口）：先激活该项目再
// 新建（React 版 handleCreateThreadIn：新会话始终绑定切换后的活动工作区）。
func (cl *Client) CreateThreadIn(workspaceID string) {
	cl.createAsync(cl.sb.Mode, workspaceID, "")
}

// CopyThreadID 把会话 Id 写入剪贴板并提示（sessionActions.copySessionID）。
// mygo 剪贴板要经主循环（onMain），view 帧内直调会死锁：这里经
// deliver（win.Update）排到本帧之后执行，提示走排队 toast。
func (cl *Client) CopyThreadID(id string) {
	cl.deliver(func() {
		// 已在主线程帧回调里（win.Update 执行点），剪贴板可直写。
		mygoClipboard(id)
		cl.sb.QueueToast("已复制会话 Id " + id)
	})
}

// createAsync 是新建会话的公共编排：激活工作区 → Create → 刷新列表并
// 选中新会话。mode 由主线程入口快照传入，goroutine 里不再读状态。
func (cl *Client) createAsync(mode Mode, workspaceID, title string) {
	go func() {
		if cl.workspaces != nil && workspaceID != "" {
			if _, err := cl.workspaces.SetActive(workspaceID); err != nil {
				slog.Warn("切换工作区失败", "err", err)
			}
		}
		thread, err := cl.threads.Create(string(mode), workspaceID, title)
		if err != nil {
			cl.deliver(func() { cl.toast("新建会话失败：" + err.Error()) })
			return
		}
		cl.refreshAsync(mode, func() {
			cl.sb.Select(thread.ID)
		})
	}()
}

// RenameThread 修改会话标题（主线程入口），成功后刷新列表
// （标题以服务端结果为准）。
func (cl *Client) RenameThread(id, title string) {
	mode := cl.sb.Mode
	go func() {
		if err := cl.threads.Rename(id, title); err != nil {
			cl.deliver(func() { cl.toast("重命名失败：" + err.Error()) })
			return
		}
		cl.refreshAsync(mode, nil)
	}()
}

// DeleteThread 弹危险确认（原生 Dialog.Message，07 号票换应用内浮层），
// 确认后彻底删除并刷新列表与计数。
func (cl *Client) DeleteThread(id string) {
	if cl.confirmDelete == nil {
		cl.removeAsync(cl.sb.Mode, id)
		return
	}
	// 主线程读会话标题作为确认框详情。
	title := id
	if t, ok := cl.sb.ThreadByID(id); ok && t.Title != "" {
		title = t.Title
	}
	mode := cl.sb.Mode
	cl.confirmDelete(
		"彻底删除会话？",
		"会话的消息、事件与审批记录都会被删除，无法恢复。",
		title,
		func(ok bool) {
			if ok {
				cl.removeAsync(mode, id)
			}
		},
	)
}

// removeAsync 执行删除并刷新；删除当前会话时选中列表第一条
// （threads.remove 的语义），并补一条提示。
func (cl *Client) removeAsync(mode Mode, id string) {
	go func() {
		if err := cl.threads.Delete(id); err != nil {
			cl.deliver(func() { cl.toast("删除失败：" + err.Error()) })
			return
		}
		cl.refreshAsync(mode, func() {
			cl.toast("会话已删除")
		})
	}()
}

// refreshAsync 重新拉取指定模式列表与计数，回主线程写状态后执行 after。
// 删除当前会话后自动选中列表第一条；列表为空则清空选中。
func (cl *Client) refreshAsync(mode Mode, after func()) {
	threads, err := cl.listThreads(mode)
	counts, cntErr := cl.loadCounts()
	cl.deliver(func() {
		if err != nil {
			slog.Warn("读取会话列表失败", "err", err)
		} else {
			cl.sb.ReplaceThreads(mode, threads)
			if active := cl.sb.ActiveID(); active != "" && !cl.sb.HasThread(active) {
				if len(threads) > 0 {
					cl.sb.Select(threads[0].ID)
				} else {
					cl.sb.Select("")
				}
			}
		}
		if cntErr == nil {
			cl.sb.SetCounts(counts)
		}
		if after != nil {
			after()
		}
	})
}

// listThreads 拉一个模式的会话列表并转换为侧栏 Thread。
func (cl *Client) listThreads(mode Mode) ([]Thread, error) {
	views, err := cl.threads.List(string(mode), false)
	if err != nil {
		return nil, err
	}
	out := make([]Thread, 0, len(views))
	for _, v := range views {
		out = append(out, Thread{
			ID:          v.ID,
			Title:       v.Title,
			WorkspaceID: v.WorkspaceID,
			Status:      v.Status,
			UpdatedAt:   v.UpdatedAt,
		})
	}
	return out, nil
}

// listWorkspaces 拉工作区清单并转换为侧栏 Workspace。
func (cl *Client) listWorkspaces() ([]Workspace, error) {
	views, err := cl.workspaces.List()
	if err != nil {
		return nil, err
	}
	out := make([]Workspace, 0, len(views))
	for _, v := range views {
		out = append(out, Workspace{ID: v.ID, Name: v.Name, Purpose: v.Purpose})
	}
	return out, nil
}

// conversationWorkspace 找内置临时会话目录的 Id；缺失是初始化失败的
// 极端情况，返回可读错误。
func (cl *Client) conversationWorkspace() (string, error) {
	views, err := cl.workspaces.List()
	if err != nil {
		return "", err
	}
	for _, v := range views {
		if v.Purpose == "conversation" {
			return v.ID, nil
		}
	}
	return "", fmt.Errorf("内置临时目录不存在")
}

// loadCounts 拉底部计数：今日输入 + 输出 token 与最近 200 条审计数。
func (cl *Client) loadCounts() (Counts, error) {
	counts := Counts{}
	usage, err := cl.audit.Usage()
	if err != nil {
		return counts, err
	}
	counts.UsageTokens = usage.Today.InputTokens + usage.Today.OutputTokens
	entries, err := cl.audit.List(200)
	if err != nil {
		return counts, err
	}
	counts.AuditItems = len(entries)
	return counts, nil
}

// mygoClipboard 写剪贴板文本（仅主线程调用点使用）。
func mygoClipboard(text string) { mygoapp.Clipboard.WriteText(text) }
