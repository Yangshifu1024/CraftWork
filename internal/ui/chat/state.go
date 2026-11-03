// state.go 是聊天列的界面状态壳：当前会话的消息、运行态、待审批队列、
// 会话信息表、附件草稿与分页记忆。立即模式约定：只在主线程（view 或
// win.Update 回调）里读写，不放锁。
//
// 状态转移分两类：
//   - ApplyBatch 把事件批次交给 Reduce（纯函数）后写回结果；
//   - LoadSnapshot / LoadEarlier 用快照接管整份消息（刷新与切换会话）。
//
// 服务触达的动作在 client.go（Client），经 SetActions 注入；Actions 为 nil
// 或字段为 nil 时（单测路径）按钮只改本地状态。

package chat

import (
	"encoding/base64"
	"fmt"
	"strings"
	"time"

	mygo "github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// PageSize 是首屏与每次向前翻页的消息条数（threads.ts 同值）。
const PageSize = 80

// MaxAttachments 是附件草稿上限（threads.ts 同值）。
const MaxAttachments = 20

// Attachment 是一条待发送的附件草稿。
type Attachment struct {
	Path string
	Name string
}

// Actions 是聊天列触达服务层的动作集合（internal/app 装配注入；nil 字段
// 回退本地行为，供单测与接线前使用）。
type Actions struct {
	// Submit 发送一条消息（运行中自动转为 steer；无会话自动新建）。
	Submit func(text string, attachments []Attachment)
	// Cancel 停止当前会话的运行。
	Cancel func()
	// Regenerate 让最后一轮回复重新生成。
	Regenerate func()
	// Decide 提交审批决定（scope 为空串表示仅本次）。
	Decide func(approvalID string, allow bool, scope string)
	// PickFiles 打开系统文件选择框并把选中的文件加入附件草稿。
	PickFiles func()
	// PasteImage 读取剪贴板图片并转存为附件（goroutine；视觉模型门控在
	// paste 入口已完成）。
	PasteImage func()
	// LoadEarlier 向前翻页加载更早的消息。
	LoadEarlier func()
	// OpenFile 打开 path:line 引用指向的文件（面板 tab 系统归 08 号票，
	// 装配暂接 toast 占位）。
	OpenFile func(path string, line int)
	// ConfirmExternal 弹外链确认框并在同意后交给系统浏览器（goroutine）。
	ConfirmExternal func(url string)
	// Copy 把文本写入剪贴板。剪贴板要经主循环，实现必须排队到帧后执行
	//（app.go 用 win.Update），不能帧内直调。
	Copy func(text string)
	// OpenDiff 打开审批卡的「查看完整差异」（面板差异 tab 的入口；diff
	// 内容渲染归 09 号票，装配接 shell 的面板打开动作）。
	OpenDiff func(path, oldText, newText string)
	// ReadAttachment 读取一张附件图片（mime + base64；goroutine 内调用），
	// 供缩略图异步解码。nil 时图片附件回退为普通文件 chip。
	ReadAttachment func(path string) (mime, base64 string, err error)
}

// Chat 是聊天列的界面状态壳（thread store 的会话内部分 + composer/附件）。
type Chat struct {
	// activeThreadID 是当前会话 Id；空表示没有选中任何会话。
	activeThreadID string
	// slice 是会话内状态（消息、运行态、待审批、会话信息表）。
	slice Slice
	// hasEarlier 表示还有更早的消息可以向前翻页；oldestSeq 是翻页游标
	//（当前页第一条消息的最小部件序号）。
	hasEarlier bool
	oldestSeq  int64
	// loadingEarlier 防止翻页请求重复发出。
	loadingEarlier bool

	// attachments 是待发送的附件草稿。
	attachments []Attachment
	// draft 是输入框内容。
	draft string

	// pendingToast 是待弹的提示（异步动作完成后的反馈通道）。
	pendingToast string
	// interruptArmed 是双击 Esc 的待确认标记；armedAt 记录武装时刻
	//（composer 的 800ms 确认窗口）。
	interruptArmed bool
	armedAt        time.Time

	// approvalLevel 是审批档位的中文短标签（设置服务在装配时同步）。
	approvalLevel string
	// approvalLevelRaw 是审批档位的原始字符串（graded / strict / readonly /
	// free），审批卡按它判断永久授权按钮的显隐；空按分级处理（默认档）。
	approvalLevelRaw string
	// fallbackMode 是无选中会话时的兜底模式（侧栏当前模式，app 装配
	// 在模式切换时写入）；有会话时以会话自身 Mode 为准。
	fallbackMode theme.Mode
	// tempWorkspace 报告当前上下文是否为内置临时会话目录（空态文案分支）。
	tempWorkspace bool
	// list 是消息流的列表状态（Key / FollowEnd / 平滑跟底）。
	list mygo.ListState

	// lastColumnW 是最近一帧消息正文列的绘制宽度（14 号票居中限宽的帧内
	// 记录）：messageRow 每条消息行写入，同帧同值，ui.Tester 据此断言
	// 限宽与保底在真实布局里生效。
	lastColumnW float32
	// lastComposerW 是最近一帧 composer 外框的绘制宽度：外框与消息正文
	// 列同参数限宽（14 号票），走查据此断言两段对齐。
	lastComposerW float32
	// composerFocused 是 composer 外框的聚焦态（外框 FocusWithin 的帧内
	// 记录，14 号票聚焦高亮整个外框）：聚焦时描边切模式强调色，失焦回
	// 中性描边；ComposerFocused 供走查断言。
	composerFocused bool

	// 卡片界面状态（按部件 id 键控；展开语义见各卡片）。
	toolOpenState  map[string]bool
	toolTouched    map[string]bool
	reasoningOpen  map[string]bool
	reasoningTouch map[string]bool
	reasoningStart map[string]int64
	workOpenState  map[string]bool

	// thumbs 是附件图片的缩略图缓存：成功存位图、失败进负缓存、
	// 加载中有标记防重复发起（overlays 的 logo 模式）。
	thumbs map[string]thumbEntry
	// preview 是附件图片的放大预览浮层（DialogBase，07 浮层体系同机制）。
	preview imagePreviewState

	// acts 是服务动作入口；SetActions 注入。
	acts *Actions

	// update 把回调投回主线程（缩略图等后台加载完成后的写回通道）；
	// Client 构造时经 SetUpdate 注入，nil 时后台结果丢弃（单测路径）。
	update func(fn func())

	// streamStartMs 是当前流式回复首个增量的本地起点（实时速度估算用；
	// 没有流式正文时归零）。
	streamStartMs int64

	// mdOpts 是共享的 Markdown 渲染选项（解析缓存随视图帧保持），
	// mdTheme 是它的主题（每帧从当前色板同步，跟随系统深浅切换）。
	mdOpts  markdown.Options
	mdTheme markdown.Theme
}

// NewChat 创建聊天列状态。
func NewChat() *Chat {
	s := &Chat{}
	// 链接统一走 OnLink（安全策略见 markdown.go），处理函数绑定状态壳。
	s.mdOpts.OnLink = s.handleLink
	return s
}

// SetActions 注入服务动作集合（app.go 装配时调用一次；nil 合法）。
func (s *Chat) SetActions(a *Actions) { s.acts = a }

// SetUpdate 注入主线程投递通道（Client 构造时调用；nil 合法）。
func (s *Chat) SetUpdate(fn func(fn func())) { s.update = fn }

// Actions 返回动作集合（view 读取）。
func (s *Chat) Actions() *Actions { return s.acts }

// ---------------------------------------------------------------------------
// 只读派生（view 每帧调用）
// ---------------------------------------------------------------------------

// Messages 返回当前会话的消息列表。
func (s *Chat) Messages() []*Message { return s.slice.Messages }

// Running 报告当前会话是否运行中（awaiting 对界面同样属于运行中，
// 快照加载时已归一）。
func (s *Chat) Running() bool { return s.slice.Running }

// ActiveThreadID 返回当前会话 Id。
func (s *Chat) ActiveThreadID() string { return s.activeThreadID }

// Thread 返回当前会话的信息（标题、状态、模型与用量）；没有会话信息时
// 返回零值。列表（标题、会话级状态）以 sidebar 的会话列表为准，这里只
// 服务聊天头部与 Composer 的实时字段。
func (s *Chat) Thread() ThreadInfo { return s.slice.Threads[s.activeThreadID] }

// HasEarlier 报告是否还有更早的消息。
func (s *Chat) HasEarlier() bool { return s.hasEarlier }

// LoadingEarlier 报告翻页请求是否进行中。
func (s *Chat) LoadingEarlier() bool { return s.loadingEarlier }

// PendingApprovals 返回待审批队列。
func (s *Chat) PendingApprovals() []map[string]any { return s.slice.PendingApprovals }

// Attachments 返回附件草稿。
func (s *Chat) Attachments() []Attachment { return s.attachments }

// Draft 返回输入框内容（ *string 语义：ui.TextArea 直接绑定）。
func (s *Chat) DraftRef() *string { return &s.draft }

// InterruptArmed 返回双击 Esc 的待确认标记。
func (s *Chat) InterruptArmed() bool { return s.interruptArmed }

// TakeToast 取走排队中的提示；空表示没有待弹的提示。
func (s *Chat) TakeToast() string {
	message := s.pendingToast
	s.pendingToast = ""
	return message
}

// QueueToast 排队一条提示，view 下一帧开头弹出。
func (s *Chat) QueueToast(message string) { s.pendingToast = message }

// ---------------------------------------------------------------------------
// 事件合并（bus Batch 的唯一入口）
// ---------------------------------------------------------------------------

// ApplyBatch 把一批事件合并进状态，返回合并结果里的刷新标记：为真表示
// 出现了其他会话的变更或一次运行收尾，由 client 负责刷新会话列表。
func (s *Chat) ApplyBatch(batch events.Batch) bool {
	result := Reduce(s.slice, batch, s.activeThreadID)
	s.slice = result.Slice
	return result.RefreshNeeded
}

// ---------------------------------------------------------------------------
// 快照接管
// ---------------------------------------------------------------------------

// LoadSnapshot 用服务端快照接管整份消息（选中会话与刷新）。
//
// messages 是已归一化的当前页；thread 是会话信息；hasMore/oldestSeq 来自
// 分页接口。运行中可能有最后一条仍未写完的消息，事件处理需要它的 id。
func (s *Chat) LoadSnapshot(threadID string, messages []*Message, thread ThreadInfo, hasMore bool, oldestSeq int64) {
	s.activeThreadID = threadID
	s.slice.Messages = messages
	// awaiting 表示正在等审批，对界面来说同样属于「运行中」。
	s.slice.Running = thread.Status == ThreadStatusRunning || thread.Status == ThreadStatusAwaiting
	s.slice.RunningMessageID = ""
	for i := len(messages) - 1; i >= 0; i-- {
		if messages[i].Status == StatusStreaming {
			s.slice.RunningMessageID = messages[i].ID
			break
		}
	}
	// 历史快照里可能还有未处理的审批，恢复待审批队列。
	s.slice.PendingApprovals = nil
	for _, m := range messages {
		for _, p := range m.Parts {
			if p.Kind == PartApproval && strOf(p.Data, "status") == "waiting" {
				s.slice.PendingApprovals = append(s.slice.PendingApprovals, p.Data)
			}
		}
	}
	if thread.ID != "" {
		if s.slice.Threads == nil {
			s.slice.Threads = map[string]ThreadInfo{}
		}
		s.slice.Threads[threadID] = thread
	}
	s.hasEarlier = hasMore
	s.oldestSeq = oldestSeq
	s.loadingEarlier = false
}

// Clear 选中会话被清空（列表为空）时复位会话内状态；activeThreadID 由
// 调用方按需置空。
func (s *Chat) Clear() {
	s.slice.Messages = nil
	s.slice.Running = false
	s.slice.RunningMessageID = ""
	s.slice.PendingApprovals = nil
	s.hasEarlier = false
	s.oldestSeq = 0
	s.loadingEarlier = false
}

// SetActiveThread 写入当前会话 Id（client 在快照加载后调用）。
func (s *Chat) SetActiveThread(id string) { s.activeThreadID = id }

// AppendLocalUserMessage 乐观插入一条本地用户消息（普通发送路径：
// 后端不再广播 user.message，显示由界面先插入；React 版同语义）。
// 引导（steer）不走这里——后端广播的事件是显示的唯一来源。
func (s *Chat) AppendLocalUserMessage(text string, attachments []Attachment) {
	id := fmt.Sprintf("msg-local-%d", time.Now().UnixNano())
	data := map[string]any{"text": text}
	list := make([]any, 0, len(attachments))
	for _, a := range attachments {
		list = append(list, map[string]any{"name": a.Name, "path": a.Path})
	}
	data["attachments"] = list
	s.slice.Messages = append(cloneMessages(s.slice.Messages), &Message{
		ID:        id,
		ThreadID:  s.activeThreadID,
		Role:      RoleUser,
		Status:    StatusComplete,
		CreatedAt: theme.Now(),
		Parts:     []Part{{ID: id + "-text", Kind: PartText, Data: data}},
	})
}

// PrependEarlier 把更早的一页消息按 id 去重后拼到前面，并更新翻页游标。
func (s *Chat) PrependEarlier(older []*Message, hasMore bool, oldestSeq int64) {
	existing := make(map[string]bool, len(s.slice.Messages))
	for _, m := range s.slice.Messages {
		existing[m.ID] = true
	}
	prepended := make([]*Message, 0, len(older))
	for _, m := range older {
		if !existing[m.ID] {
			prepended = append(prepended, m)
		}
	}
	merged := make([]*Message, 0, len(prepended)+len(s.slice.Messages))
	merged = append(merged, prepended...)
	merged = append(merged, s.slice.Messages...)
	s.slice.Messages = merged
	s.hasEarlier = hasMore
	s.oldestSeq = oldestSeq
	s.loadingEarlier = false
	// 更早的历史里可能还有未处理的审批，重新汇总一次。
	s.slice.PendingApprovals = nil
	for _, m := range merged {
		for _, p := range m.Parts {
			if p.Kind == PartApproval && strOf(p.Data, "status") == "waiting" {
				s.slice.PendingApprovals = append(s.slice.PendingApprovals, p.Data)
			}
		}
	}
}

// OldestSeq 返回翻页游标（当前第一条消息的最小有效部件序号；没有有效
// 序号时返回 0，调用方据此停止翻页）。
func (s *Chat) OldestSeq() int64 {
	smallest := int64(0)
	for _, m := range s.slice.Messages {
		for _, p := range m.Parts {
			if p.Seq > 0 && (smallest == 0 || p.Seq < smallest) {
				smallest = p.Seq
			}
		}
	}
	return smallest
}

// SetLoadingEarlier 写入翻页进行中标记。
func (s *Chat) SetLoadingEarlier(v bool) { s.loadingEarlier = v }

// ---------------------------------------------------------------------------
// 附件草稿
// ---------------------------------------------------------------------------

// AddAttachments 追加附件草稿：按路径去重、上限 20 条。
func (s *Chat) AddAttachments(paths []string) {
	existing := make(map[string]bool, len(s.attachments))
	for _, a := range s.attachments {
		existing[a.Path] = true
	}
	for _, p := range paths {
		if p == "" || existing[p] {
			continue
		}
		existing[p] = true // 同一次调用内的重复路径也去重
		name := p
		if idx := strings.LastIndex(p, "/"); idx >= 0 {
			name = p[idx+1:]
		}
		s.attachments = append(s.attachments, Attachment{Path: p, Name: name})
	}
	if len(s.attachments) > MaxAttachments {
		s.attachments = s.attachments[:MaxAttachments]
	}
}

// RemoveAttachment 按路径移除单个附件草稿。
func (s *Chat) RemoveAttachment(path string) {
	kept := s.attachments[:0]
	for _, a := range s.attachments {
		if a.Path != path {
			kept = append(kept, a)
		}
	}
	s.attachments = kept
}

// ClearAttachments 清空附件草稿（发送成功后调用）。
func (s *Chat) ClearAttachments() { s.attachments = nil }

// ---------------------------------------------------------------------------
// 输入与运行控制（主线程入口）
// ---------------------------------------------------------------------------

// Submit 提交输入：走 Actions.Submit（策略分流在 client：运行中转 steer、
// 无会话自动新建）。发送成功由 client 清空草稿与附件；这里先按乐观路径
// 清空输入框（与 assistant-ui 提交即清空的观感一致），失败时 client 会
// 把文本还回来。
func (s *Chat) Submit() {
	text := strings.TrimSpace(s.draft)
	if text == "" && len(s.attachments) == 0 {
		return
	}
	attachments := s.attachments
	if s.acts != nil && s.acts.Submit != nil {
		s.draft = ""
		s.attachments = nil
		s.acts.Submit(text, attachments)
		return
	}
	// 未接服务（单测）：只清输入。
	s.draft = ""
	s.attachments = nil
}

// RestoreDraft 发送失败时把文本与附件还回输入框。
func (s *Chat) RestoreDraft(text string, attachments []Attachment) {
	s.draft = text
	s.attachments = attachments
}

// SetRunning 写入运行态（cancel / regenerate 的乐观标记）。
func (s *Chat) SetRunning(v bool) { s.slice.Running = v }

// SetFallbackMode 写入无会话时的兜底模式（侧栏模式切换时由装配同步）。
func (s *Chat) SetFallbackMode(m theme.Mode) { s.fallbackMode = m }

// ModeFromSidebar 把侧栏的模式字符串（code / work）换算为 ui 模式枚举；
// 未知值按 Code 处理。
func ModeFromSidebar(mode string) theme.Mode {
	if mode == "work" {
		return theme.ModeWork
	}
	return theme.ModeCode
}

// SetTempWorkspace 写入临时会话目录标记（空态文案分支）。
func (s *Chat) SetTempWorkspace(v bool) { s.tempWorkspace = v }

// SetApprovalLevel 写入审批档位的中文短标签。
func (s *Chat) SetApprovalLevel(label string) { s.approvalLevel = label }

// SetApprovalLevelRaw 写入审批档位的原始字符串（审批卡永久授权按钮的
// 显隐判断用；空按分级处理）。
func (s *Chat) SetApprovalLevelRaw(level string) { s.approvalLevelRaw = level }

// HandleWindowEscape 是窗口级 Esc 退出链里聊天列的两级（AppShell.tsx 的
// Esc 链移植；浮层 / 面板的层级归浮层体系与 08 / 10 号票）：
//   - 运行中：双击 Esc 才取消，第一击只武装停止按钮（短窗口内再按一次
//     才真正停止，避免误触打断任务）；
//   - 非运行但有待审批：按 Esc 视为拒绝最后一条审批请求，范围固定为
//     「仅本次」。
//
// 返回 true 表示 Esc 已被消费（根视图的窗口快捷键据此忽略）。
func (s *Chat) HandleWindowEscape() bool {
	if s.Running() {
		now := time.Now()
		if s.interruptArmed && now.Sub(s.armedAt) <= escArmWindow {
			s.DisarmInterrupt()
			s.SetRunning(false)
			if s.acts != nil && s.acts.Cancel != nil {
				s.acts.Cancel()
			}
			return true
		}
		s.interruptArmed = true
		s.armedAt = now
		return true
	}
	s.DisarmInterrupt()
	if pending := s.slice.PendingApprovals; len(pending) > 0 {
		last := pending[len(pending)-1]
		if id := strOf(last, "approvalId"); id != "" {
			if s.acts != nil && s.acts.Decide != nil {
				s.acts.Decide(id, false, "once")
			}
			return true
		}
	}
	return false
}

// ---------------------------------------------------------------------------
// 卡片界面状态（按部件 id 键控）
// ---------------------------------------------------------------------------

// toolOpen 读取工具卡的展开态；没有手动操作过时按状态取默认（失败默认
// 展开，其余收起；ToolCard 的 useState(data.status === "error") 同语义）。
func (s *Chat) toolOpen(id string, defaultOpen bool) bool {
	if s.toolTouched[id] {
		return s.toolOpenState[id]
	}
	return defaultOpen
}

// setToolOpen 写入工具卡的展开态并标记已手动操作。
func (s *Chat) setToolOpen(id string, open bool) {
	if s.toolOpenState == nil {
		s.toolOpenState = map[string]bool{}
	}
	if s.toolTouched == nil {
		s.toolTouched = map[string]bool{}
	}
	s.toolOpenState[id], s.toolTouched[id] = open, true
}

// rememberReasoningStart 记录思考部件的流式起点（首次见到时）。
func (s *Chat) rememberReasoningStart(id string) {
	if s.reasoningStart == nil {
		s.reasoningStart = map[string]int64{}
	}
	if _, ok := s.reasoningStart[id]; !ok {
		s.reasoningStart[id] = theme.Now()
	}
}

// forgetReasoningStart 清掉思考部件的流式起点（流式结束）。
func (s *Chat) forgetReasoningStart(id string) {
	if s.reasoningStart != nil {
		delete(s.reasoningStart, id)
	}
}

// reasoningElapsed 返回思考已进行的秒数（至少 1；起点缺失按 1 秒）。
func (s *Chat) reasoningElapsed(id string) int {
	start, ok := s.reasoningStart[id]
	if !ok {
		return 1
	}
	seconds := int((theme.Now() - start) / 1000)
	if seconds < 1 {
		seconds = 1
	}
	return seconds
}

// setReasoningOpen 写入思考块的展开态并标记已手动操作（手动操作过后，
// 流式结束不再自动收起）。
func (s *Chat) setReasoningOpen(id string, open bool) {
	if s.reasoningOpen == nil {
		s.reasoningOpen = map[string]bool{}
	}
	if s.reasoningTouch == nil {
		s.reasoningTouch = map[string]bool{}
	}
	s.reasoningOpen[id], s.reasoningTouch[id] = open, true
}

// workOpen 读取「已工作」折叠区的展开态（默认收起）。
func (s *Chat) workOpen(key string) bool { return s.workOpenState[key] }

// setWorkOpen 写入「已工作」折叠区的展开态。
func (s *Chat) setWorkOpen(key string, open bool) {
	if s.workOpenState == nil {
		s.workOpenState = map[string]bool{}
	}
	s.workOpenState[key] = open
}

// ---------------------------------------------------------------------------
// 附件缩略图与放大预览
// ---------------------------------------------------------------------------

// thumbEntry 是一张附件图片的加载状态：成功存位图，失败进负缓存。
type thumbEntry struct {
	bmp     *mygo.Bitmap
	loading bool
	failed  bool
}

// imagePreviewState 是附件图片放大预览浮层的状态。
type imagePreviewState struct {
	open bool
	name string
	bmp  *mygo.Bitmap
}

// thumbOf 返回附件路径的缩略图位图；没有缓存时发起后台加载并返回 nil
// （视图先画文件 chip，加载完成后自动替换）。纯 UI / 测试路径（没有注入
// ReadAttachment）不加载，始终回退文件 chip。
func (s *Chat) thumbOf(path string) *mygo.Bitmap {
	if s.acts == nil || s.acts.ReadAttachment == nil || !isImagePath(path) {
		return nil
	}
	if s.thumbs == nil {
		s.thumbs = map[string]thumbEntry{}
	}
	entry, ok := s.thumbs[path]
	if !ok {
		s.thumbs[path] = thumbEntry{loading: true}
		read := s.acts.ReadAttachment
		notify := s.update
		go func() {
			_, base64Data, err := read(path)
			var bmp *mygo.Bitmap
			if err == nil && base64Data != "" {
				if data, derr := base64.StdEncoding.DecodeString(base64Data); derr == nil {
					bmp, derr = mygo.DecodeBitmap(data)
					if derr != nil {
						bmp = nil
					}
				}
			}
			if notify == nil {
				return
			}
			// 位图解码在后台完成，经 win.Update 回主线程写缓存。
			notify(func() {
				e := s.thumbs[path]
				e.loading = false
				e.failed = bmp == nil
				e.bmp = bmp
				s.thumbs[path] = e
			})
		}()
		return nil
	}
	return entry.bmp
}

// OpenImagePreview 打开一张图片的放大预览浮层（07 浮层体系的 DialogBase
// 同机制，浮层归聊天列自持：预览的是消息附件，Esc / 遮罩点击关闭）。
func (s *Chat) OpenImagePreview(name string, bmp *mygo.Bitmap) {
	s.preview = imagePreviewState{open: true, name: name, bmp: bmp}
}

// Preview 返回当前打开的图片预览（无则 open 为 false）。
func (s *Chat) Preview() imagePreviewState { return s.preview }

// CloseImagePreview 关闭图片预览浮层。
func (s *Chat) CloseImagePreview() { s.preview.open = false }

// ArmInterrupt 标记双击 Esc 的第一击。
func (s *Chat) ArmInterrupt() { s.interruptArmed = true }

// DisarmInterrupt 解除双击 Esc 标记。
func (s *Chat) DisarmInterrupt() { s.interruptArmed = false }
