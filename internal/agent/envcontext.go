// 本文件负责在每次运行开始时收集环境上下文：当天日期、平台标识、默认
// shell、Git 快照与两级指令文件（用户全局与工作区项目的 AGENTS.md）。
//
// 收集到的快照交给 prompt.Build 组装进动态提示词块。这里只做读取与
// 截断，不做任何提示词拼接；读取失败一律降级为省略对应部分，不阻断
// 运行——环境公告是增强信息，不值得为它让对话失败。

package agent

import (
	"bufio"
	"context"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"

	"craftwork/internal/agent/prompt"
	"craftwork/internal/platform"
)

const (
	// gitSnapshotTimeout 是收集 Git 快照时单条命令的超时。快照在运行
	// 开始时同步读取，必须远小于一次模型往返的时间。
	gitSnapshotTimeout = 3 * time.Second
	// gitSnapshotCommits 是快照包含的最近提交条数。
	gitSnapshotCommits = 5
	// instructionMaxLines 是单个指令文件读取的最大行数。
	instructionMaxLines = 200
	// instructionMaxChars 是单个指令文件正文的最大字符数。
	instructionMaxChars = 25000
)

// collectEnvContext 收集本次运行的环境上下文。任何一步失败都降级：
// Git 不可用时不产出快照，指令文件缺失或不可读时跳过该文件。
func collectEnvContext(ctx context.Context, dataDir, workspace string) prompt.EnvContextSnapshot {
	snapshot := prompt.EnvContextSnapshot{
		Env: prompt.EnvInfo{
			Today:    time.Now().Format("2006-01-02"),
			Platform: runtime.GOOS + "/" + runtime.GOARCH,
			Shell:    shellName(),
		},
	}
	snapshot.Git = collectGitSnapshot(ctx, workspace)
	snapshot.Instructions = collectInstructions(dataDir, workspace)
	return snapshot
}

// shellName 返回 SHELL 环境变量的可执行名；未设置时返回空。
func shellName() string {
	shell := os.Getenv("SHELL")
	if shell == "" {
		return ""
	}
	return filepath.Base(shell)
}

// collectGitSnapshot 读取工作区 Git 快照：是否仓库、当前分支、状态与最近提交。
//
// 不在仓库（rev-parse 失败）时返回 nil；个别命令失败时跳过对应部分，
// 只要仓库判定成立就尽量给出其余信息。
func collectGitSnapshot(ctx context.Context, workspace string) *prompt.GitSnapshot {
	if workspace == "" {
		return nil
	}
	if _, exitCode, timedOut, _ := runGitCommand(ctx, workspace, "rev-parse", "--is-inside-work-tree"); timedOut || exitCode != 0 {
		return nil
	}
	snapshot := &prompt.GitSnapshot{}
	if branch, _, timedOut, _ := runGitCommand(ctx, workspace, "symbolic-ref", "--short", "-q", "HEAD"); !timedOut {
		snapshot.Branch = strings.TrimSpace(branch)
	}
	if status, _, timedOut, _ := runGitCommand(ctx, workspace, "status", "--short"); !timedOut {
		snapshot.StatusLines = nonEmptyLines(status)
	}
	if log, _, timedOut, _ := runGitCommand(ctx, workspace,
		"log", "--oneline", "-n", strconv.Itoa(gitSnapshotCommits)); !timedOut {
		snapshot.Commits = nonEmptyLines(log)
	}
	return snapshot
}

// runGitCommand 直接执行一条只读 git 命令，返回输出、退出码与是否超时。
func runGitCommand(ctx context.Context, dir string, args ...string) (string, int, bool, error) {
	result := platform.Run(ctx, platform.RunSpec{
		Path: "git", Args: args, Dir: dir, Timeout: gitSnapshotTimeout, OutputLimit: 64 * 1024,
	})
	return result.Output, result.ExitCode, result.TimedOut, result.Err
}

// collectInstructions 读取两级指令文件：用户全局（数据目录下的
// AGENTS.md）在前，工作区项目的在后。顺序即显示顺序，全局约定先于
// 项目约定进入上下文。
func collectInstructions(dataDir, workspace string) []prompt.Instruction {
	var instructions []prompt.Instruction
	if dataDir != "" {
		if item := readInstruction(filepath.Join(dataDir, "AGENTS.md"), "global"); item != nil {
			instructions = append(instructions, *item)
		}
	}
	if workspace != "" {
		if item := readInstruction(filepath.Join(workspace, "AGENTS.md"), "project"); item != nil {
			instructions = append(instructions, *item)
		}
	}
	return instructions
}

// readInstruction 读取单个指令文件，按行数与字符数上限截断。
// 文件不存在或不可读时返回 nil；空文件同样返回 nil（没有可注入的内容）。
func readInstruction(path, scope string) *prompt.Instruction {
	file, err := os.Open(path)
	if err != nil {
		return nil
	}
	defer file.Close()

	var builder strings.Builder
	scanner := bufio.NewScanner(file)
	// 指令文件可能包含很长的行，放大缓冲避免扫描器提前失败。
	scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	lines := 0
	truncated := false
	for scanner.Scan() {
		if lines >= instructionMaxLines || builder.Len()+len(scanner.Text())+1 > instructionMaxChars {
			truncated = true
			break
		}
		builder.Write(scanner.Bytes())
		builder.WriteByte('\n')
		lines++
	}
	content := strings.TrimRight(builder.String(), "\n")
	if strings.TrimSpace(content) == "" {
		return nil
	}
	return &prompt.Instruction{Path: path, Scope: scope, Content: content, Truncated: truncated}
}

// nonEmptyLines 把命令输出拆成非空行。
func nonEmptyLines(text string) []string {
	var lines []string
	for _, line := range strings.Split(strings.TrimSpace(text), "\n") {
		if strings.TrimSpace(line) != "" {
			lines = append(lines, line)
		}
	}
	return lines
}
