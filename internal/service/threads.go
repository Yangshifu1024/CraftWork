package service

import (
	"errors"
	"fmt"
	"strings"

	"craftwork/internal/events"
	"craftwork/internal/id"
	"craftwork/internal/model"
	"craftwork/internal/store"
)

// ThreadView 是线程列表使用的扁平结构。
type ThreadView struct {
	ID          string `json:"id"`
	Mode        string `json:"mode"`
	Title       string `json:"title"`
	ModelID     string `json:"modelId"`
	ModelName   string `json:"modelName"`
	WorkspaceID string `json:"workspaceId"`
	Status      string `json:"status"`
	Archived    bool   `json:"archived"`
	Tokens      int64  `json:"tokens"`
	CreatedAt   int64  `json:"createdAt"`
	UpdatedAt   int64  `json:"updatedAt"`
}

// ThreadService 管理线程的创建、列表、搜索与快照。
type ThreadService struct {
	db  *store.DB
	bus *events.Bus
}

// NewThreadService 创建线程服务。
func NewThreadService(db *store.DB, bus *events.Bus) *ThreadService {
	return &ThreadService{db: db, bus: bus}
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
	s.bus.Publish(events.TypeThreadUpdated, threadID, "", map[string]any{
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

func toViews(threads []store.Thread) []ThreadView {
	result := make([]ThreadView, 0, len(threads))
	for _, thread := range threads {
		result = append(result, toView(thread))
	}
	return result
}

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

func (s *ThreadService) publishChanged(threadID, mode string) {
	thread, err := s.db.GetThread(threadID)
	if err != nil {
		return
	}
	s.bus.Publish(events.TypeThreadUpdated, threadID, "", map[string]any{
		"threadId": threadID,
		"mode":     mode,
		"thread":   toView(*thread),
	})
}

func normalizeMode(mode string) string {
	if mode == "work" {
		return "work"
	}
	return "code"
}
