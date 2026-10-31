package agent

import (
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
// 当前实现的 key 是“工具名 + 调用目标”，目标通常是命令文本或文件路径。
// 这是保守策略：授权一次命令不会放行其它命令。
func (g *Grants) Allowed(tool, target string) bool {
	key := grantKey(tool, target)
	g.mu.RLock()
	defer g.mu.RUnlock()
	return g.session[key] || g.workspace[key]
}

// Grant 记录一次授权。
func (g *Grants) Grant(tool, target, scope string) error {
	key := grantKey(tool, target)
	g.mu.Lock()
	defer g.mu.Unlock()

	switch scope {
	case "session":
		g.session[key] = true
		return nil
	case "workspace":
		g.workspace[key] = true
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
