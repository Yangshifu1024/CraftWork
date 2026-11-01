//go:build !windows

package tools

import (
	"context"
	"strings"
	"testing"
	"time"
)

func TestSanitizedEnvRemovesSecrets(t *testing.T) {
	t.Setenv("CRAFTWORK_TEST_API_KEY", "secret")
	t.Setenv("CRAFTWORK_TEST_VISIBLE", "ok")

	env := sanitizedEnv()
	joined := strings.Join(env, "\n")
	if strings.Contains(joined, "CRAFTWORK_TEST_API_KEY") {
		t.Fatal("API Key 环境变量不应传给子进程")
	}
	if !strings.Contains(joined, "CRAFTWORK_TEST_VISIBLE") {
		t.Fatal("普通环境变量应保留")
	}
}

func TestRunShellTimeoutKillsProcessGroup(t *testing.T) {
	started := time.Now()
	_, _, timedOut, _ := runShell(context.Background(), t.TempDir(), "sleep 5 & wait", 300*time.Millisecond)
	elapsed := time.Since(started)
	if !timedOut {
		t.Fatal("命令应在超时后结束")
	}
	if elapsed > 3*time.Second {
		t.Fatalf("超时后进程组未被及时终止，耗时 %s", elapsed)
	}
}
