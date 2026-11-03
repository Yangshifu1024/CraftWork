// reducer 单测对齐 frontend/src/stores/threadReducer.test.ts 的用例语义
//（每个 it 一个 Go 用例，断言一致），另补 Go 侧的终态工时回填与部件
// upsert 位置保持用例。

package chat

import (
	"testing"

	"github.com/yangshifu1024/craftwork/internal/events"
)

// baseThread 与 TS 用例的 thread 常量同构。
func baseThread() ThreadInfo {
	return ThreadInfo{
		ID: "th", Mode: "code", Title: "测试会话",
		ModelID: "openai/gpt-5.1", ModelName: "GPT-5.1",
		Status: ThreadStatusIdle,
	}
}

func baseState() Slice {
	return Slice{
		Messages:         nil,
		Threads:          map[string]ThreadInfo{"th": baseThread()},
		PendingApprovals: nil,
	}
}

func envelope(eventType string, data any, seq int64, threadID string) *events.Envelope {
	return &events.Envelope{Seq: seq, Type: eventType, ThreadID: threadID, RunID: "run_1", TS: 0, Data: data}
}

func batch(list ...*events.Envelope) events.Batch {
	return events.Batch{V: 1, ThreadID: "th", Events: list}
}

// mapData 构造 map 载荷，等价 TS 测试里的对象字面量。
func mapData(kv ...any) map[string]any {
	m := map[string]any{}
	for i := 0; i+1 < len(kv); i += 2 {
		m[kv[i].(string)] = kv[i+1]
	}
	return m
}

func findMessage(t *testing.T, messages []*Message, id string) *Message {
	t.Helper()
	for _, m := range messages {
		if m.ID == id {
			return m
		}
	}
	t.Fatalf("找不到消息 %s", id)
	return nil
}

func firstPart(m *Message) Part { return m.Parts[0] }

// 对齐「合并文本增量并在完成事件后保留完整文本」。
func TestReduceMergesTextDeltasAndCompletes(t *testing.T) {
	state := baseState()
	started := Reduce(state, batch(envelope(events.TypeRunStarted, mapData("messageId", "msg_1"), 1, "th")), "th")
	if !started.Running || started.RunningMessageID != "msg_1" {
		t.Fatalf("run.started 后应为运行中: %+v", started.Slice)
	}

	delta1 := Reduce(started.Slice, batch(envelope(events.TypeTextDelta, events.TextDelta{PartID: "p_text", Text: "你"}, 2, "th")), "th")
	delta2 := Reduce(delta1.Slice, batch(envelope(events.TypeTextDelta, events.TextDelta{PartID: "p_text", Text: "好"}, 3, "th")), "th")
	completed := Reduce(delta2.Slice, batch(envelope(events.TypeTextCompleted, mapData("partId", "p_text", "text", "你好"), 4, "th")), "th")
	finished := Reduce(completed.Slice, batch(envelope(events.TypeRunCompleted, mapData("status", "done"), 5, "th")), "th")

	msg := findMessage(t, finished.Messages, "msg_1")
	if got := strOf(firstPart(msg).Data, "text"); got != "你好" {
		t.Fatalf("完成事件后应为完整文本，得到 %q", got)
	}
	if msg.Status != StatusComplete {
		t.Fatalf("消息终态应为 complete，得到 %q", msg.Status)
	}
	if finished.Running || finished.RunningMessageID != "" {
		t.Fatalf("运行结束 后不应再运行中: %+v", finished.Slice)
	}
	if finished.Threads["th"].Status != ThreadStatusIdle {
		t.Fatalf("会话状态应为 idle，得到 %q", finished.Threads["th"].Status)
	}
	if !finished.RefreshNeeded {
		t.Fatal("运行结束应标记需要刷新列表")
	}
}

// 对齐「审批请求进入待审批列表并切换会话状态」。
func TestReduceApprovalQueueAndStatus(t *testing.T) {
	started := Reduce(baseState(), batch(envelope(events.TypeRunStarted, mapData("messageId", "msg_1"), 1, "th")), "th")
	requested := Reduce(started.Slice, batch(envelope(events.TypeApprovalRequested, mapData(
		"partId", "p_ap", "approvalId", "ap_1", "tool", "fs_write", "risk", "R1",
		"title", "写入文件", "target", "note.txt", "status", "waiting", "allowPermanent", true,
	), 2, "th")), "th")

	if len(requested.PendingApprovals) != 1 {
		t.Fatalf("待审批队列应有 1 条，得到 %d", len(requested.PendingApprovals))
	}
	if requested.Threads["th"].Status != ThreadStatusAwaiting {
		t.Fatalf("会话状态应为 awaiting，得到 %q", requested.Threads["th"].Status)
	}
	msg := findMessage(t, requested.Messages, "msg_1")
	if firstPart(msg).Kind != PartApproval {
		t.Fatalf("首个部件应为 approval，得到 %q", firstPart(msg).Kind)
	}

	resolved := Reduce(requested.Slice, batch(envelope(events.TypeApprovalResolved, mapData(
		"partId", "p_ap", "approvalId", "ap_1", "status", "approved", "decision", "allow", "scope", "session",
	), 3, "th")), "th")

	if len(resolved.PendingApprovals) != 0 {
		t.Fatalf("决定后待审批队列应为空，得到 %d", len(resolved.PendingApprovals))
	}
	resolvedMsg := findMessage(t, resolved.Messages, "msg_1")
	if got := strOf(firstPart(resolvedMsg).Data, "status"); got != "approved" {
		t.Fatalf("审批部件状态应为 approved，得到 %q", got)
	}
	if resolved.Threads["th"].Status != ThreadStatusRunning {
		t.Fatalf("决定后会话应回到 running，得到 %q", resolved.Threads["th"].Status)
	}
}

// 对齐「usage.updated 累加会话 tokens」。
func TestReduceUsageAccumulatesTokens(t *testing.T) {
	next := Reduce(baseState(), batch(envelope(events.TypeUsageUpdated, mapData("inputTokens", 10, "outputTokens", 5), 1, "th")), "th")
	if next.Threads["th"].Tokens != 15 {
		t.Fatalf("tokens 应累加为 15，得到 %d", next.Threads["th"].Tokens)
	}
}

// 对齐「usage.updated 覆盖上下文占用与速度，缺字段时保持原值」。
func TestReduceUsageContextOverlay(t *testing.T) {
	first := Reduce(baseState(), batch(envelope(events.TypeUsageUpdated, mapData(
		"inputTokens", 100, "outputTokens", 20, "contextTokens", 120, "cacheTokens", 90, "tokensPerSecond", 42.5,
	), 1, "th")), "th")
	th := first.Threads["th"]
	if th.ContextTokens != 120 || th.ContextCacheTokens != 90 || th.ContextSpeed != 42.5 {
		t.Fatalf("第一轮应覆盖上下文字段: %+v", th)
	}

	// 第二轮是不带上下文字段的 sidecar 请求：tokens 照常累加，水位不被压低。
	second := Reduce(first.Slice, batch(envelope(events.TypeUsageUpdated, mapData("inputTokens", 30, "outputTokens", 5), 2, "th")), "th")
	th = second.Threads["th"]
	if th.Tokens != 155 || th.ContextTokens != 120 || th.ContextSpeed != 42.5 {
		t.Fatalf("sidecar 请求不应覆盖上下文水位: %+v", th)
	}

	// 第三轮主对话再次携带字段：覆盖为最新一轮。
	third := Reduce(second.Slice, batch(envelope(events.TypeUsageUpdated, mapData(
		"inputTokens", 200, "outputTokens", 50, "contextTokens", 250, "cacheTokens", 180, "tokensPerSecond", 60,
	), 3, "th")), "th")
	th = third.Threads["th"]
	if th.ContextTokens != 250 || th.ContextCacheTokens != 180 || th.ContextSpeed != 60 {
		t.Fatalf("第三轮应覆盖为最新值: %+v", th)
	}
}

// 对齐「其他会话的 thread.updated 只标记刷新，不修改当前会话」。
func TestReduceOtherThreadUpdateOnlyRefreshes(t *testing.T) {
	result := Reduce(baseState(), batch(envelope(events.TypeThreadUpdated, mapData("threadId", "other", "status", ThreadStatusIdle), 1, "other")), "th")
	if !result.RefreshNeeded {
		t.Fatal("其他会话更新应标记刷新")
	}
	if result.Threads["th"].Status != ThreadStatusIdle {
		t.Fatalf("当前会话状态不应被改，得到 %q", result.Threads["th"].Status)
	}
}

// 对齐「终端与全局事件不修改会话状态」。
func TestReduceTerminalAndGlobalEventsNoop(t *testing.T) {
	result := Reduce(baseState(), batch(
		envelope(events.TypeTerminalOutput, events.TerminalDelta{SessionID: "s1", Data: "x"}, 1, "terminal:s1"),
		envelope(events.TypeSettingsUpdated, mapData("theme", "light"), 2, ""),
		envelope(events.TypeWorkspaceUpdated, mapData("id", "ws"), 3, ""),
		envelope(events.TypeContextCompressed, mapData("summary", "摘要"), 4, "th"),
	), "th")
	if len(result.Messages) != 0 || result.Threads["th"].Tokens != 0 || result.RefreshNeeded {
		t.Fatalf("终端与全局事件不应修改会话状态: %+v", result)
	}
}

// 对齐「user.message 插入待送达的引导消息并按 id 去重」。
func TestReduceSteerMessageDedup(t *testing.T) {
	started := Reduce(baseState(), batch(envelope(events.TypeRunStarted, mapData("messageId", "msg_run"), 1, "th")), "th")
	steer := func(seq int64) *events.Envelope {
		return envelope(events.TypeUserMessage, mapData("messageId", "msg_steer", "text", "补充指令", "attachments", []any{}, "via", "steer"), seq, "th")
	}

	steered := Reduce(started.Slice, batch(steer(2)), "th")
	msg := findMessage(t, steered.Messages, "msg_steer")
	if msg.Role != RoleUser || !msg.SteerQueued {
		t.Fatalf("引导消息应为 user 且待送达: %+v", msg)
	}
	if got := strOf(firstPart(msg).Data, "text"); got != "补充指令" {
		t.Fatalf("引导文本应为「补充指令」，得到 %q", got)
	}
	if _, ok := firstPart(msg).Data["attachments"]; !ok {
		t.Fatal("引导部件应携带 attachments 字段")
	}

	// 同一条引导重复推送（多窗口、事件重放）按消息 id 去重。
	deduped := Reduce(steered.Slice, batch(steer(3)), "th")
	if len(deduped.Messages) != len(steered.Messages) {
		t.Fatalf("重复引导应去重，得到 %d 条", len(deduped.Messages))
	}
}

// 对齐「待送达标记在流式与工具事件到达时清除，审批事件不影响」。
func TestReduceSteerFlagClearedByActivity(t *testing.T) {
	started := Reduce(baseState(), batch(envelope(events.TypeRunStarted, mapData("messageId", "msg_run"), 1, "th")), "th")
	steered := Reduce(started.Slice, batch(envelope(events.TypeUserMessage, mapData(
		"messageId", "msg_steer", "text", "补充指令", "attachments", []any{}, "via", "steer",
	), 2, "th")), "th")

	// 审批对应的是引导之前已经发起的工具调用，不代表引导已投递。
	approval := Reduce(steered.Slice, batch(envelope(events.TypeApprovalRequested, mapData(
		"partId", "p_ap", "approvalId", "ap_1", "tool", "fs_write", "risk", "R1",
		"title", "写入", "target", "note.txt", "status", "waiting", "allowPermanent", true,
	), 3, "th")), "th")
	if msg := findMessage(t, approval.Messages, "msg_steer"); !msg.SteerQueued {
		t.Fatal("审批事件不应清除待送达标记")
	}

	// 流式增量到达，说明引导已被并入模型请求。
	delivered := Reduce(approval.Slice, batch(envelope(events.TypeTextDelta, events.TextDelta{PartID: "p_text", Text: "收到"}, 4, "th")), "th")
	if msg := findMessage(t, delivered.Messages, "msg_steer"); msg.SteerQueued {
		t.Fatal("流式增量应清除待送达标记")
	}
}

// 对齐「工具开始与运行终态同样清除待送达标记」。
func TestReduceSteerFlagClearedByToolAndFinal(t *testing.T) {
	steer := func(seq int64) *events.Envelope {
		return envelope(events.TypeUserMessage, mapData("messageId", "msg_steer", "text", "补充指令", "attachments", []any{}, "via", "steer"), seq, "th")
	}
	started := Reduce(baseState(), batch(envelope(events.TypeRunStarted, mapData("messageId", "msg_run"), 1, "th")), "th")
	steered := Reduce(started.Slice, batch(steer(2)), "th")
	tool := Reduce(steered.Slice, batch(envelope(events.TypeToolStarted, mapData(
		"partId", "p_tool", "toolCallId", "c1", "name", "fs_list", "args", mapData(),
		"risk", "R0", "status", "running", "summary", ".",
	), 3, "th")), "th")
	if msg := findMessage(t, tool.Messages, "msg_steer"); msg.SteerQueued {
		t.Fatal("工具开始应清除待送达标记")
	}

	// 取消后未投递的引导留在会话里，但不再有「即将投递」语义。
	steeredAgain := Reduce(started.Slice, batch(steer(2)), "th")
	canceled := Reduce(steeredAgain.Slice, batch(envelope(events.TypeRunCanceled, mapData("status", "canceled"), 3, "th")), "th")
	if msg := findMessage(t, canceled.Messages, "msg_steer"); msg.SteerQueued {
		t.Fatal("运行终态应清除待送达标记")
	}
}

// 对齐「未知事件类型被忽略」。
func TestReduceUnknownEventIgnored(t *testing.T) {
	result := Reduce(baseState(), batch(envelope("future.event", mapData("anything", true), 1, "th")), "th")
	if len(result.Messages) != 0 || result.Running {
		t.Fatalf("未知事件应被忽略: %+v", result.Slice)
	}
}

// 补充 Go 侧：运行终态把事件里的 durationMs 回填为消息工时。
func TestReduceBackfillsWorkDuration(t *testing.T) {
	started := Reduce(baseState(), batch(envelope(events.TypeRunStarted, mapData("messageId", "msg_1"), 1, "th")), "th")
	finished := Reduce(started.Slice, batch(envelope(events.TypeRunCompleted, mapData("durationMs", 4200), 2, "th")), "th")
	msg := findMessage(t, finished.Messages, "msg_1")
	if msg.WorkDurationMs != 4200 {
		t.Fatalf("工时应回填 4200ms，得到 %d", msg.WorkDurationMs)
	}
	// 缺时长时工时保持零值。
	started2 := Reduce(baseState(), batch(envelope(events.TypeRunStarted, mapData("messageId", "msg_2"), 1, "th")), "th")
	finished2 := Reduce(started2.Slice, batch(envelope(events.TypeRunCanceled, mapData("status", "canceled"), 2, "th")), "th")
	if msg2 := findMessage(t, finished2.Messages, "msg_2"); msg2.WorkDurationMs != 0 {
		t.Fatalf("无 durationMs 时工时应为 0，得到 %d", msg2.WorkDurationMs)
	}
}

// 补充 Go 侧：同一部件先 started 再 completed 时替换数据、位置不变。
func TestReduceUpsertKeepsPartPosition(t *testing.T) {
	started := Reduce(baseState(), batch(envelope(events.TypeRunStarted, mapData("messageId", "msg_1"), 1, "th")), "th")
	withReasoning := Reduce(started.Slice, batch(envelope(events.TypeReasoningDelta, events.TextDelta{PartID: "p_r", Text: "思考"}, 2, "th")), "th")
	completedTool := Reduce(withReasoning.Slice, batch(envelope(events.TypeToolStarted, mapData(
		"partId", "p_tool", "toolCallId", "c1", "name", "fs_list", "risk", "R0", "status", "running",
	), 3, "th")), "th")
	doneTool := Reduce(completedTool.Slice, batch(envelope(events.TypeToolCompleted, mapData(
		"partId", "p_tool", "toolCallId", "c1", "name", "fs_list", "risk", "R0", "status", "done", "output", "a.go",
	), 4, "th")), "th")

	msg := findMessage(t, doneTool.Messages, "msg_1")
	if len(msg.Parts) != 2 {
		t.Fatalf("应有 2 个部件，得到 %d", len(msg.Parts))
	}
	if msg.Parts[0].ID != "p_r" || msg.Parts[1].ID != "p_tool" {
		t.Fatalf("部件顺序应保持，得到 %v %v", msg.Parts[0].ID, msg.Parts[1].ID)
	}
	if strOf(msg.Parts[1].Data, "output") != "a.go" || strOf(msg.Parts[1].Data, "status") != "done" {
		t.Fatalf("完成事件应整体覆盖部件数据: %+v", msg.Parts[1].Data)
	}
}

// 补充 Go 侧：thread.updated 带整条会话时按 id 合并，两处取值都以 Id 命中。
func TestReduceThreadUpdatedFullMerge(t *testing.T) {
	state := baseState()
	state.Threads["other"] = ThreadInfo{ID: "other", Status: ThreadStatusRunning}
	result := Reduce(state, batch(envelope(events.TypeThreadUpdated, mapData("thread", mapData(
		"id", "th", "title", "新标题", "tokens", 42, "status", ThreadStatusIdle,
	)), 1, "th")), "th")
	th := result.Threads["th"]
	if th.Title != "新标题" || th.Tokens != 42 {
		t.Fatalf("整条会话应合并进表: %+v", th)
	}
	if result.Threads["other"].Status != ThreadStatusRunning {
		t.Fatal("其他会话不应被改动")
	}
}

// 补充 Go 侧：Reduce 不修改传入状态（纯函数语义，React 版靠不可变结构保证）。
func TestReduceDoesNotMutateInput(t *testing.T) {
	state := baseState()
	started := Reduce(state, batch(envelope(events.TypeRunStarted, mapData("messageId", "msg_1"), 1, "th")), "th")
	if state.Running || state.RunningMessageID != "" {
		t.Fatalf("Reduce 后原状态不应改变: %+v", state)
	}
	if len(state.Messages) != 0 {
		t.Fatalf("原状态消息列表不应改变: %d", len(state.Messages))
	}
	started.Messages[0].Parts = append(started.Messages[0].Parts, Part{ID: "later"})
	if len(state.Messages) != 0 {
		t.Fatal("新状态的消息指针共享不应影响旧状态切片")
	}
}
