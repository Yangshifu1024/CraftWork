// 工作区服务的内置临时目录行为：ensure 幂等、内置目录的守卫拒绝，
// 以及视图字段 purpose 的透传。

package service

import (
	"os"
	"path/filepath"
	"testing"

	"craftwork/internal/events"
	"craftwork/internal/record"
	"craftwork/internal/store"
)

// newWorkspaceService 搭一个带内存总线的最小工作区服务，供测试使用。
func newWorkspaceService(t *testing.T) (*store.DB, *WorkspaceService) {
	t.Helper()
	db, err := store.Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	bus := events.NewBus(func(string, any) {})
	t.Cleanup(bus.Close)
	service := NewWorkspaceService(db, bus, "")
	return db, service
}

func TestEnsureConversationWorkspaceIdempotent(t *testing.T) {
	db, _ := newWorkspaceService(t)
	dataDir := t.TempDir()

	first, err := EnsureConversationWorkspace(db, dataDir)
	if err != nil {
		t.Fatal(err)
	}
	// 目录真实存在且用途、信任固定。
	info, err := os.Stat(first.Path)
	if err != nil || !info.IsDir() {
		t.Fatalf("临时会话目录应被创建: %v", err)
	}
	if first.Purpose != store.WorkspacePurposeConversation {
		t.Fatalf("用途应为 conversation，实际 %q", first.Purpose)
	}
	if first.Trust != "trusted" {
		t.Fatalf("内置目录应固定为 trusted，实际 %q", first.Trust)
	}

	// 再次 ensure：沿用同一条记录，不新建、不刷新最近打开时间。
	second, err := EnsureConversationWorkspace(db, dataDir)
	if err != nil {
		t.Fatal(err)
	}
	if second.ID != first.ID || second.LastOpenedAt != first.LastOpenedAt {
		t.Fatalf("ensure 应幂等: first=%+v second=%+v", first, second)
	}
}

func TestConversationWorkspaceGuards(t *testing.T) {
	db, service := newWorkspaceService(t)
	conversation, err := EnsureConversationWorkspace(db, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}

	// 内置目录不能移除、不能改信任。
	if err := service.Remove(conversation.ID); err == nil {
		t.Fatal("移除内置临时目录应被拒绝")
	}
	if err := service.SetTrust(conversation.ID, "restricted"); err == nil {
		t.Fatal("修改内置临时目录信任级别应被拒绝")
	}

	// 内置目录路径不能作为项目工作区重复添加。
	if _, err := service.Add(conversation.Path, "trusted"); err == nil {
		t.Fatal("把内置目录添加为项目工作区应被拒绝")
	}

	// 列表视图带用途字段，前端据此分组。
	workspaces, err := service.List()
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, workspace := range workspaces {
		if workspace.ID == conversation.ID {
			found = true
			if workspace.Purpose != store.WorkspacePurposeConversation {
				t.Fatalf("视图用途应为 conversation，实际 %q", workspace.Purpose)
			}
		}
	}
	if !found {
		t.Fatal("内置临时目录应出现在工作区列表中")
	}
}

func TestCreateRejectsMissingWorkspace(t *testing.T) {
	db, _ := newWorkspaceService(t)
	if _, err := EnsureConversationWorkspace(db, t.TempDir()); err != nil {
		t.Fatal(err)
	}
	bus := events.NewBus(func(string, any) {})
	t.Cleanup(bus.Close)
	threads := NewThreadService(db, record.New(db, bus))

	if _, err := threads.Create("code", "ws_missing", ""); err == nil {
		t.Fatal("绑定不存在工作区的会话应被拒绝")
	}
	// 空工作区仍允许（无工作区的历史语义），存在的工作区正常创建。
	if _, err := threads.Create("code", "", ""); err != nil {
		t.Fatalf("空工作区创建会话不应报错: %v", err)
	}
}
