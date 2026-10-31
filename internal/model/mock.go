package model

import (
	"context"
	"strings"
	"time"

	"craftwork/internal/id"
)

// MockClient 是本地演示模型。它不访问网络，按固定脚本产生推理、文本与
// 工具调用，用于端到端验证流式渲染、工具执行与审批链路。
type MockClient struct {
	modelID string
}

// NewMock 创建演示客户端。
func NewMock(modelID string) *MockClient {
	if modelID == "" {
		modelID = "local-demo"
	}
	return &MockClient{modelID: modelID}
}

// Stream 实现 Client 接口。
func (m *MockClient) Stream(ctx context.Context, req Request) (<-chan Chunk, error) {
	channel := make(chan Chunk, 16)
	go func() {
		defer close(channel)

		lastUser, toolResults, toolNames := scanMessages(req.Messages)
		generated := 0

		if !hasTool(toolNames, "fs_list") {
			reasoning := "我先确认工作区的目录结构，再决定下一步。"
			if !streamPiece(ctx, channel, "reasoning", reasoning, &generated) {
				return
			}
			intro := "我先列一下当前工作区的文件，确认项目结构。"
			if !streamPiece(ctx, channel, "text", intro, &generated) {
				return
			}
			emitToolCall(ctx, channel, ToolCall{
				ID:   id.New("call"),
				Name: "todo_write",
				Args: map[string]any{"items": []map[string]any{
					{"text": "确认工作区结构", "status": "done"},
					{"text": "读取关键文件", "status": "active"},
					{"text": "给出下一步建议", "status": "pending"},
				}},
			})
			emitToolCall(ctx, channel, ToolCall{
				ID:   id.New("call"),
				Name: "fs_list",
				Args: map[string]any{"path": "."},
			})
		} else if !hasTool(toolNames, "shell_exec") {
			if wantsCommand(lastUser) {
				if !streamPiece(ctx, channel, "text", "结构已确认。接下来运行一次只读检查，确认当前分支状态。", &generated) {
					return
				}
				emitToolCall(ctx, channel, ToolCall{
					ID:   id.New("call"),
					Name: "shell_exec",
					Args: map[string]any{"command": "git status --short"},
				})
			} else {
				summary := summarizeToolResults(toolResults)
				if !streamPiece(ctx, channel, "text", "已读取工作区："+summary+"\n\n需要我继续查看某个文件或运行测试吗？", &generated) {
					return
				}
			}
		} else {
			summary := summarizeToolResults(toolResults)
			if !streamPiece(ctx, channel, "text", "命令执行完成。最近一次输出：\n\n"+summary+"\n\n你可以继续让我修改代码或运行测试。", &generated) {
				return
			}
		}

		usage := &Usage{
			InputTokens:  int64(estimateTokens(req.Messages)),
			OutputTokens: int64(generated/4 + 1),
		}
		select {
		case channel <- Chunk{Usage: usage, Done: true}:
		case <-ctx.Done():
		}
	}()
	return channel, nil
}

func scanMessages(messages []Message) (lastUser string, toolResults []string, toolNames []string) {
	for _, message := range messages {
		switch message.Role {
		case "user":
			lastUser = message.Content
		case "tool":
			toolResults = append(toolResults, message.Content)
		case "assistant":
			for _, call := range message.ToolCalls {
				toolNames = append(toolNames, call.Name)
			}
		}
	}
	return lastUser, toolResults, toolNames
}

func hasTool(names []string, name string) bool {
	for _, item := range names {
		if item == name {
			return true
		}
	}
	return false
}

func wantsCommand(text string) bool {
	for _, keyword := range []string{"测试", "运行", "命令", "检查", "test", "run", "git"} {
		if strings.Contains(strings.ToLower(text), keyword) {
			return true
		}
	}
	return false
}

func summarizeToolResults(results []string) string {
	if len(results) == 0 {
		return "（没有工具输出）"
	}
	last := results[len(results)-1]
	lines := strings.Split(strings.TrimSpace(last), "\n")
	if len(lines) > 12 {
		lines = lines[:12]
	}
	return strings.Join(lines, "\n")
}

func estimateTokens(messages []Message) int {
	total := 0
	for _, message := range messages {
		total += len([]rune(message.Content))
	}
	return total/4 + 1
}

func streamPiece(ctx context.Context, channel chan<- Chunk, field, text string, generated *int) bool {
	runes := []rune(text)
	for start := 0; start < len(runes); start += 4 {
		end := start + 4
		if end > len(runes) {
			end = len(runes)
		}
		piece := string(runes[start:end])
		*generated += len([]rune(piece))
		chunk := Chunk{}
		if field == "reasoning" {
			chunk.Reasoning = piece
		} else {
			chunk.Text = piece
		}
		select {
		case channel <- chunk:
		case <-ctx.Done():
			return false
		}
		if !sleep(ctx, 10*time.Millisecond) {
			return false
		}
	}
	return true
}

func emitToolCall(ctx context.Context, channel chan<- Chunk, call ToolCall) bool {
	select {
	case channel <- Chunk{ToolCall: &call}:
		return true
	case <-ctx.Done():
		return false
	}
}

func sleep(ctx context.Context, duration time.Duration) bool {
	timer := time.NewTimer(duration)
	defer timer.Stop()
	select {
	case <-timer.C:
		return true
	case <-ctx.Done():
		return false
	}
}

var _ Client = (*MockClient)(nil)
