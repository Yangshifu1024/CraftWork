// 本文件是线程相关的前端绑定：列表、搜索、创建、重命名、切换模型、
// 归档、删除与分页快照。服务层负责把存储层的线程结构转换成前端
// 可直接使用的扁平视图，并在变更后推送 thread.updated 事件。

package service

import (
	"errors"
	"fmt"
	"strings"

	"craftwork/internal/events"
	"craftwork/internal/id"
	"craftwork/internal/model"
	"craftwork/internal/record"
	"craftwork/internal/store"
)

// ThreadView 是线程列表使用的扁平结构。
type ThreadView struct {
	// ID 是线程标识。
	ID string `json:"id"`
	// Mode 是 code 或 work。
	Mode string `json:"mode"`
	// Title 是展示标题。
	Title string `json:"title"`
	// ModelID 是线程当前使用的模型标识。
	ModelID string `json:"modelId"`
	// ModelName 是模型展示名称。
	ModelName string `json:"modelName"`
	// WorkspaceID 是线程绑定的工作区。
	WorkspaceID string `json:"workspaceId"`
	// Status 是 idle、running 或 awaiting。
	Status string `json:"status"`
	// Archived 表示线程是否已归档。
	Archived bool `json:"archived"`
	// Tokens 是该线程累计使用的 token 数。
	Tokens int64 `json:"tokens"`
	// CreatedAt 是创建时间（毫秒时间戳）。
	CreatedAt int64 `json:"createdAt"`
	// UpdatedAt 是最后更新时间（毫秒时间戳）。
	UpdatedAt int64 `json:"updatedAt"`
}

// ThreadService 管理线程的创建、列表、搜索与快照。
type ThreadService struct {
	// db 是线程与事件的存储入口。
	db *store.DB
	// records 用于推送线程变更事件。
	records *record.Recorder
}

// NewThreadService 创建线程服务。
func NewThreadService(db *store.DB, records *record.Recorder) *ThreadService {
	return &ThreadService{db: db, records: records}
}

// List 列出指定模式的线程。
func (s *ThreadService) List(mode string, includeArchived bool) ([]ThreadView, error) {
	threads, err := s.db.ListThreads(normalizeMode(mode), includeArchived)
	if err != nil {
		return nil, err
	}
	return toViews(threads), nil
}

// Search 按标题与内容搜索线程。
func (s *ThreadService) Search(mode, query string) ([]ThreadView, error) {
	if strings.TrimSpace(query) == "" {
		return s.List(mode, false)
	}
	threads, err := s.db.SearchThreads(normalizeMode(mode), query)
	if err != nil {
		return nil, err
	}
	return toViews(threads), nil
}

// Create 新建线程。
//
// 标题为空时使用“新对话”；模型取自当前设置的默认模型，
// 读取失败时保留 DefaultSettings 给出的默认值。
func (s *ThreadService) Create(mode, workspaceID, title string) (ThreadView, error) {
	settings := DefaultSettings()
	_ = s.db.GetSetting(settingsKey, &settings)

	if strings.TrimSpace(title) == "" {
		title = "新对话"
	}
	now := store.NowMillis()
	thread := &store.Thread{
		ID:          id.New("th"),
		Mode:        normalizeMode(mode),
		Title:       strings.TrimSpace(title),
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
	return toView(*thread), nil
}

// Rename 修改线程标题。
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

// SetModel 切换线程使用的模型。
func (s *ThreadService) SetModel(threadID, modelID string) error {
	if _, _, ok := model.FindModel(modelID); !ok {
		return fmt.Errorf("未知模型: %s", modelID)
	}
	if err := s.db.SetThreadModel(threadID, modelID); err != nil {
		return err
	}
	s.publishChanged(threadID, "")
	return nil
}

// Archive 归档或取消归档线程。
func (s *ThreadService) Archive(threadID string, archived bool) error {
	thread, err := s.db.GetThread(threadID)
	if err != nil {
		return err
	}
	if archived && (thread.Status == "running" || thread.Status == "awaiting") {
		return errors.New("线程正在运行，不能归档")
	}
	if err := s.db.SetThreadArchived(threadID, archived); err != nil {
		return err
	}
	s.publishChanged(threadID, thread.Mode)
	return nil
}

// Delete 删除线程及其事件。运行中的线程必须先取消。
func (s *ThreadService) Delete(threadID string) error {
	thread, err := s.db.GetThread(threadID)
	if err != nil {
		return err
	}
	if thread.Status == "running" || thread.Status == "awaiting" {
		return errors.New("线程正在运行，请先取消")
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

// Snapshot 返回线程消息、待审批与最后事件序号。
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

// toViews 批量把存储层的线程转换成前端视图，保持输入顺序。
func toViews(threads []store.Thread) []ThreadView {
	result := make([]ThreadView, 0, len(threads))
	for _, thread := range threads {
		result = append(result, toView(thread))
	}
	return result
}

// toView 把单个线程转换成前端视图，并补齐模型展示名称。
func toView(thread store.Thread) ThreadView {
	_, modelInfo := model.Resolve(thread.ModelID)
	return ThreadView{
		ID:          thread.ID,
		Mode:        thread.Mode,
		Title:       thread.Title,
		ModelID:     thread.ModelID,
		ModelName:   modelInfo.Name,
		WorkspaceID: thread.WorkspaceID,
		Status:      thread.Status,
		Archived:    thread.Archived,
		Tokens:      thread.Tokens,
		CreatedAt:   thread.CreatedAt,
		UpdatedAt:   thread.UpdatedAt,
	}
}

// publishChanged 重新读取线程并推送 thread.updated 事件。
// 线程已被删除或读取失败时静默返回，不影响主流程。
func (s *ThreadService) publishChanged(threadID, mode string) {
	thread, err := s.db.GetThread(threadID)
	if err != nil {
		return
	}
	s.records.Emit(events.TypeThreadUpdated, threadID, "", map[string]any{
		"threadId": threadID,
		"mode":     mode,
		"thread":   toView(*thread),
	})
}

// normalizeMode 归一化线程模式，未知值一律按 code 处理。
func normalizeMode(mode string) string {
	if mode == "work" {
		return "work"
	}
	return "code"
}
