package agent

import (
	"path/filepath"
	"testing"

	"github.com/yangshifu1024/craftwork/internal/store"
)

func newTestGrants(t *testing.T) (*Grants, *store.DB) {
	t.Helper()
	db, err := store.Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return NewGrants(db), db
}

func TestCommandPrefix(t *testing.T) {
	cases := []struct {
		command string
		prefix  string
		ok      bool
	}{
		{command: "git status --short", prefix: "git status", ok: true},
		{command: "go test ./...", prefix: "go test", ok: true},
		{command: "npm install -D vitest", prefix: "npm install", ok: true},
		{command: "ls -la", prefix: "ls", ok: true},
		{command: "pwd", prefix: "pwd", ok: true},
		{command: "git status && rm -rf x", ok: false},
		{command: "echo hi | cat", ok: false},
		{command: "sudo ls", ok: false},
		{command: "bash -c 'ls'", ok: false},
		{command: "env FOO=1 go test", ok: false},
	}
	for _, tc := range cases {
		prefix, ok := commandPrefix(tc.command)
		if ok != tc.ok || (ok && prefix != tc.prefix) {
			t.Fatalf("%q 前缀不符: got (%q,%v) want (%q,%v)", tc.command, prefix, ok, tc.prefix, tc.ok)
		}
	}
}

func TestGrantsAllowCommandPrefix(t *testing.T) {
	grants, _ := newTestGrants(t)
	if err := grants.Grant("shell_exec", "git status --short", "session"); err != nil {
		t.Fatal(err)
	}
	if !grants.Allowed("shell_exec", "git status --short") {
		t.Fatal("原命令应被允许")
	}
	if !grants.Allowed("shell_exec", "git status --porcelain") {
		t.Fatal("同前缀命令应被允许")
	}
	if grants.Allowed("shell_exec", "git commit -m x") {
		t.Fatal("不同子命令不应被允许")
	}
	if grants.Allowed("fs_write", "git status --short") {
		t.Fatal("前缀授权只应作用于 shell_exec")
	}
}

func TestGrantsDoNotPrefixComplexCommands(t *testing.T) {
	grants, _ := newTestGrants(t)
	command := "git status && rm -rf build"
	if err := grants.Grant("shell_exec", command, "session"); err != nil {
		t.Fatal(err)
	}
	if !grants.Allowed("shell_exec", command) {
		t.Fatal("精确命令应被允许")
	}
	if grants.Allowed("shell_exec", "git status") {
		t.Fatal("复杂命令不应产生前缀授权")
	}
}

func TestGrantsWorkspaceScopePersists(t *testing.T) {
	grants, db := newTestGrants(t)
	if err := grants.Grant("shell_exec", "go test ./...", "workspace"); err != nil {
		t.Fatal(err)
	}
	reloaded := NewGrants(db)
	if !reloaded.Allowed("shell_exec", "go test ./internal/agent") {
		t.Fatal("工作区授权应在重启后保留前缀匹配")
	}
}
