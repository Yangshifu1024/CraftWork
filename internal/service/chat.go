// 本文件是对话相关的前端绑定：发送消息、重新生成、取消运行与回传
// 审批决定。服务层只做参数非空与状态前置校验，真正的 Agent 循环、
// 审批状态机与事件推送由 internal/agent 负责。

package service

import (
	"errors"
	"strings"

	"craftwork/internal/agent"
	"craftwork/internal/id"
	"craftwork/internal/model"
	"craftwork/internal/record"
	"craftwork/internal/store"
)

// ChatService 处理发送、取消、重新生成与审批决定。
type ChatService struct {
	// db 用于读取会话并写入运行记录。
	db *store.DB
	// records 负责写入消息事件与会话状态。
	records *record.Recorder
	// runner 执行 Agent 循环，一个会话同一时间只允许一个 run。
	runner *agent.Runner
	// approvals 保存待审批请求，并按用户决定放行或拒绝。
	approvals *agent.Approvals
}

// NewChatService 创建对话服务。
func NewChatService(db *store.DB, records *record.Recorder, runner *agent.Runner, approvals *agent.Approvals) *ChatService {
	return &ChatService{db: db, records: records, runner: runner, approvals: approvals}
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
		return "", errors.New("会话正在运行，请先停止")
	}
	if err := ensureThreadModel(*thread); err != nil {
		return "", err
	}
	text = strings.TrimSpace(text)
	if text == "" && len(attachments) == 0 {
		return "", errors.New("消息不能为空")
	}

	userMessageID := id.New("msg")
	// 用户消息先落事件表，再启动 run；即使后续模型调用失败，
	// 用户输入的原文也不会丢失。
	if err := s.records.Append(store.EventRecord{
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
		return "", errors.New("会话正在运行，请先停止")
	}
	if err := ensureThreadModel(*thread); err != nil {
		return "", err
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

// Cancel 停止会话上的 run，并取消等待中的审批。
func (s *ChatService) Cancel(threadID string) (bool, error) {
	canceled := s.runner.Cancel(threadID)
	if err := s.db.CancelThreadApprovals(threadID); err != nil {
		return canceled, err
	}
	if canceled {
		_ = s.records.SetThreadStatus(threadID, "", "idle")
	}
	return canceled, nil
}

// DecideApproval 回传用户对审批的决定。
//
// scope 的合法性与“拒绝时强制一次”等状态机规则由审批模块负责，
// 服务层只做进程间通信（IPC）参数的非空校验。
func (s *ChatService) DecideApproval(approvalID string, allow bool, scope string) error {
	if strings.TrimSpace(approvalID) == "" {
		return errors.New("approvalID 不能为空")
	}
	return s.approvals.Resolve(approvalID, agent.Decision{Allow: allow, Scope: scope})
}

// startRun 创建运行记录、更新会话状态并异步启动 Agent 循环，返回 runID。
func (s *ChatService) startRun(thread store.Thread) (string, error) {
	if err := ensureThreadModel(thread); err != nil {
		return "", err
	}
	runID := id.New("run")
	messageID := id.New("msg")
	// 先创建运行记录并把会话置为运行中，再交给 Runner 异步执行。
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
	if err := s.records.SetThreadStatus(thread.ID, runID, "running"); err != nil {
		return "", err
	}
	s.runner.Start(thread, runID, messageID, thread.Mode)
	return runID, nil
}

// ensureThreadModel 在写入用户消息前确认会话已经绑定可用模型。
//
// 首次启动没有任何提供商时，发送消息应该直接给出配置提示，而不是先写一条
// 用户消息再让 run 异步失败。
func ensureThreadModel(thread store.Thread) error {
	if strings.TrimSpace(thread.ModelID) == "" {
		return errors.New("尚未选择模型，请先在设置中添加提供商并选择模型")
	}
	if _, _, ok := model.Resolve(thread.ModelID); !ok {
		return errors.New("当前模型不可用，请重新选择模型")
	}
	return nil
}
