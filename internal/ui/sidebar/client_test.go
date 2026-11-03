package sidebar

// 13 号票：新建会话后立即切换的服务接线测试。用真实 store + service
// 走 Client 的异步编排（CreateTempThread 的完整链路），断言新建完成后
// 聊天列联动（Actions.SelectThread）被以新会话 Id 调用——旧版行为是
// 创建即切换（threads.ts create 写 activeID 并清空消息区）。

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/policy"
	"github.com/yangshifu1024/craftwork/internal/record"
	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/store"
)

// newTestClient 起一套隔离目录下的真实服务接线：Deliver 同步执行并在
// 每次投递后发信号，测试用信号等待异步动作完成（channel 建立先后发生
// 顺序，状态断言无数据竞争）。返回的 *store.DB 供用例补种工作区。
func newTestClient(t *testing.T) (*Sidebar, *Client, *store.DB, chan struct{}) {
	t.Helper()
	root := t.TempDir()
	db, err := store.Open(filepath.Join(root, "sidebar.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	// 内置临时目录：CreateTempThread 的归属目标。
	if err := db.UpsertWorkspace(&store.Workspace{
		ID: "conv", Name: "临时会话", Path: filepath.Join(root, "conv"),
		Trust: "trusted", Purpose: store.WorkspacePurposeConversation,
		CreatedAt: store.NowMillis(), LastOpenedAt: store.NowMillis(),
	}); err != nil {
		t.Fatal(err)
	}
	bus := events.NewBus(func(events.Batch) {})
	t.Cleanup(bus.Close)
	recorder := record.New(db, bus)

	sb := NewSidebar()
	done := make(chan struct{}, 8)
	cl := NewClient(sb, ClientDeps{
		Threads:    service.NewThreadService(db, recorder),
		Workspaces: service.NewWorkspaceService(db, bus, filepath.Join(root, "backups")),
		Settings:   service.NewSettingsService(db, bus, policy.New(policy.LevelGraded)),
		Audit:      service.NewAuditService(db),
		Deliver: func(fn func()) {
			fn()
			done <- struct{}{}
		},
		Toast: func(string) {},
	})
	return sb, cl, db, done
}

// waitDone 等一次投递信号，超时直接终止用例。
func waitDone(t *testing.T, done chan struct{}) {
	t.Helper()
	select {
	case <-done:
	case <-time.After(10 * time.Second):
		t.Fatal("等待异步动作完成超时")
	}
}

func TestClientCreateSelectsNewThread(t *testing.T) {
	sb, cl, _, done := newTestClient(t)
	var selected []string
	sb.SetActions(&Actions{SelectThread: func(id string) { selected = append(selected, id) }})

	cl.CreateTempThread()
	waitDone(t, done)

	id := sb.ActiveID()
	if id == "" {
		t.Fatal("新建后侧栏应选中新会话")
	}
	if !sb.HasThread(id) {
		t.Errorf("选中的 %q 应在列表里", id)
	}
	if len(selected) != 1 || selected[0] != id {
		t.Errorf("聊天列联动 SelectThread = %v, want [%s]（新建应立即切换）", selected, id)
	}
}

func TestClientCreateInSelectsNewThread(t *testing.T) {
	sb, cl, db, done := newTestClient(t)
	if err := db.UpsertWorkspace(&store.Workspace{
		ID: "ws-a", Name: "项目 A", Path: filepath.Join(t.TempDir(), "ws-a"),
		Trust: "trusted", Purpose: store.WorkspacePurposeProject,
		CreatedAt: store.NowMillis(), LastOpenedAt: store.NowMillis(),
	}); err != nil {
		t.Fatal(err)
	}
	var selected []string
	sb.SetActions(&Actions{SelectThread: func(id string) { selected = append(selected, id) }})

	cl.CreateThreadIn("ws-a")
	waitDone(t, done)

	if id := sb.ActiveID(); id == "" || len(selected) != 1 || selected[0] != id {
		t.Errorf("项目下新建应立即切换: active=%q selected=%v", sb.ActiveID(), selected)
	}
}
