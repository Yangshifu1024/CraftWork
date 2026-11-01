// 本文件负责首轮结束后的模型自动命名。
//
// 标题生成是独立的、尽力而为的小请求：使用当前运行实际使用的模型，设置
// 独立超时；超时、报错或返回空文本时退回本地首条消息提取，不阻塞本轮回复。

package agent

import (
	"context"
	"log/slog"
	"strings"
	"time"

	"craftwork/internal/events"
	"craftwork/internal/model"
	"craftwork/internal/store"
	"craftwork/internal/title"
)

// titleRequestTimeout 是标题模型调用的最长等待时间。超时后使用本地降级标题。
var titleRequestTimeout = 8 * time.Second

// titleMaxTokens 限制标题请求的输出长度，避免推理模型消耗过多 token。
const titleMaxTokens = 128

// titleSystemPrompt 是标题生成请求的系统提示词。
const titleSystemPrompt = `你是会话标题生成器。根据用户的第一条请求，生成一个概括任务的中文短标题。

要求：
- 不超过 20 个汉字；
- 不要引号、书名号、句号或“标题：”前缀；
- 只输出标题本身，不要解释。`

// autoTitle 与第一轮模型请求并行生成标题，并广播线程更新。
//
// 只有仍允许自动命名且标题仍为“新对话”的线程才会进入。模型调用失败、
// 超时或返回空文本时，使用首条用户消息提取降级标题。
func (r *Runner) autoTitle(thread store.Thread, modelRef string) {
	r.mu.Lock()
	if r.titlePending[thread.ID] {
		r.mu.Unlock()
		return
	}
	r.titlePending[thread.ID] = true
	r.mu.Unlock()
	defer func() {
		r.mu.Lock()
		delete(r.titlePending, thread.ID)
		r.mu.Unlock()
	}()

	// 传入的线程可能是运行开始时的快照；重新读取，避免覆盖刚刚完成的手动改名。
	current, err := r.db.GetThread(thread.ID)
	if err != nil || !current.TitleAuto || current.Title != title.Default {
		return
	}
	userText, attachments := r.firstUserTurn(current.ID)
	if strings.TrimSpace(userText) == "" && len(attachments) == 0 {
		return
	}

	var generated string
	var usage *model.Usage
	// 纯附件消息没有可读文本，直接使用文件名降级，避免无意义的模型请求。
	if strings.TrimSpace(userText) != "" {
		ctx, cancel := context.WithTimeout(context.Background(), titleRequestTimeout)
		defer cancel()
		generated, usage = r.generateTitle(ctx, modelRef, userText)
		if usage != nil {
			// 标题调用同样消耗 token，计入用量与费用，保持统计透明。
			r.recordUsage(*current, "", modelRef, usage)
		}
	}
	if generated == "" {
		generated = title.Derive(userText, attachments)
	}
	if generated == "" {
		return
	}

	changed, err := r.db.AutoTitleThread(current.ID, generated, title.Default)
	if err != nil {
		slog.Warn("自动命名线程失败", "thread", current.ID, "err", err)
		return
	}
	if !changed {
		return
	}
	updated, err := r.db.GetThread(current.ID)
	if err != nil {
		return
	}
	r.records.Emit(events.TypeThreadUpdated, current.ID, "", map[string]any{
		"threadId": current.ID,
		"mode":     updated.Mode,
		"thread": map[string]any{
			"id":        updated.ID,
			"title":     updated.Title,
			"updatedAt": updated.UpdatedAt,
		},
	})
}

// generateTitle 调用当前模型生成标题。失败或超时返回空字符串，由调用方降级。
func (r *Runner) generateTitle(ctx context.Context, modelRef, userText string) (string, *model.Usage) {
	client, err := r.models.Client(modelRef)
	if err != nil {
		return "", nil
	}
	_, info, ok := model.Resolve(modelRef)
	if !ok {
		return "", nil
	}
	stream, err := client.Stream(ctx, model.Request{
		Model:     info.ID,
		System:    titleSystemPrompt,
		Messages:  []model.Message{{Role: "user", Content: titleTranscript(userText)}},
		MaxTokens: titleMaxTokens,
	})
	if err != nil {
		return "", nil
	}
	var builder strings.Builder
	var usage *model.Usage
	for {
		select {
		case <-ctx.Done():
			// 即使适配器没有主动关闭通道，超时也必须立即走降级路径。
			return "", usage
		case chunk, ok := <-stream:
			if !ok {
				if ctx.Err() != nil {
					return "", usage
				}
				return sanitizeModelTitle(builder.String()), usage
			}
			if chunk.Err != nil {
				return "", usage
			}
			builder.WriteString(chunk.Text)
			if chunk.Usage != nil {
				usage = chunk.Usage
			}
		}
	}
}

// titleTranscript 拼出标题请求的输入，限制长度避免标题请求本身过大。
func titleTranscript(userText string) string {
	return "用户：" + truncateRunes(strings.TrimSpace(userText), 1500)
}

// sanitizeModelTitle 清洗模型输出，并限制在 20 个字符内。
func sanitizeModelTitle(raw string) string {
	value := title.Normalize(raw)
	for _, prefix := range []string{"标题：", "标题:", "Title:", "title:"} {
		if strings.HasPrefix(value, prefix) {
			value = strings.TrimSpace(strings.TrimPrefix(value, prefix))
			value = title.Normalize(value)
			break
		}
	}
	if value == "" || !title.HasContent(value) {
		return ""
	}
	return title.Limit(value)
}

// firstUserTurn 从事件快照中读取首条用户消息的文本与附件。
func (r *Runner) firstUserTurn(threadID string) (string, []string) {
	snapshot, err := r.db.Snapshot(threadID)
	if err != nil {
		return "", nil
	}
	for _, message := range snapshot.Messages {
		if message.Role != "user" {
			continue
		}
		var text strings.Builder
		var attachments []string
		for _, part := range message.Parts {
			if part.Kind != "text" {
				continue
			}
			data, _ := part.Data.(map[string]any)
			if value, ok := data["text"].(string); ok {
				text.WriteString(value)
			}
			switch list := data["attachments"].(type) {
			case []any:
				for _, item := range list {
					if path, ok := item.(string); ok {
						attachments = append(attachments, path)
					}
				}
			case []string:
				attachments = append(attachments, list...)
			}
		}
		if text.Len() > 0 || len(attachments) > 0 {
			return text.String(), attachments
		}
	}
	return "", nil
}
