// client.go 是聊天列的服务接线（对齐 sidebar/client.go 的模式）：把
// ChatService / ThreadService / WorkspaceService / AppService 的调用编排
// 在后台 goroutine 里完成，结果经 deliver（win.Update）回主线程写进
// Chat 状态。
//
// 线程约定：Chat 状态只在主线程（view 或 deliver 回调）读写；动作入口在
// 主线程先快照需要的字段（会话 Id、运行态、模型引用），再进 goroutine。
//
// 事件约定：bus 的 Batch 由 app 的 emitter 桥投递到主线程后进入
// ApplyBatch（reducer 纯函数）；批次要求刷新会话列表时通知宿主（侧栏）。
//
// 失败约定（AGENTS.md 6.6）：读取失败写日志、toast 提示并保留旧状态；
// 改动失败用 toast 提示，发送失败把草稿与附件还回输入框。

package chat

import (
	"encoding/base64"
	"errors"
	"log/slog"
	"strconv"
	"strings"
	"time"

	mygoapp "github.com/egoist/mygo"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// maxPastedAttachmentBytes 是粘贴/拾取图片附件的大小上限（lib/paste.ts
// MAX_PASTED_ATTACHMENT_BYTES：12MB）。文件拾取走 Dialog.Open 不读内容，
// 大小由 Agent 读取附件时施加限制；图片必须先读进内存转存，因此这里拦截。
const maxPastedAttachmentBytes = 12 << 20

// RefreshThreads 通知宿主刷新会话列表（reducer 报告 refreshNeeded 时）。
// app 装配注入侧栏 Client 的刷新；nil 时忽略。
type RefreshThreads func()

// SelectInSidebar 把选中态同步回侧栏列表（自动选中第一条时调用）。
type SelectInSidebar func(id string)

// ActiveInMode 读取侧栏在指定模式记忆的选中会话（跨模式切换时恢复）。
type ActiveInMode func(mode string) string

// Client 是聊天列的服务接线。
type Client struct {
	// chat 是聊天列界面状态（主线程读写）。
	chat *Chat
	// chatSvc 负责发送 / 引导 / 停止 / 重新生成 / 审批决定。
	chatSvc *service.ChatService
	// threads 负责快照与新建会话。
	threads *service.ThreadService
	// workspaces 负责自动新建会话时找归属目录。
	workspaces *service.WorkspaceService
	// deliver 把状态写入调度回主线程。
	deliver func(fn func())
	// refreshThreads 通知宿主刷新会话列表（可空）。
	refreshThreads RefreshThreads
	// selectInSidebar 把选中态同步回侧栏（可空）。
	selectInSidebar SelectInSidebar
	// activeInMode 读取侧栏记忆的模式内选中（可空）。
	activeInMode ActiveInMode
	// openExternal 把外链交给系统浏览器（mygo.Shell.OpenExternal 的
	// goroutine 包装；测试注入记录器）。
	openExternal func(url string) error
	// savePastedImage 把粘贴的图片转存为附件并返回路径（测试注入记录器）。
	savePastedImage func(name string, png []byte) (string, error)
}

// NewClient 创建聊天列服务接线。refreshThreads / selectInSidebar /
// activeInMode 允许为 nil（侧栏未接线或单测路径）。
func NewClient(
	chat *Chat,
	chatSvc *service.ChatService,
	threads *service.ThreadService,
	workspaces *service.WorkspaceService,
	deliver func(fn func()),
	refreshThreads RefreshThreads,
	selectInSidebar SelectInSidebar,
	activeInMode ActiveInMode,
) *Client {
	// 缩略图等后台加载的结果经同一 deliver 通道回主线程。
	chat.SetUpdate(deliver)
	return &Client{
		chat:            chat,
		chatSvc:         chatSvc,
		threads:         threads,
		workspaces:      workspaces,
		deliver:         deliver,
		refreshThreads:  refreshThreads,
		selectInSidebar: selectInSidebar,
		activeInMode:    activeInMode,
		openExternal: func(url string) error {
			// OpenExternal 走 onMain；从 goroutine 调用会排队等待，安全。
			return mygoapp.Shell.OpenExternal(url)
		},
		savePastedImage: func(name string, png []byte) (string, error) {
			return appService.SavePastedAttachment(name, base64.StdEncoding.EncodeToString(png))
		},
	}
}

// appService 是粘贴转存用的应用服务入口（app.go 装配时经 SetAppService 注入；
// 为空时粘贴图片不可用）。构造注入会形成对 service 包的环引用风险，
// 这里用包级变量 + setter 的折中（侧栏模式是构造注入，但 Chat 的动作表
// 比 Client 早一步构建，需要两段装配）。
var appService *service.AppService

// SetAppService 注入应用服务（app.go 装配时调用一次）。
func SetAppService(a *service.AppService) { appService = a }

// Actions 返回注入聊天状态的动作集合；app.go 在装配时调用：
//
//	chat.SetActions(client.Actions())
func (cl *Client) Actions() *Actions {
	return &Actions{
		Submit:          cl.Submit,
		Cancel:          cl.Cancel,
		Regenerate:      cl.Regenerate,
		Decide:          cl.Decide,
		PickFiles:       cl.PickFiles,
		PasteImage:      cl.PasteImage,
		LoadEarlier:     cl.LoadEarlier,
		OpenFile:        cl.OpenFile,
		ConfirmExternal: cl.ConfirmExternal,
		Copy:            cl.Copy,
		OpenDiff:        cl.OpenDiff,
		ReadAttachment:  cl.ReadAttachment,
		SetThreadModel:  cl.SetThreadModel,
	}
}

// SetThreadModel 切换当前会话使用的模型：服务写库并广播快照刷新
// （composer 的模型选择与档位切换同一反馈路径——失败 toast）。
func (cl *Client) SetThreadModel(threadID, modelID string) {
	go func() {
		if err := cl.threads.SetModel(threadID, modelID); err != nil {
			slog.Warn("切换会话模型失败", "err", err)
			cl.deliver(func() { cl.chat.QueueToast("切换模型失败：" + err.Error()) })
			return
		}
		cl.loadSnapshot(threadID)
	}()
}

// Boot 启动首拉：按当前模式恢复选中会话并加载快照。在窗口创建后由
// app.go 调用一次（晚于侧栏 Boot，列表拉取互不依赖）。
func (cl *Client) Boot(mode string) {
	cl.SelectThreadOrFirst(cl.remembered(mode), mode)
}

// remembered 返回侧栏在该模式记忆的选中会话；侧栏未注入时为空。
func (cl *Client) remembered(mode string) string {
	if cl.activeInMode == nil {
		return ""
	}
	return cl.activeInMode(mode)
}

// SelectThreadOrFirst 选中指定会话；为空时选中该模式列表的第一条。
// 侧栏点击与启动 / 切换模式的自动选中都走这里。
func (cl *Client) SelectThreadOrFirst(threadID, mode string) {
	go func() {
		if threadID == "" {
			views, err := cl.threads.List(mode, false)
			if err != nil {
				slog.Warn("读取会话列表失败", "err", err)
				cl.deliver(func() { cl.chat.QueueToast("读取会话列表失败：" + err.Error()) })
				return
			}
			if len(views) == 0 {
				cl.deliver(func() {
					cl.chat.Clear()
					cl.chat.SetActiveThread("")
				})
				return
			}
			threadID = views[0].ID
		}
		cl.loadSnapshot(threadID)
	}()
}

// SelectThread 侧栏点击会话后的联动：加载快照（选中态由侧栏自己写）。
func (cl *Client) SelectThread(threadID string) {
	if threadID == "" {
		cl.deliver(func() {
			cl.chat.Clear()
			cl.chat.SetActiveThread("")
		})
		return
	}
	go cl.loadSnapshot(threadID)
}

// loadSnapshot 拉取会话快照并回主线程接管整份消息。分页接口取最近
// PageSize 条；顺带解析会话绑定目录的用途（空态文案分支）。
func (cl *Client) loadSnapshot(threadID string) {
	snapshot, err := cl.threads.SnapshotPage(threadID, PageSize, 0)
	if err != nil {
		slog.Warn("读取会话快照失败", "err", err)
		cl.deliver(func() { cl.chat.QueueToast("读取会话失败：" + err.Error()) })
		return
	}
	messages := make([]*Message, 0, len(snapshot.Messages))
	for i := range snapshot.Messages {
		messages = append(messages, messageFromStore(&snapshot.Messages[i], threadID))
	}
	info := threadInfoFromSnapshot(snapshot)
	temp := cl.isConversation(snapshot.Thread.WorkspaceID)
	cl.deliver(func() {
		cl.chat.LoadSnapshot(threadID, messages, info, snapshot.HasMore, snapshot.OldestSeq)
		if cl.selectInSidebar != nil {
			cl.selectInSidebar(threadID)
		}
		cl.chat.SetTempWorkspace(temp)
	})
}

// isConversation 报告工作区是否为内置临时会话目录；读取失败按 false 处理。
func (cl *Client) isConversation(workspaceID string) bool {
	if cl.workspaces == nil || workspaceID == "" {
		return false
	}
	views, err := cl.workspaces.List()
	if err != nil {
		return false
	}
	for _, v := range views {
		if v.ID == workspaceID {
			return v.Purpose == "conversation"
		}
	}
	return false
}

// ---------------------------------------------------------------------------
// 发送 / 引导 / 停止 / 重新生成
// ---------------------------------------------------------------------------

// Submit 发送入口（唯一策略点，对应 threads.ts submit）：当前会话必须有
// 可用模型；模型不支持图片时拦截图片附件；运行中把输入转为运行中引导；
// 没有会话时先自动新建。被策略拦下时把草稿还回输入框。
func (cl *Client) Submit(text string, attachments []Attachment) {
	// 主线程快照：goroutine 里不再读状态。
	threadID := cl.chat.ActiveThreadID()
	mode := cl.currentMode()
	running := cl.chat.Running()
	modelRef := cl.chat.Thread().ModelID
	paths := attachmentPaths(attachments)

	if modelRef == "" {
		cl.deliver(func() {
			cl.chat.QueueToast("尚未选择模型：请先在设置中添加提供商并选择模型。")
			cl.chat.RestoreDraft(text, attachments)
		})
		return
	}
	if hasImagePath(paths) && !cl.modelVision(modelRef) {
		cl.deliver(func() {
			cl.chat.QueueToast("当前模型不支持图片输入：请切换支持图片的模型，或移除图片附件后再发送。")
			cl.chat.RestoreDraft(text, attachments)
		})
		return
	}
	if running {
		go cl.steerAsync(threadID, text, paths)
		return
	}
	// 普通发送：先乐观插入本地用户消息（后端不再广播 user.message），
	// 助手回复随后由事件流补上。
	cl.chat.AppendLocalUserMessage(text, attachments)
	cl.chat.SetRunning(true)
	if threadID == "" {
		go cl.createAndSendAsync(mode, text, paths)
		return
	}
	go cl.sendAsync(threadID, text, paths)
}

// sendAsync 调用发送命令；失败回滚运行态并还原草稿。
func (cl *Client) sendAsync(threadID, text string, paths []string) {
	if _, err := cl.chatSvc.Send(threadID, text, paths); err != nil {
		cl.deliver(func() {
			cl.chat.SetRunning(false)
			cl.chat.QueueToast("发送失败：" + err.Error())
		})
		slog.Warn("发送消息失败", "err", err)
		return
	}
	cl.deliver(func() { cl.chat.ClearAttachments() })
}

// steerAsync 运行中引导；失败只提示（引导的事件由后端广播，reducer
// 负责插入气泡）。
func (cl *Client) steerAsync(threadID, text string, paths []string) {
	if _, err := cl.chatSvc.Steer(threadID, text, paths); err != nil {
		cl.deliver(func() {
			cl.chat.QueueToast("引导发送失败：" + err.Error())
		})
		slog.Warn("发送引导失败", "err", err)
		return
	}
	cl.deliver(func() { cl.chat.ClearAttachments() })
}

// createAndSendAsync 自动新建会话后发送。会话归属：设置里的活动工作区；
// 没有活动工作区时退回内置临时目录。新建成功后同步侧栏选中。
func (cl *Client) createAndSendAsync(mode, text string, paths []string) {
	workspaceID := ""
	if cl.workspaces != nil {
		if views, err := cl.workspaces.List(); err == nil {
			for _, v := range views {
				if v.Purpose == "conversation" {
					workspaceID = v.ID
					break
				}
			}
		}
	}
	thread, err := cl.threads.Create(mode, workspaceID, "")
	if err != nil {
		cl.deliver(func() {
			cl.chat.SetRunning(false)
			cl.chat.QueueToast("新建会话失败：" + err.Error())
		})
		return
	}
	if cl.selectInSidebar != nil {
		cl.deliver(func() { cl.selectInSidebar(thread.ID) })
	}
	cl.sendAsync(thread.ID, text, paths)
}

// currentMode 返回当前模式（主线程）：有会话用会话模式，否则用兜底模式。
func (cl *Client) currentMode() string {
	if th := cl.chat.Thread(); th.Mode == "work" || th.Mode == "code" {
		return th.Mode
	}
	if cl.chat.fallbackMode == theme.ModeWork {
		return "work"
	}
	return "code"
}

// Cancel 停止当前会话运行：本地立即退出运行态，不等后端事件回来
// （threads.ts cancel 同语义）。
func (cl *Client) Cancel() {
	threadID := cl.chat.ActiveThreadID()
	if threadID == "" {
		return
	}
	cl.chat.SetRunning(false)
	cl.chat.DisarmInterrupt()
	go func() {
		if _, err := cl.chatSvc.Cancel(threadID); err != nil {
			slog.Warn("停止运行失败", "err", err)
		}
	}()
}

// Regenerate 重新生成最后一轮回复：立即标记运行中，失败回退并提示。
func (cl *Client) Regenerate() {
	threadID := cl.chat.ActiveThreadID()
	if threadID == "" {
		return
	}
	cl.chat.SetRunning(true)
	go func() {
		if _, err := cl.chatSvc.Regenerate(threadID); err != nil {
			cl.deliver(func() {
				cl.chat.SetRunning(false)
				cl.chat.QueueToast("重新生成失败：" + err.Error())
			})
			return
		}
		cl.deliver(func() { cl.chat.SetRunning(true) })
	}()
}

// Decide 提交审批决定（06 号票的审批卡片接管交互；接线先行）。
// 采用乐观更新由调用方处理：这里只回传命令，失败提示。
func (cl *Client) Decide(approvalID string, allow bool, scope string) {
	go func() {
		if err := cl.chatSvc.DecideApproval(approvalID, allow, scope); err != nil {
			cl.deliver(func() { cl.chat.QueueToast("审批提交失败：" + err.Error()) })
		}
	}()
}

// ---------------------------------------------------------------------------
// 附件
// ---------------------------------------------------------------------------

// PickFiles 打开系统文件选择框，把选中的文件加入待发送附件。
// Dialog.Open 阻塞到用户关闭，整体放 goroutine（04 号票坑：原生对话框
// 不能进 view 帧）。
func (cl *Client) PickFiles() {
	go func() {
		paths, err := mygoapp.Dialog.Open(mygoapp.OpenDialogOptions{Title: "添加附件", Multiple: true})
		if err != nil {
			slog.Warn("选择附件失败", "err", err)
			return
		}
		if len(paths) == 0 {
			return
		}
		cl.deliver(func() { cl.chat.AddAttachments(paths) })
	}()
}

// PasteImage 读取剪贴板图片并转存为附件（OnPaste 消费粘贴事件后调用，
// 已经在后台路径上）。视觉模型门控在 paste 入口完成；12MB 上限在这里
// 拦截（与 lib/paste.ts 的提示口径一致）。
func (cl *Client) PasteImage() {
	if appService == nil || cl.savePastedImage == nil {
		return
	}
	go func() {
		png := mygoapp.Clipboard.ReadImage()
		if len(png) == 0 {
			return
		}
		if len(png) > maxPastedAttachmentBytes {
			cl.deliver(func() { cl.chat.QueueToast("附件过大：图片超过 12MB，未加入附件。") })
			return
		}
		name := time.Now().Format("paste-20060102-150405.png")
		path, err := cl.savePastedImage(name, png)
		if err != nil {
			cl.deliver(func() { cl.chat.QueueToast("粘贴附件失败：" + err.Error()) })
			return
		}
		cl.deliver(func() { cl.chat.AddAttachments([]string{path}) })
	}()
}

// hasImagePath 报告附件里是否有图片（lib/utils isImagePath 同口径）。
func hasImagePath(paths []string) bool {
	for _, p := range paths {
		lower := strings.ToLower(p)
		for _, ext := range []string{".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"} {
			if strings.HasSuffix(lower, ext) {
				return true
			}
		}
	}
	return false
}

// modelVision 查询模型的图片能力；目录查不到按不支持处理。
func (cl *Client) modelVision(ref string) bool {
	_, info, ok := modelByRef(ref)
	return ok && info.Caps.Vision
}

// attachmentPaths 取附件草稿的路径列表。
func attachmentPaths(attachments []Attachment) []string {
	paths := make([]string, 0, len(attachments))
	for _, a := range attachments {
		paths = append(paths, a.Path)
	}
	return paths
}

// ---------------------------------------------------------------------------
// 翻页与杂项动作
// ---------------------------------------------------------------------------

// LoadEarlier 向前翻页：以当前第一条消息的最小部件序号作游标，取更早
// 一页并按 id 去重拼接（threads.ts loadEarlier 同语义）。
func (cl *Client) LoadEarlier() {
	threadID := cl.chat.ActiveThreadID()
	if threadID == "" || !cl.chat.HasEarlier() || cl.chat.LoadingEarlier() {
		return
	}
	firstSeq := cl.chat.OldestSeq()
	if firstSeq <= 0 {
		cl.deliver(func() { cl.chat.SetLoadingEarlier(false) })
		return
	}
	cl.chat.SetLoadingEarlier(true)
	go func() {
		snapshot, err := cl.threads.SnapshotPage(threadID, PageSize, firstSeq)
		if err != nil {
			cl.deliver(func() {
				cl.chat.SetLoadingEarlier(false)
				cl.chat.QueueToast("加载更早消息失败：" + err.Error())
			})
			return
		}
		older := make([]*Message, 0, len(snapshot.Messages))
		for i := range snapshot.Messages {
			older = append(older, messageFromStore(&snapshot.Messages[i], threadID))
		}
		cl.deliver(func() {
			cl.chat.PrependEarlier(older, snapshot.HasMore, snapshot.OldestSeq)
		})
	}()
}

// OpenFile 打开 path:line 引用（md.OnLink → craftwork:// 链接）。
// 生产装配把本动作覆盖为 shell.OpenFileAt（08 号票的正式入口：
// Pane.OpenFileAt）；这里的 toast 是装配缺失时（Tester / 兜底）的行为。
func (cl *Client) OpenFile(path string, line int) {
	locate := ""
	if line > 0 {
		locate = " 第 " + strconv.Itoa(line) + " 行"
	}
	cl.deliver(func() {
		cl.chat.QueueToast("文件引用：" + path + locate + "（文件面板待 08 号票接入）")
	})
}

// OpenDiff 打开审批卡的「查看完整差异」：生产装配把本动作覆盖为面板的
// 差异槽入口（shell.OpenDiff → Pane.OpenDiff，「应用全部」走面板的
// WriteFile 链路）；这里的 toast 是装配缺失时（Tester / 兜底）的行为。
func (cl *Client) OpenDiff(path, oldText, newText string) {
	title := path
	if idx := strings.LastIndex(title, "/"); idx >= 0 {
		title = title[idx+1:]
	}
	cl.deliver(func() {
		cl.chat.QueueToast("打开差异：" + title + "（面板差异槽未接线）")
	})
}

// ReadAttachment 读取一张附件图片的内容（mime 与 base64；goroutine 内
// 调用，结果经 SetUpdate 的通道回主线程写缓存）。appService 未注入时
// 返回错误，缩略图回退为文件 chip。
func (cl *Client) ReadAttachment(path string) (string, string, error) {
	if appService == nil {
		return "", "", errors.New("附件服务未接线")
	}
	content, err := appService.ReadAttachment(path)
	if err != nil || content == nil {
		return "", "", err
	}
	return content.Mime, content.Base64, nil
}

// ConfirmExternal 弹外链确认框，同意后交给系统浏览器。Dialog.Message
// 阻塞调用 goroutine，因此整体放后台，结果经 deliver 回主线程提示。
func (cl *Client) ConfirmExternal(url string) {
	go func() {
		result, err := mygoapp.Dialog.Message(mygoapp.MessageOptions{
			Type:          mygoapp.MessageQuestion,
			Title:         "用系统浏览器打开链接？",
			Message:       "应用内不会加载外部网页，链接将交给系统浏览器。",
			Detail:        url,
			Buttons:       []string{"取消", "打开链接"},
			DefaultButton: 0,
			CancelButton:  0,
		})
		if err != nil {
			slog.Warn("弹出链接确认失败", "err", err)
			return
		}
		if result.Button != 1 {
			return
		}
		if err := cl.openExternal(url); err != nil {
			cl.deliver(func() { cl.chat.QueueToast("无法打开链接：" + err.Error()) })
		}
	}()
}

// Copy 把文本写入剪贴板（service.ClipboardWriteText 统一出口）。剪贴板
// 要经主循环，从 deliver（win.Update）的回调里直写是安全的（04 号票的
// 既定做法）。
func (cl *Client) Copy(text string) {
	cl.deliver(func() {
		service.ClipboardWriteText(text)
		cl.chat.QueueToast("已复制")
	})
}

// ApplyBatch 把事件批次合并进聊天状态（主线程，emitter 桥投递）。
// 返回是否需要刷新会话列表（reducer 的 refreshNeeded）。
func (cl *Client) ApplyBatch(batch events.Batch) bool {
	return cl.chat.ApplyBatch(batch)
}
