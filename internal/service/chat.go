package service

import (
	"errors"
	"strings"

	"craftwork/internal/agent"
	"craftwork/internal/events"
	"craftwork/internal/id"
	"craftwork/internal/store"
)

// ChatService 处理发送、取消、重新生成与审批决定。
type ChatService struct {
	db     *store.DB
	bus    *events.Bus
	runner *agent.Runner
	broker *agent.Broker
}

// NewChatService 创建对话服务。
func NewChatService(db *store.DB, bus *events.Bus, runner *agent.Runner, broker *agent.Broker) *ChatService {
	return &ChatService{db: db, bus: bus, runner: runner, broker: broker}
}

// Send 写入用户消息并启动一次 run，返回 runID。
//
// attachments 是附件路径列表。路径与内容由 Agent 在构建模型历史时读取，
// 读取前会经 policy.Resolve 校验工作区边界并施加大小限制。
func (s *ChatService) Send(threadID, text string, attachments []string) (string, error) {
	thread, err := s.db.GetThread(threadID)
	if err != nil {
		return "", err
	}
	if s.runner.IsRunning(threadID) {
		return "", errors.New("线程正在运行，请先停止")
	}
	text = strings.TrimSpace(text)
	if text == "" && len(attachments) == 0 {
		return "", errors.New("消息不能为空")
	}

	userMessageID := id.New("msg")
	if _, err := s.db.AppendEvent(&store.EventRecord{
		ID:        id.New("part"),
		ThreadID:  threadID,
		MessageID: userMessageID,
		Kind:      "user_message",
		Payload:   map[string]any{"text": text, "attachments": attachments},
	}); err != nil {
		return "", err
	}

	return s.startRun(*thread)
}

// Regenerate 基于当前历史重新生成回复，不重复写入用户消息。
func (s *ChatService) Regenerate(threadID string) (string, error) {
	thread, err := s.db.GetThread(threadID)
	if err != nil {
		return "", err
	}
	if s.runner.IsRunning(threadID) {
		return "", errors.New("线程正在运行，请先停止")
	}
	snapshot, err := s.db.Snapshot(threadID)
	if err != nil {
		return "", err
	}
	hasUser := false
	for _, message := range snapshot.Messages {
		if message.Role == "user" {
			hasUser = true
			break
		}
	}
	if !hasUser {
		return "", errors.New("没有可重新生成的内容")
	}
	return s.startRun(*thread)
}

// Cancel 停止线程上的 run，并取消等待中的审批。
func (s *ChatService) Cancel(threadID string) (bool, error) {
	canceled := s.runner.Cancel(threadID)
	if err := s.db.CancelThreadApprovals(threadID); err != nil {
		return canceled, err
	}
	if canceled {
		_ = s.db.SetThreadStatus(threadID, "idle")
		s.bus.Publish(events.TypeThreadUpdated, threadID, "", map[string]any{
			"threadId": threadID,
			"status":   "idle",
		})
	}
	return canceled, nil
}

// DecideApproval 回传用户对审批的决定。
func (s *ChatService) DecideApproval(approvalID string, allow bool, scope string) error {
	if strings.TrimSpace(approvalID) == "" {
		return errors.New("approvalID 不能为空")
	}
	if !allow {
		scope = "once"
	}
	switch scope {
	case "once", "session", "workspace":
	default:
		return errors.New("无效的授权范围")
	}
	return s.broker.Resolve(approvalID, agent.Decision{Allow: allow, Scope: scope})
}

func (s *ChatService) startRun(thread store.Thread) (string, error) {
	runID := id.New("run")
	messageID := id.New("msg")
	if err := s.db.CreateRun(&store.Run{
		ID:        runID,
		ThreadID:  thread.ID,
		MessageID: messageID,
		Status:    "running",
		ModelID:   thread.ModelID,
		StartedAt: store.NowMillis(),
	}); err != nil {
		return "", err
	}
	if err := s.db.SetThreadStatus(thread.ID, "running"); err != nil {
		return "", err
	}
	s.bus.Publish(events.TypeThreadUpdated, thread.ID, runID, map[string]any{
		"threadId": thread.ID,
		"status":   "running",
	})
	s.runner.Start(thread, runID, messageID, thread.Mode)
	return runID, nil
}
