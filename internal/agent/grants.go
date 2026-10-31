package agent

import (
	"strings"
	"sync"

	"craftwork/internal/store"
)

const workspaceGrantsKey = "policy.workspace_grants"

// Grants 保存“本会话”和“本工作区”的授权。
//
// 会话授权只存在内存中，应用退出即失效；工作区授权写入 settings，
// 下次启动仍然有效。R3/R4 不会走到这里（策略层已禁止永久允许）。
type Grants struct {
	mu        sync.RWMutex
	db        *store.DB
	session   map[string]bool
	workspace map[string]bool
}

// NewGrants 创建工作区授权缓存并读取已保存的授权。
func NewGrants(db *store.DB) *Grants {
	grants := &Grants{db: db, session: map[string]bool{}, workspace: map[string]bool{}}
	var saved map[string]bool
	if err := db.GetSetting(workspaceGrantsKey, &saved); err == nil && saved != nil {
		grants.workspace = saved
	}
	return grants
}

// Allowed 判断某个工具目标是否已被授权。
//
// 文件类工具按“工具名 + 目标”精确匹配；shell_exec 额外支持命令前缀授权，
// 例如允许 `git status` 后，`git status --short` 也不再询问。
func (g *Grants) Allowed(tool, target string) bool {
	key := grantKey(tool, target)
	g.mu.RLock()
	defer g.mu.RUnlock()
	if g.session[key] || g.workspace[key] {
		return true
	}
	if tool == "shell_exec" {
		if prefix, ok := commandPrefix(target); ok {
			prefixKey := grantPrefixKey(tool, prefix)
			if g.session[prefixKey] || g.workspace[prefixKey] {
				return true
			}
		}
	}
	return false
}

// Grant 记录一次授权。
func (g *Grants) Grant(tool, target, scope string) error {
	key := grantKey(tool, target)
	prefix := ""
	if tool == "shell_exec" {
		if value, ok := commandPrefix(target); ok {
			prefix = grantPrefixKey(tool, value)
		}
	}
	g.mu.Lock()
	defer g.mu.Unlock()

	switch scope {
	case "session":
		g.session[key] = true
		if prefix != "" {
			g.session[prefix] = true
		}
		return nil
	case "workspace":
		g.workspace[key] = true
		if prefix != "" {
			g.workspace[prefix] = true
		}
		snapshot := make(map[string]bool, len(g.workspace))
		for item := range g.workspace {
			snapshot[item] = true
		}
		return g.db.SetSetting(workspaceGrantsKey, snapshot)
	default:
		return nil
	}
}

// Snapshot 返回工作区授权的副本，供设置页展示。
func (g *Grants) Snapshot() map[string]bool {
	g.mu.RLock()
	defer g.mu.RUnlock()
	result := make(map[string]bool, len(g.workspace))
	for key := range g.workspace {
		result[key] = true
	}
	return result
}

func grantKey(tool, target string) string {
	return tool + "|" + target
}

func grantPrefixKey(tool, prefix string) string {
	return tool + "|prefix:" + prefix
}

// commandPrefix 计算可安全复用的命令前缀。
//
// 规则：取前两个空白分隔的片段；第二段是选项时只取第一段。包含管道、
// 重定向、命令串联或解释器包装的命令过于复杂，不提供前缀授权，只允许
// 精确匹配这一次调用。
func commandPrefix(command string) (string, bool) {
	trimmed := strings.TrimSpace(command)
	if trimmed == "" {
		return "", false
	}
	if strings.ContainsAny(trimmed, ";|&><`\n\r") ||
		strings.Contains(trimmed, "$(") || strings.Contains(trimmed, "${") {
		return "", false
	}
	fields := strings.Fields(trimmed)
	if len(fields) == 0 {
		return "", false
	}
	switch fields[0] {
	case "env", "sudo", "bash", "sh", "zsh", "fish", "powershell", "cmd", "eval", "exec":
		return "", false
	}
	if len(fields) == 1 || strings.HasPrefix(fields[1], "-") {
		return fields[0], true
	}
	return fields[0] + " " + fields[1], true
}
