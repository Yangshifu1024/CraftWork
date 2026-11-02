package service

import (
	"path/filepath"
	"strings"
	"testing"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/store"
	"github.com/yangshifu1024/craftwork/internal/tools"
)

func TestSplitCommandLine(t *testing.T) {
	cases := []struct {
		line    string
		command string
		args    []string
	}{
		{line: "npx -y server .", command: "npx", args: []string{"-y", "server", "."}},
		{line: `sh -c "echo hello"`, command: "sh", args: []string{"-c", "echo hello"}},
		{line: `tool --path '/tmp/a b'`, command: "tool", args: []string{"--path", "/tmp/a b"}},
	}
	for _, tc := range cases {
		command, args, err := splitCommandLine(tc.line)
		if err != nil {
			t.Fatalf("%q 解析失败: %v", tc.line, err)
		}
		if command != tc.command || strings.Join(args, "\x00") != strings.Join(tc.args, "\x00") {
			t.Fatalf("%q 解析不符: (%s, %v)", tc.line, command, args)
		}
	}
	if _, _, err := splitCommandLine(`tool "unclosed`); err == nil {
		t.Fatal("未闭合引号应报错")
	}
	if _, _, err := splitCommandLine("   "); err == nil {
		t.Fatal("空命令应报错")
	}
}

func TestMCPServicePersistsAndReportsConnectionFailure(t *testing.T) {
	db, err := store.Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	bus := events.NewBus(func(string, any) {})
	t.Cleanup(bus.Close)
	registry := tools.NewRegistry(tools.Options{})
	service := NewMCPService(db, registry)

	view, err := service.Add("broken", "false")
	if err != nil {
		t.Fatalf("配置应已保存，连接错误通过视图返回: %v", err)
	}
	if view.Error == "" || view.Connected {
		t.Fatalf("失败状态不符: %+v", view)
	}

	list := service.List()
	if len(list) != 1 || list[0].Name != "broken" {
		t.Fatalf("服务器应已保存: %+v", list)
	}

	reloaded := NewMCPService(db, registry)
	if len(reloaded.List()) != 1 {
		t.Fatal("服务器配置应在重启后保留")
	}

	if err := service.Remove(view.ID); err != nil {
		t.Fatal(err)
	}
	if len(service.List()) != 0 {
		t.Fatal("移除后不应再有服务器")
	}
}
