// 本文件维护“已经允许过”的工具授权缓存，避免用户在会话内或同一工作区里
// 被反复询问同一个操作。
//
// 会话授权只存在内存，应用退出即失效；工作区授权写入 settings 表，重启后仍然有效。
// 缓存只负责“是否不再询问”，具体的风险分级与受限工作区规则仍由策略层决定。

package agent

import (
	"strings"
	"sync"

	"github.com/yangshifu1024/craftwork/internal/store"
)

// workspaceGrantsKey 是工作区授权在 settings 表中的键名。
const workspaceGrantsKey = "policy.workspace_grants"

// Grants 保存“本会话”和“本工作区”的授权。
//
// 会话授权只存在内存中，应用退出即失效；工作区授权写入 settings，
// 下次启动仍然有效。R3/R4 不会走到这里（策略层已禁止永久允许）。
type Grants struct {
	// mu 是读写锁：判断授权远多于写入授权，读路径可以并发。
	mu sync.RWMutex
	// db 用于持久化工作区授权。
	db *store.DB
	// session 保存本进程内的会话授权，键为工具名与目标的组合。
	session map[string]bool
	// workspace 保存跨进程有效的工作区授权，与 settings 中的内容保持一致。
	workspace map[string]bool
}

// NewGrants 创建工作区授权缓存并读取已保存的授权。
//
// 读取失败时保持空缓存：宁可多问一次，也不要因为配置损坏而放行。
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
	// 先看精确匹配：同一工具、同一个目标。
	if g.session[key] || g.workspace[key] {
		return true
	}
	// 再看命令前缀授权；无法提取安全前缀的命令只能靠精确匹配。
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
//
// once 不写入缓存（本来就要每次都问）；session 只写内存；workspace 在写内存的
// 同时把整份工作区授权保存到 settings。
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
		// 复制一份快照再写库，避免在持锁期间序列化 map 导致并发问题。
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
//
// 返回副本而不是内部 map，调用方修改它不会影响正在生效的授权。
func (g *Grants) Snapshot() map[string]bool {
	g.mu.RLock()
	defer g.mu.RUnlock()
	result := make(map[string]bool, len(g.workspace))
	for key := range g.workspace {
		result[key] = true
	}
	return result
}

// grantKey 生成“精确匹配”的授权键，工具名与目标用竖线分隔。
func grantKey(tool, target string) string {
	return tool + "|" + target
}

// grantPrefixKey 生成“命令前缀”的授权键，与精确匹配区分开，避免两者混淆。
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
	// 含管道、重定向、命令串联、反引号或变量替换的命令行为难以预测，不提供前缀授权。
	if strings.ContainsAny(trimmed, ";|&><`\n\r") ||
		strings.Contains(trimmed, "$(") || strings.Contains(trimmed, "${") {
		return "", false
	}
	fields := strings.Fields(trimmed)
	if len(fields) == 0 {
		return "", false
	}
	// env、sudo 与各种解释器会改变实际执行的命令，同样不提供前缀授权。
	switch fields[0] {
	case "env", "sudo", "bash", "sh", "zsh", "fish", "powershell", "cmd", "eval", "exec":
		return "", false
	}
	// 只有一个片段，或第二段是选项时，前缀就是命令本身。
	if len(fields) == 1 || strings.HasPrefix(fields[1], "-") {
		return fields[0], true
	}
	return fields[0] + " " + fields[1], true
}
