// 本文件是会话相关的前端绑定：列表、搜索、创建、重命名、切换模型、
// 归档、删除与分页快照。服务层负责把存储层的会话结构转换成前端
// 可直接使用的扁平视图，并在变更后推送 thread.updated 事件。

package service

import (
	"errors"
	"fmt"
	"strings"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/id"
	"github.com/yangshifu1024/craftwork/internal/model"
	"github.com/yangshifu1024/craftwork/internal/record"
	"github.com/yangshifu1024/craftwork/internal/store"

	threadtitle "github.com/yangshifu1024/craftwork/internal/title"
)

// ThreadView 是会话列表使用的扁平结构。
type ThreadView struct {
	// ID 是会话标识。
	ID string `json:"id"`
	// Mode 是 code 或 work。
	Mode string `json:"mode"`
	// Title 是展示标题。
	Title string `json:"title"`
	// ModelID 是会话当前使用的模型标识。
	ModelID string `json:"modelId"`
	// ModelName 是模型展示名称。
	ModelName string `json:"modelName"`
	// WorkspaceID 是会话绑定的工作区。
	WorkspaceID string `json:"workspaceId"`
	// Status 是 idle、running 或 awaiting。
	Status string `json:"status"`
	// Archived 表示会话是否已归档。
	Archived bool `json:"archived"`
	// Tokens 是该会话累计使用的 token 数。
	Tokens int64 `json:"tokens"`
	// ContextTokens 是最近一次模型调用的输入加输出 token，即当前上下文
	// 占用；每轮调用后覆盖更新，与累计的 Tokens 口径不同。
	ContextTokens int64 `json:"contextTokens"`
	// ContextCacheTokens 是最近一次模型调用命中的缓存 token，用于展示
	// 缓存收益；没有调用记录时为 0。
	ContextCacheTokens int64 `json:"contextCacheTokens"`
	// ContextSpeed 是最近一次模型调用的输出速度（tok/s），由输出 token
	// 除以调用耗时得出；耗时未知或没有调用记录时为 0。
	ContextSpeed float64 `json:"contextSpeed"`
	// CreatedAt 是创建时间（毫秒时间戳）。
	CreatedAt int64 `json:"createdAt"`
	// UpdatedAt 是最后更新时间（毫秒时间戳）。
	UpdatedAt int64 `json:"updatedAt"`
}

// ThreadService 管理会话的创建、列表、搜索与快照。
type ThreadService struct {
	// db 是会话与事件的存储入口。
	db *store.DB
	// records 用于推送会话变更事件。
	records *record.Recorder
}

// NewThreadService 创建会话服务。
func NewThreadService(db *store.DB, records *record.Recorder) *ThreadService {
	return &ThreadService{db: db, records: records}
}

// List 列出指定模式的会话。
func (s *ThreadService) List(mode string, includeArchived bool) ([]ThreadView, error) {
	threads, err := s.db.ListThreads(normalizeMode(mode), includeArchived)
	if err != nil {
		return nil, err
	}
	return toViews(threads, s.latestUsage(threads)), nil
}

// Search 按标题与内容搜索会话。
func (s *ThreadService) Search(mode, query string) ([]ThreadView, error) {
	if strings.TrimSpace(query) == "" {
		return s.List(mode, false)
	}
	threads, err := s.db.SearchThreads(normalizeMode(mode), query)
	if err != nil {
		return nil, err
	}
	return toViews(threads, s.latestUsage(threads)), nil
}

// Create 新建会话。
//
// 标题为空时使用“新对话”；模型取自当前设置的默认模型，
// 读取失败时保留 DefaultSettings 给出的默认值。
// workspaceID 可为空（允许无工作区的历史语义），非空时必须是存在的
// 工作区，避免产生运行时 workspace_missing 的悬挂绑定。
func (s *ThreadService) Create(mode, workspaceID, title string) (ThreadView, error) {
	if workspaceID != "" {
		if _, err := s.db.GetWorkspace(workspaceID); err != nil {
			return ThreadView{}, errors.New("工作区不存在，请重新选择工作区后再新建会话")
		}
	}
	settings := DefaultSettings()
	_ = s.db.GetSetting(settingsKey, &settings)
	// 默认模型可能是旧格式或已失效；创建会话时先规范化，避免把坏值写进新会话。
	normalizeModelSettings(&settings)

	trimmedTitle := strings.TrimSpace(title)
	// 没有显式标题的新会话允许首轮自动命名；复制会话等显式标题视为手动命名。
	titleAuto := trimmedTitle == ""
	if titleAuto {
		trimmedTitle = threadtitle.Default
	}
	now := store.NowMillis()
	thread := &store.Thread{
		ID:          id.New("th"),
		Mode:        normalizeMode(mode),
		Title:       trimmedTitle,
		TitleAuto:   titleAuto,
		ModelID:     settings.DefaultModel,
		WorkspaceID: workspaceID,
		Status:      "idle",
		CreatedAt:   now,
		UpdatedAt:   now,
	}
	if err := s.db.CreateThread(thread); err != nil {
		return ThreadView{}, err
	}
	s.publishChanged(thread.ID, thread.Mode)
	return toView(*thread, nil), nil
}

// Rename 修改会话标题。
func (s *ThreadService) Rename(threadID, title string) error {
	title = strings.TrimSpace(title)
	if title == "" {
		return errors.New("标题不能为空")
	}
	if len([]rune(title)) > 80 {
		title = string([]rune(title)[:80])
	}
	if err := s.db.RenameThread(threadID, title); err != nil {
		return err
	}
	s.publishChanged(threadID, "")
	return nil
}

// SetModel 切换会话使用的模型。
//
// 传入的可以是完整引用或旧的原始模型 ID；保存时统一写成引用。
func (s *ThreadService) SetModel(threadID, modelID string) error {
	if strings.TrimSpace(modelID) == "" {
		return errors.New("请选择模型")
	}
	_, info, ok := model.Resolve(modelID)
	if !ok {
		return fmt.Errorf("未知模型: %s", modelID)
	}
	if err := s.db.SetThreadModel(threadID, info.Ref()); err != nil {
		return err
	}
	s.publishChanged(threadID, "")
	return nil
}

// Archive 归档或取消归档会话。
func (s *ThreadService) Archive(threadID string, archived bool) error {
	thread, err := s.db.GetThread(threadID)
	if err != nil {
		return err
	}
	if archived && (thread.Status == "running" || thread.Status == "awaiting") {
		return errors.New("会话正在运行，不能归档")
	}
	if err := s.db.SetThreadArchived(threadID, archived); err != nil {
		return err
	}
	s.publishChanged(threadID, thread.Mode)
	return nil
}

// Delete 删除会话及其事件。运行中的会话必须先取消。
func (s *ThreadService) Delete(threadID string) error {
	thread, err := s.db.GetThread(threadID)
	if err != nil {
		return err
	}
	if thread.Status == "running" || thread.Status == "awaiting" {
		return errors.New("会话正在运行，请先取消")
	}
	if err := s.db.DeleteThread(threadID); err != nil {
		return err
	}
	s.records.Emit(events.TypeThreadUpdated, threadID, "", map[string]any{
		"threadId": threadID,
		"deleted":  true,
		"mode":     thread.Mode,
	})
	return nil
}

// Snapshot 返回会话消息、待审批与最后事件序号。
func (s *ThreadService) Snapshot(threadID string) (*store.Snapshot, error) {
	return s.db.Snapshot(threadID)
}

// SnapshotPage 返回最近一页消息，支持按部件序号向前翻页。
//
// beforeSeq 为 0 时取最新一页；大于 0 时返回该序号之前的消息。limit
// 会被限制在 1–500 之间，避免一次拉取过多数据。
func (s *ThreadService) SnapshotPage(threadID string, limit int, beforeSeq int64) (*store.Snapshot, error) {
	if limit <= 0 {
		limit = 80
	}
	if limit > 500 {
		limit = 500
	}
	snapshot, err := s.db.Snapshot(threadID)
	if err != nil {
		return nil, err
	}

	messages := snapshot.Messages
	if beforeSeq > 0 {
		// 只保留最后一条消息的序号小于游标的记录，即更早的一页。
		older := make([]store.Message, 0, len(messages))
		for _, message := range messages {
			if lastPartSeq(message) < beforeSeq {
				older = append(older, message)
			}
		}
		messages = older
	}
	if len(messages) > limit {
		// 取末尾 limit 条（最新的消息），并告知前端还有更早的内容。
		snapshot.Messages = messages[len(messages)-limit:]
		snapshot.HasMore = true
	} else {
		snapshot.Messages = messages
		snapshot.HasMore = false
	}
	// 返回本页最小序号，前端下次用它作为 beforeSeq 继续向前翻页。
	snapshot.OldestSeq = oldestPartSeq(snapshot.Messages)
	return snapshot, nil
}

// lastPartSeq 返回一条消息中最大的部件序号，用于分页边界比较。
func lastPartSeq(message store.Message) int64 {
	var last int64
	for _, part := range message.Parts {
		if part.Seq > last {
			last = part.Seq
		}
	}
	return last
}

// oldestPartSeq 返回一组消息中最小的部件序号，作为继续向前翻页的游标。
// 没有部件时返回 0。
func oldestPartSeq(messages []store.Message) int64 {
	for _, message := range messages {
		for _, part := range message.Parts {
			return part.Seq
		}
	}
	return 0
}

// latestUsage 批量查询会话的最近一次模型用量。
//
// 上下文占用属于展示性数据，读取失败时返回空映射而不是让整个列表
// 接口失败；前端按零值显示。
func (s *ThreadService) latestUsage(threads []store.Thread) map[string]store.UsageRow {
	ids := make([]string, len(threads))
	for i, thread := range threads {
		ids[i] = thread.ID
	}
	latest, err := s.db.LatestUsageByThreads(ids)
	if err != nil {
		return map[string]store.UsageRow{}
	}
	return latest
}

// toViews 批量把存储层的会话转换成前端视图，保持输入顺序。
//
// latest 是每个会话最近一次模型调用的用量，用于填充上下文占用；
// 允许传 nil，此时相关字段为零值。
func toViews(threads []store.Thread, latest map[string]store.UsageRow) []ThreadView {
	result := make([]ThreadView, 0, len(threads))
	for _, thread := range threads {
		result = append(result, toView(thread, latest))
	}
	return result
}

// toView 把单个会话转换成前端视图，并补齐模型展示名称。
//
// ModelID 返回规范引用，前端用它匹配模型选择器；旧会话里的原始模型 ID
// 会在读取时自动转换。模型为空或已失效时给出可读提示。
func toView(thread store.Thread, latest map[string]store.UsageRow) ThreadView {
	modelID := thread.ModelID
	modelName := "未选择模型"
	if modelID != "" {
		if _, info, ok := model.Resolve(modelID); ok {
			modelID = info.Ref()
			modelName = info.Name
		} else {
			modelName = "模型不可用"
		}
	}
	// 上下文占用 = 最近一次调用的输入加输出；没有调用记录时保持零值，
	// 前端据此回退到本地估算。速度由输出 token 除以调用耗时得出。
	var contextTokens, contextCache int64
	contextSpeed := 0.0
	if row, ok := latest[thread.ID]; ok {
		contextTokens = row.InputTokens + row.OutputTokens
		contextCache = row.CacheTokens
		if row.DurationMs > 0 {
			contextSpeed = float64(row.OutputTokens) / (float64(row.DurationMs) / 1000)
		}
	}
	return ThreadView{
		ID:                 thread.ID,
		Mode:               thread.Mode,
		Title:              thread.Title,
		ModelID:            modelID,
		ModelName:          modelName,
		WorkspaceID:        thread.WorkspaceID,
		Status:             thread.Status,
		Archived:           thread.Archived,
		Tokens:             thread.Tokens,
		ContextTokens:      contextTokens,
		ContextCacheTokens: contextCache,
		ContextSpeed:       contextSpeed,
		CreatedAt:          thread.CreatedAt,
		UpdatedAt:          thread.UpdatedAt,
	}
}

// publishChanged 重新读取会话并推送 thread.updated 事件。
// 会话已被删除或读取失败时静默返回，不影响主流程。
func (s *ThreadService) publishChanged(threadID, mode string) {
	thread, err := s.db.GetThread(threadID)
	if err != nil {
		return
	}
	s.records.Emit(events.TypeThreadUpdated, threadID, "", map[string]any{
		"threadId": threadID,
		"mode":     mode,
		"thread":   toView(*thread, s.latestUsage([]store.Thread{*thread})),
	})
}

// normalizeMode 归一化会话模式，未知值一律按 code 处理。
func normalizeMode(mode string) string {
	if mode == "work" {
		return "work"
	}
	return "code"
}
