//go:build !windows

package platform

import (
	"context"
	"testing"
	"time"
)

func TestRunTimeoutKillsProcessGroup(t *testing.T) {
	started := time.Now()
	result := Run(context.Background(), RunSpec{
		Path: "/bin/sh", Args: []string{"-c", "sleep 5 & wait"},
		Dir: t.TempDir(), Timeout: 300 * time.Millisecond, OutputLimit: 1024,
	})
	elapsed := time.Since(started)
	if !result.TimedOut {
		t.Fatal("命令应在超时后结束")
	}
	if elapsed > 3*time.Second {
		t.Fatalf("超时后进程组未被及时终止，耗时 %s", elapsed)
	}
}
