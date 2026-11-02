package agent

import (
	"context"
	"fmt"
	"sync/atomic"
	"testing"
	"time"

	"craftwork/internal/model"
	"craftwork/internal/store"
)

// steerGatedClient 第一次请求进入后等待 gate 放行，让测试能在运行途中
// 稳定地入队引导。toolFirst 决定第一次请求返回工具调用还是收尾文本；
// count 是请求计数（waitFor 用它等待接力，不受读取排空影响），
// requests 记录每次到达的模型请求，供最终断言引导是否并入历史。
type steerGatedClient struct {
	entered   chan struct{}
	gate      chan struct{}
	requests  chan model.Request
	count     atomic.Int32
	toolFirst bool
	step      int
}

func (c *steerGatedClient) Stream(_ context.Context, request model.Request) (<-chan model.Chunk, error) {
	c.step++
	c.count.Add(1)
	select {
	case c.requests <- request:
	default:
	}
	channel := make(chan model.Chunk, 2)
	go func() {
		defer close(channel)
		if c.step == 1 {
			close(c.entered)
			<-c.gate
		}
		if c.toolFirst && c.step == 1 {
			channel <- model.Chunk{ToolCall: &model.ToolCall{
				ID:   "call_1",
				Name: "fs_list",
				Args: map[string]any{"path": "."},
			}}
		} else {
			channel <- model.Chunk{Text: "完成"}
		}
		channel <- model.Chunk{Usage: &model.Usage{}, Done: true}
	}()
	return channel, nil
}

// approvalSteerClient 第一次请求返回 fs_write 工具调用（触发审批），
// 之后返回收尾文本；requests 记录每次请求，count 为请求计数。
type approvalSteerClient struct {
	requests chan model.Request
	count    atomic.Int32
	step     int
}

func (c *approvalSteerClient) Stream(_ context.Context, request model.Request) (<-chan model.Chunk, error) {
	c.step++
	c.count.Add(1)
	select {
	case c.requests <- request:
	default:
	}
	channel := make(chan model.Chunk, 2)
	go func() {
		defer close(channel)
		if c.step == 1 {
			channel <- model.Chunk{ToolCall: &model.ToolCall{
				ID:   "call_1",
				Name: "fs_write",
				Args: map[string]any{"path": "note.txt", "content": "hello"},
			}}
		} else {
			channel <- model.Chunk{Text: "完成"}
		}
		channel <- model.Chunk{Usage: &model.Usage{}, Done: true}
	}()
	return channel, nil
}

// readRequests 读取请求通道里已缓存的所有请求。
func readRequests(ch chan model.Request) []model.Request {
	var result []model.Request
	for {
		select {
		case request := <-ch:
			result = append(result, request)
		default:
			return result
		}
	}
}

// waitFor 轮询等待条件成立。接力运行会先短暂回到 idle 再开新 run，
// 所以涉及接力的断言必须等第二个请求真正到达，而不能只看会话状态。
func waitFor(t *testing.T, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("等待条件超时")
}

// assertLastMessageIsSteer 断言请求历史的末尾是指定文本的引导消息。
func assertLastMessageIsSteer(t *testing.T, request model.Request, text string) {
	t.Helper()
	if len(request.Messages) == 0 {
		t.Fatal("请求不应没有消息")
	}
	last := request.Messages[len(request.Messages)-1]
	if last.Role != "user" || last.Content != text {
		t.Fatalf("请求末尾应是引导消息 %q: %+v", text, last)
	}
}

// TestRunnerSteerDeliveredBeforeNextModelCall 验证投递点：工具执行期间入队的
// 引导，在下一次模型请求前并入历史末尾，随本轮请求生效。
func TestRunnerSteerDeliveredBeforeNextModelCall(t *testing.T) {
	client := &steerGatedClient{
		entered:   make(chan struct{}),
		gate:      make(chan struct{}),
		requests:  make(chan model.Request, 8),
		toolFirst: true,
	}
	h := newHarnessWithFactory(t, fakeFactory{client: client})
	h.start(t)
	<-client.entered

	if ok, err := h.runner.TrySteer("th_1", SteerItem{MessageID: "msg_steer", Text: "优先使用方案 B"}); err != nil || !ok {
		t.Fatalf("运行中入队引导应成功: ok=%v err=%v", ok, err)
	}
	close(client.gate)
	waitStatus(t, h.db, "th_1", "idle")

	requests := readRequests(client.requests)
	if len(requests) != 2 {
		t.Fatalf("应恰好请求模型 2 次，实际 %d", len(requests))
	}
	assertLastMessageIsSteer(t, requests[1], "优先使用方案 B")
}

// TestRunnerSteerFollowupRunAfterComplete 验证接力：引导在运行收尾前入队时，
// 本次运行正常结束后自动开一个接力 run，把引导作为新的用户输入执行。
func TestRunnerSteerFollowupRunAfterComplete(t *testing.T) {
	client := &steerGatedClient{
		entered:  make(chan struct{}),
		gate:     make(chan struct{}),
		requests: make(chan model.Request, 8),
	}
	h := newHarnessWithFactory(t, fakeFactory{client: client})
	h.start(t)
	<-client.entered

	if ok, err := h.runner.TrySteer("th_1", SteerItem{MessageID: "msg_steer", Text: "再补充一点"}); err != nil || !ok {
		t.Fatalf("运行中入队引导应成功: ok=%v err=%v", ok, err)
	}
	close(client.gate)
	waitFor(t, func() bool { return client.count.Load() == 2 })
	waitStatus(t, h.db, "th_1", "idle")

	requests := readRequests(client.requests)
	assertLastMessageIsSteer(t, requests[1], "再补充一点")

	// 接力产生第二条 assistant 消息：每个 run 在事件表里对应一条。
	snapshot, err := h.db.Snapshot("th_1")
	if err != nil {
		t.Fatal(err)
	}
	assistants := 0
	for _, message := range snapshot.Messages {
		if message.Role == "assistant" {
			assistants++
		}
	}
	if assistants != 2 {
		t.Fatalf("接力后应有两条 assistant 消息，实际 %d", assistants)
	}
}

// TestRunnerSteerDeliveredAfterApproval 验证审批挂起期间入队的引导，
// 在审批决定后随下一次模型请求生效。
func TestRunnerSteerDeliveredAfterApproval(t *testing.T) {
	client := &approvalSteerClient{requests: make(chan model.Request, 8)}
	h := newHarnessWithFactory(t, fakeFactory{client: client})
	h.start(t)

	approval := waitApproval(t, h.db, "th_1")
	if ok, err := h.runner.TrySteer("th_1", SteerItem{MessageID: "msg_steer", Text: "写完后顺便列出目录"}); err != nil || !ok {
		t.Fatalf("审批挂起时入队引导应成功: ok=%v err=%v", ok, err)
	}
	if err := h.approvals.Resolve(approval.ID, Decision{Allow: true, Scope: "once"}); err != nil {
		t.Fatal(err)
	}
	waitStatus(t, h.db, "th_1", "idle")

	requests := readRequests(client.requests)
	if len(requests) != 2 {
		t.Fatalf("应恰好请求模型 2 次，实际 %d", len(requests))
	}
	assertLastMessageIsSteer(t, requests[1], "写完后顺便列出目录")
}

// TestRunnerSteerCancelDiscardsQueue 验证取消：未投递的引导随运行收尾丢弃，
// 不产生接力运行，注销后再次入队失败（服务层据此回退为普通发送）。
func TestRunnerSteerCancelDiscardsQueue(t *testing.T) {
	client := &steerGatedClient{
		entered:   make(chan struct{}),
		gate:      make(chan struct{}),
		requests:  make(chan model.Request, 8),
		toolFirst: true,
	}
	h := newHarnessWithFactory(t, fakeFactory{client: client})
	h.start(t)
	<-client.entered

	if ok, err := h.runner.TrySteer("th_1", SteerItem{MessageID: "msg_steer", Text: "不该被投递"}); err != nil || !ok {
		t.Fatalf("入队引导应成功: ok=%v err=%v", ok, err)
	}
	h.runner.Cancel("th_1")
	close(client.gate)
	waitStatus(t, h.db, "th_1", "idle")

	// 收尾 goroutine 与测试并发：轮询到注销完成（入队开始失败）为止。
	waitFor(t, func() bool {
		ok, _ := h.runner.TrySteer("th_1", SteerItem{Text: "x"})
		return !ok
	})
	if requests := readRequests(client.requests); len(requests) != 1 {
		t.Fatalf("取消后不应有后续模型请求，实际 %d", len(requests))
	}
}

// TestTrySteerRequiresRunning 验证没有在途运行时入队失败。
func TestTrySteerRequiresRunning(t *testing.T) {
	h := newHarness(t)
	if ok, err := h.runner.TrySteer("th_1", SteerItem{Text: "x"}); ok || err != nil {
		t.Fatalf("无运行时入队应返回 (false, nil): ok=%v err=%v", ok, err)
	}
}

// TestTrySteerQueueLimit 验证队列上限：超过 maxSteerQueue 条后明确拒绝。
func TestTrySteerQueueLimit(t *testing.T) {
	client := &steerGatedClient{
		entered:  make(chan struct{}),
		gate:     make(chan struct{}),
		requests: make(chan model.Request, 8),
	}
	h := newHarnessWithFactory(t, fakeFactory{client: client})
	h.start(t)
	<-client.entered

	for i := 0; i < maxSteerQueue; i++ {
		item := SteerItem{MessageID: fmt.Sprintf("msg_s_%d", i), Text: fmt.Sprintf("引导 %d", i)}
		if ok, err := h.runner.TrySteer("th_1", item); err != nil || !ok {
			t.Fatalf("第 %d 条引导应入队成功: ok=%v err=%v", i+1, ok, err)
		}
	}
	if ok, err := h.runner.TrySteer("th_1", SteerItem{Text: "超出"}); ok || err == nil {
		t.Fatalf("超出上限应拒绝: ok=%v err=%v", ok, err)
	}
	close(client.gate)
	// 运行收尾后接力 run 会把 10 条引导全部并入，等它真正跑完。
	waitFor(t, func() bool { return client.count.Load() == 2 })
	waitStatus(t, h.db, "th_1", "idle")
}

// TestHistorySteerDeduplication 验证接力运行的去重：引导随事件表进入
// 重建历史后，投递时按消息 ID 跳过，不会重复并入。
func TestHistorySteerDeduplication(t *testing.T) {
	h := newHarness(t)
	if _, err := h.db.AppendEvent(&store.EventRecord{
		ID: "p_steer", ThreadID: "th_1", MessageID: "msg_steer",
		Kind: "user_message", Payload: map[string]any{"text": "接力前已落库的引导"},
	}); err != nil {
		t.Fatal(err)
	}
	history, _, err := h.runner.history(h.thread, h.workspace)
	if err != nil {
		t.Fatal(err)
	}
	if !historyHasMessage(history, "msg_steer") {
		t.Fatal("重建历史应包含已落库的引导消息")
	}
	if historyHasMessage(history, "msg_other") {
		t.Fatal("不存在的信息不应命中去重")
	}
}
