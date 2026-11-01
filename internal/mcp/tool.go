package mcp

import (
	"context"
	"fmt"
	"strings"

	"craftwork/internal/tools"
)

// DynamicTool 把 MCP 工具适配成 Agent 可调用的工具。
type DynamicTool struct {
	client *Client
	server ServerConfig
	info   ToolInfo
}

// NewDynamicTool 创建 MCP 工具适配器。
func NewDynamicTool(client *Client, server ServerConfig, info ToolInfo) *DynamicTool {
	return &DynamicTool{client: client, server: server, info: info}
}

// Name 返回 mcp__服务器__工具 形式的名称。
func (t *DynamicTool) Name() string {
	return "mcp__" + sanitize(t.server.ID) + "__" + sanitize(t.info.Name)
}

func (t *DynamicTool) Description() string {
	return "MCP " + t.server.Name + "：" + t.info.Description
}

func (t *DynamicTool) Schema() map[string]any {
	if t.info.InputSchema == nil {
		return map[string]any{"type": "object", "properties": map[string]any{}}
	}
	return t.info.InputSchema
}

func (t *DynamicTool) Execute(ctx context.Context, args map[string]any) tools.Result {
	output, isError, err := t.client.CallTool(ctx, t.info.Name, args)
	if err != nil {
		return tools.Result{Output: fmt.Sprintf("MCP 调用失败: %v", err), IsError: true}
	}
	return tools.Result{
		Output:  output,
		IsError: isError,
		Meta:    map[string]any{"server": t.server.Name, "tool": t.info.Name},
	}
}

func sanitize(value string) string {
	var builder strings.Builder
	for _, char := range value {
		switch {
		case char >= 'a' && char <= 'z',
			char >= 'A' && char <= 'Z',
			char >= '0' && char <= '9',
			char == '_', char == '-':
			builder.WriteRune(char)
		default:
			builder.WriteByte('_')
		}
	}
	result := strings.Trim(builder.String(), "_")
	if result == "" {
		return "tool"
	}
	return result
}

var _ tools.Tool = (*DynamicTool)(nil)
