// 本文件把 MCP 服务器暴露的工具适配成 Agent 可调用的 tools.Tool，
// 并统一处理名称净化、风险声明与结果转换。

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
	// client 是已连接的 MCP 客户端。
	client *Client
	// server 是工具所属服务器的配置，用于展示名称与生成前缀。
	server ServerConfig
	// info 是服务器声明的工具名称、描述与参数 Schema。
	info ToolInfo
}

// NewDynamicTool 创建 MCP 工具适配器。
func NewDynamicTool(client *Client, server ServerConfig, info ToolInfo) *DynamicTool {
	return &DynamicTool{client: client, server: server, info: info}
}

// Name 返回 mcp__服务器__工具 形式的名称。
//
// 名称中的非法字符会被替换，保证能安全地出现在提示词与审批卡中。
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

// Execute 调用远端工具并转换成统一结果结构。
//
// 传输层失败与工具自身报告的错误都会标记为 IsError，前端与模型
// 可以据此决定是重试还是换一种做法。
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

// sanitize 把任意字符串转换成只含字母、数字、下划线与短横线的标识。
// 其他字符替换为下划线，并去掉首尾下划线；结果为空时回退到 tool。
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

// 编译期检查：DynamicTool 必须满足 tools.Tool 接口。
var _ tools.Tool = (*DynamicTool)(nil)
