// 工作区服务的内置临时目录行为：ensure 幂等、内置目录的守卫拒绝，
// 以及视图字段 purpose 的透传。

package service

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/record"
	"github.com/yangshifu1024/craftwork/internal/store"
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

func TestAddProjectMultiFolder(t *testing.T) {
	db, service := newWorkspaceService(t)
	// 内置临时目录先存在，验证 AddProject 的内置目录守卫。
	if _, err := EnsureConversationWorkspace(db, t.TempDir()); err != nil {
		t.Fatal(err)
	}
	primary := t.TempDir()
	additional := t.TempDir()
	// macOS 的 /var 是 /private/var 的别名：归一化后再断言绝对路径。
	if canonical, err := filepath.EvalSymlinks(primary); err == nil {
		primary = canonical
	}
	if canonical, err := filepath.EvalSymlinks(additional); err == nil {
		additional = canonical
	}
	conversation, err := EnsureConversationWorkspace(db, filepath.Join(t.TempDir(), "data"))
	if err != nil {
		t.Fatal(err)
	}

	// 空文件夹列表拒绝。
	if _, err := service.AddProject("", nil, "trusted"); err == nil {
		t.Fatal("没有源文件夹时应拒绝")
	}
	// 内置临时目录作为源文件夹拒绝。
	if _, err := service.AddProject("", []string{conversation.Path}, "trusted"); err == nil {
		t.Fatal("内置临时目录作为源文件夹应拒绝")
	}

	view, err := service.AddProject("", []string{primary, additional}, "trusted")
	if err != nil {
		t.Fatalf("创建项目失败: %v", err)
	}
	// 名称从主要目录推导，信任档位与主要目录正确。
	if view.Name != filepath.Base(primary) {
		t.Fatalf("名称应为主要目录名，实际 %q", view.Name)
	}
	if view.Trust != "trusted" || view.Path != primary {
		t.Fatalf("信任与主要目录不符: %+v", view)
	}
	if len(view.Folders) != 2 {
		t.Fatalf("应挂载两个源文件夹，实际 %d", len(view.Folders))
	}
	if !view.Folders[0].Primary || view.Folders[0].Path != primary {
		t.Fatalf("第一条应是主要目录: %+v", view.Folders[0])
	}
	if view.Folders[1].Primary {
		t.Fatal("第二条不应是主要目录")
	}

	// 重名自动追加序号。
	another := t.TempDir()
	renamed, err := service.AddProject(view.Name, []string{another}, "trusted")
	if err != nil {
		t.Fatalf("重名创建失败: %v", err)
	}
	if renamed.Name != view.Name+" (2)" {
		t.Fatalf("重名应追加序号，实际 %q", renamed.Name)
	}

	// 附加目录被其他项目占用时拒绝。
	if _, err := service.AddProject("third", []string{t.TempDir(), primary}, "trusted"); err == nil {
		t.Fatal("源文件夹已被其他工作区使用时应拒绝")
	}

	// 文件读写按 folderID 限定根。
	content := "hello project"
	if err := os.WriteFile(filepath.Join(additional, "note.txt"), []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
	file, err := service.ReadFile(view.ID, view.Folders[1].ID, "note.txt")
	if err != nil {
		t.Fatalf("按 folderID 读取失败: %v", err)
	}
	if file.Content != content || file.FolderID != view.Folders[1].ID {
		t.Fatalf("读取结果不符: %+v", file)
	}
	// 空 folderID 表示主要目录：文件不存在应报错而不是落到附加目录。
	if _, err := service.ReadFile(view.ID, "", "note.txt"); err == nil {
		t.Fatal("主要目录下不存在的文件应报错，而不是落到附加目录")
	}

	// 未知 folderID 报错。
	if _, err := service.ReadFile(view.ID, "wfo_missing", "note.txt"); err == nil {
		t.Fatal("未知 folderID 应报错")
	}
}

func TestAddProjectDuplicateFolderRejected(t *testing.T) {
	_, service := newWorkspaceService(t)
	folder := t.TempDir()
	if _, err := service.AddProject("dup", []string{folder, folder}, "trusted"); err == nil {
		t.Fatal("重复的源文件夹应拒绝")
	}
}
