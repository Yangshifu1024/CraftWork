// Package mcp 实现最小的 MCP stdio 客户端与动态工具适配。
//
// 传输使用一行一个 JSON-RPC 消息；支持 initialize、tools/list 与 tools/call。
// 不执行服务器返回的指令，工具描述与输出都按不可信数据处理。
package mcp

// ServerConfig 是一个本地 MCP 服务器配置。
type ServerConfig struct {
	ID      string   `json:"id"`
	Name    string   `json:"name"`
	Command string   `json:"command"`
	Args    []string `json:"args,omitempty"`
	Enabled bool     `json:"enabled"`
}

// ToolInfo 是 MCP 服务器暴露的工具。
type ToolInfo struct {
	Name        string         `json:"name"`
	Description string         `json:"description"`
	InputSchema map[string]any `json:"inputSchema"`
}
