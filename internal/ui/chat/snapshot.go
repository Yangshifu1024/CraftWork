// snapshot.go 把 store 的快照消息换算成聊天列的消息结构：载荷归一化为
// map（dataOf），终态字段直接透传。快照是持久化事实，事件合并以它接管。

package chat

import (
	"github.com/yangshifu1024/craftwork/internal/store"
)

// messageFromStore 把 store.Message 换算成聊天列 Message。
func messageFromStore(m *store.Message, threadID string) *Message {
	parts := make([]Part, 0, len(m.Parts))
	for _, p := range m.Parts {
		parts = append(parts, Part{
			ID:   p.ID,
			Kind: p.Kind,
			Seq:  p.Seq,
			Data: dataOf(p.Data),
		})
	}
	status := m.Status
	if status == "" {
		status = StatusComplete
	}
	return &Message{
		ID:             m.ID,
		ThreadID:       threadID,
		Role:           m.Role,
		Status:         status,
		CreatedAt:      m.CreatedAt,
		WorkDurationMs: m.WorkDurationMs,
		Parts:          parts,
	}
}

// threadInfoFromSnapshot 把快照里的会话属性换算成聊天列的会话信息。
// store.Thread 只带累计 tokens；上下文占用与速度由 usage.updated 事件
// 覆盖（覆盖语义），模型显示名由 Composer 按引用查模型目录。
func threadInfoFromSnapshot(snapshot *store.Snapshot) ThreadInfo {
	t := snapshot.Thread
	return ThreadInfo{
		ID:      t.ID,
		Title:   t.Title,
		Mode:    t.Mode,
		ModelID: t.ModelID,
		Status:  t.Status,
		Tokens:  t.Tokens,
	}
}
