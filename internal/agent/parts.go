// 本文件定义会话部件（Part）载荷的 Go 侧类型。
//
// 部件以 JSON 形式写入事件表并在前端消费，此前载荷形状只活在
// map[string]any 的字符串键约定里：写入（runTool 持久化）与读取
// （history 重建）各自拼一遍键名，靠防御性断言防止漂移。把形状
// 收进类型后，写读共用同一定义，新增字段的改动集中在一处。

package agent

import (
	"encoding/json"
)

// ToolCallPart 是 tool（工具调用）部件的载荷。
//
// 写入侧由 runTool 构造（开始与完成两个阶段共用，阶段差异只在
// Status 与结果字段）；读取侧由 history 重建模型消息时解码，工具
// 调用与工具结果都从这里取字段。
type ToolCallPart struct {
	// PartID 是部件标识，与事件 ID 一致，前端用作 React 列表键。
	PartID string `json:"partId"`
	// ToolCallID 是模型给出的工具调用标识，回填 tool 消息时用它配对。
	ToolCallID string `json:"toolCallId"`
	// Name 是工具名称。
	Name string `json:"name"`
	// Args 是模型给出的原始参数。
	Args map[string]any `json:"args"`
	// Risk 是评估后的风险等级（R0–R4 的字符串形式）。
	Risk string `json:"risk"`
	// Status 是卡片状态：running（已发起）或 done / error（已结束）。
	Status string `json:"status"`
	// Output 是工具输出或拦截原因，作为 tool 结果回填给模型。
	Output string `json:"output"`
	// IsError 表示本次调用失败。
	IsError bool `json:"isError"`
	// Code 是结构化错误码。
	Code string `json:"code,omitempty"`
	// DurationMs 是工具执行的墙钟耗时。
	DurationMs int64 `json:"durationMs"`
	// Summary 是审批卡与工具卡的操作摘要。
	Summary string `json:"summary"`
}

// decodePart 把从事件表读出的部件载荷转换成目标类型。
//
// 载荷在 JSON 落库后以 map 形式读出，这里通过二次编码回到强类型；
// 载荷缺失或形状不符时返回错误，调用方决定跳过还是降级。
func decodePart(data any, target any) error {
	raw, err := json.Marshal(data)
	if err != nil {
		return err
	}
	return json.Unmarshal(raw, target)
}
