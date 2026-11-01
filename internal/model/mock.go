// 本文件负责本地演示模型：不访问网络，按固定脚本依次产生推理、文本与工具
// 调用，用于端到端验证流式渲染、工具执行与审批链路。

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
	// modelID 是演示客户端对外声明的模型标识，只在目录展示时使用。
	modelID string
}

// NewMock 创建演示客户端。modelID 为空时默认使用 local-demo。
func NewMock(modelID string) *MockClient {
	if modelID == "" {
		modelID = "local-demo"
	}
	return &MockClient{modelID: modelID}
}

// Stream 实现 Client 接口。
//
// 脚本按对话历史里的工具调用分阶段推进：
//  1. 还没调用过 fs_list 时，先输出推理与说明，再发起 todo_write 与 fs_list；
//  2. 已调用 fs_list 但未调用 shell_exec 时，根据用户是否提到运行、测试等
//     关键词决定是否执行 git status --short，否则直接汇总工具结果；
//  3. 已执行 shell_exec 时，汇总最近一次工具输出。
//
// 文本按每次 4 个字符切片并间隔 10 毫秒发送，模拟流式输出；最后发送带用量
// 与 Done 标记的结束片段。
func (m *MockClient) Stream(ctx context.Context, req Request) (<-chan Chunk, error) {
	channel := make(chan Chunk, 16)
	go func() {
		defer close(channel)

		// 扫描历史消息，得到最后一条用户输入、已有工具结果与已调用工具名。
		lastUser, toolResults, toolNames := scanMessages(req.Messages)
		generated := 0

		if !hasTool(toolNames, "fs_list") {
			// 第一阶段：先说明思路，再登记待办并列出目录。
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
			// 第二阶段：用户提到运行、测试等动作时才发起命令，否则直接总结。
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
			// 第三阶段：命令已执行，汇总最近一次工具输出。
			summary := summarizeToolResults(toolResults)
			if !streamPiece(ctx, channel, "text", "命令执行完成。最近一次输出：\n\n"+summary+"\n\n你可以继续让我修改代码或运行测试。", &generated) {
				return
			}
		}

		// 结束片段携带估算用量与 Done 标记，Runner 据此收尾。
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

// scanMessages 扫描对话历史，提取最后一条用户输入、所有工具结果，以及
// 助手消息里出现过的工具名，供演示脚本判断当前进行到第几阶段。
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

// hasTool 判断工具名列表中是否包含指定工具。
func hasTool(names []string, name string) bool {
	for _, item := range names {
		if item == name {
			return true
		}
	}
	return false
}

// wantsCommand 判断用户输入是否表达了执行命令的意图。关键词覆盖中英文的
// 运行、测试、检查与 git 等常见说法，输入会先转为小写再比较。
func wantsCommand(text string) bool {
	for _, keyword := range []string{"测试", "运行", "命令", "检查", "test", "run", "git"} {
		if strings.Contains(strings.ToLower(text), keyword) {
			return true
		}
	}
	return false
}

// summarizeToolResults 取最近一条工具输出并截断到 12 行，避免演示回复过长。
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

// estimateTokens 用字符数除以 4 粗略估算输入 token 数，仅用于演示用量展示，
// 不代表真实计费。
func estimateTokens(messages []Message) int {
	total := 0
	for _, message := range messages {
		total += len([]rune(message.Content))
	}
	return total/4 + 1
}

// streamPiece 把一段文本按每次 4 个字符切片发送到通道，模拟流式输出。
// field 为 reasoning 时写入推理片段，否则写入正文片段；generated 累加已发送
// 字符数用于估算输出用量。上下文取消时返回 false。
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

// emitToolCall 发送一个工具调用片段。上下文取消时返回 false。
func emitToolCall(ctx context.Context, channel chan<- Chunk, call ToolCall) bool {
	select {
	case channel <- Chunk{ToolCall: &call}:
		return true
	case <-ctx.Done():
		return false
	}
}

// sleep 等待指定时长；上下文取消时提前返回 false。
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

// 编译期断言：MockClient 必须实现 Client 接口。
var _ Client = (*MockClient)(nil)
