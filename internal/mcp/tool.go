package mcp

import (
	"context"
	"fmt"
	"strings"

	"craftwork/internal/policy"
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

// Spec 声明 MCP 工具的静态元数据。
//
// MCP 工具不声明路径参数；风险固定按执行类处理，输出由 Registry
// 统一截断，服务器返回多大内容都不会直接灌进模型上下文。
func (t *DynamicTool) Spec() tools.Spec {
	schema := t.info.InputSchema
	if schema == nil {
		schema = map[string]any{"type": "object", "properties": map[string]any{}}
	}
	return tools.Spec{
		Name:        t.Name(),
		Description: "MCP " + t.server.Name + "：" + t.info.Description,
		Schema:      schema,
		Risk:        policy.R2,
		SummaryText: t.info.Name,
	}
}

func (t *DynamicTool) Execute(ctx context.Context, call tools.Call) tools.Result {
	output, isError, err := t.client.CallTool(ctx, t.info.Name, call.Args)
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
