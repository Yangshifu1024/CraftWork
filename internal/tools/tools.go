// Package tools 实现 Agent 可调用的内置工具。
//
// 本包集中声明每个工具的静态元数据（名称、描述、参数 Schema、风险等级、
// 路径与命令参数），并提供统一的注册表、路径解析、写入前备份与输出截断
// 出口。新增工具只需在 Spec 中声明一次，注册、策略判断与审批展示都会
// 读取同一份声明。
//
// 所有文件路径都经过 policy.Resolve 校验；写入类工具在落盘前备份原文件。
// 工具输出统一在 Registry.Execute 出口截断到 32 KB，避免把大段二进制
// 或日志塞进上下文。
package tools

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"

	"craftwork/internal/platform"
	"craftwork/internal/policy"
)

// Result 是工具执行结果。成功与失败共用同一结构，用 IsError 与 Code 区分。
type Result struct {
	// Output 是返回给模型的文本，可能是正常内容，也可能是错误说明。
	Output string `json:"output"`
	// IsError 表示本次执行失败，模型应据此调整后续动作。
	IsError bool `json:"isError"`
	// Code 是结构化错误码，便于前端与审计区分失败类型。
	Code string `json:"code,omitempty"`
	// Meta 是给界面展示使用的附加信息，不进入模型上下文。
	Meta map[string]any `json:"meta,omitempty"`
}

// Spec 是暴露给模型的工具定义，以及执行入口需要的静态元数据。
//
// 新增一个工具只需要在这里声明一次风险、路径与摘要位置，注册表、
// 策略判断与审批展示都从同一份声明读取。
type Spec struct {
	// Name 是工具在模型侧的唯一标识，也是注册表的键。
	Name string
	// Description 是给模型看的用途说明。
	Description string
	// Schema 是 JSON Schema 形式的参数定义。
	Schema map[string]any
	// Risk 是工具的默认风险等级；命令类工具会在执行前按命令内容重算。
	Risk policy.Risk
	// PathArg 是需要解析并校验的路径参数名；留空表示该工具不接触文件路径。
	PathArg string
	// CommandArg 是需要分类的命令参数名，目前仅 shell_exec 使用。
	CommandArg string
	// Work 表示该工具在 Work 模式下仍然开放。
	Work bool
	// Title 是审批卡标题，空时使用 Name。
	Title string
	// SummaryArg 是摘要优先取用的参数名。
	SummaryArg string
	// SummaryText 是参数为空或没有摘要参数时使用的固定摘要。
	SummaryText string
}

// TitleOf 返回审批卡标题。
func (s Spec) TitleOf() string {
	if s.Title != "" {
		return s.Title
	}
	return s.Name
}

// SummaryOf 返回一行操作摘要，用于审批卡与工具卡片。
func (s Spec) SummaryOf(args map[string]any) string {
	if s.SummaryArg != "" {
		if value := strings.TrimSpace(stringArg(args, s.SummaryArg)); value != "" {
			return value
		}
	}
	if s.SummaryText != "" {
		return s.SummaryText
	}
	if s.SummaryArg != "" {
		return "."
	}
	return s.Name
}

// Tool 是内置工具的统一接口。
//
// Spec 声明静态元数据，Execute 只实现领域动作；路径解析、权限判断、
// 输出截断与错误码由 Registry 的统一出口负责。
type Tool interface {
	// Spec 返回工具的静态元数据。
	Spec() Spec
	// Execute 执行工具动作；路径解析、备份出口与参数校验已由注册表准备。
	Execute(ctx context.Context, call Call) Result
}

// Call 是一次已经准备好的工具调用：路径已经解析，备份出口已经绑定。
type Call struct {
	// Name 是工具名称，与 Spec.Name 一致。
	Name string
	// Args 是模型给出的原始参数，保持原样交给工具自行解释。
	Args map[string]any
	// Target 是路径参数解析后的绝对路径；工具没有路径参数时为空。
	Target string
	// Display 是相对工作区的展示路径，只用于输出与审批卡。
	Display string
	// Workspace 是调用发生时的工作区根目录，用于相对路径换算。
	Workspace string
	// Err 表示路径解析失败；非空时 Registry 不会执行工具。
	Err error
	// backup 由注册表注入，写入前用它保存原文件内容。
	backup func(abs string, data []byte) error
}

// Backup 在写入前保存目标文件的原始内容。
//
// data 必须是即将被覆盖的当前内容；没有配置备份目录时静默跳过。
func (c Call) Backup(data []byte) error {
	if c.Err != nil {
		return c.Err
	}
	if c.backup == nil || c.Target == "" {
		return nil
	}
	return c.backup(c.Target, data)
}

// Resolve 解析额外路径（例如 git_commit 的 paths 列表），仍然限制在工作区内。
//
// rel 为空时按当前工作区根目录处理；返回绝对路径与相对展示路径。
func (c Call) Resolve(rel string) (abs string, display string, err error) {
	if strings.TrimSpace(rel) == "" {
		rel = "."
	}
	abs, err = resolvePath(c.Workspace, rel)
	if err != nil {
		return "", "", err
	}
	return abs, displayPath(c.Workspace, abs), nil
}

// Options 是工具注册表的配置。
type Options struct {
	// Workspace 是工作区根目录的绝对路径。
	Workspace string
	// BackupDir 是写入前备份的目录；为空表示不落盘备份。
	BackupDir string
	// RecordBackup 把备份记录写入数据库。
	RecordBackup func(path, backupPath, hash string, size int64) error
}

// Registry 持有当前工作区可用的工具集合。
type Registry struct {
	// mu 保护 opts 与 tools，支持运行期切换工作区、注册 MCP 工具。
	mu sync.RWMutex
	// opts 是注册表配置，其中 Workspace 会随活动工作区变化。
	opts Options
	// tools 以工具名称为键，内置工具在构造时一次性注册。
	tools map[string]Tool
}

// NewRegistry 创建工具注册表。
//
// 这里注册的是全部内置工具；MCP 工具稍后通过 Register 动态加入。
func NewRegistry(opts Options) *Registry {
	registry := &Registry{opts: opts, tools: map[string]Tool{}}
	for _, tool := range []Tool{
		&fsListTool{},
		&fsReadTool{},
		&fsGlobTool{},
		&fsGrepTool{},
		&fsWriteTool{},
		&fsEditTool{},
		&applyPatchTool{},
		&shellExecTool{},
		&gitStatusTool{},
		&gitDiffTool{},
		&gitLogTool{},
		&gitCommitTool{},
		&docReadTool{},
		&docEditTool{},
		&xlsxSetCellTool{},
		&todoWriteTool{},
	} {
		registry.tools[tool.Spec().Name] = tool
	}
	return registry
}

// SetWorkspace 切换工作区。
func (r *Registry) SetWorkspace(workspace string) {
	r.mu.Lock()
	r.opts.Workspace = workspace
	r.mu.Unlock()
}

// Workspace 返回当前工作区。
func (r *Registry) Workspace() string {
	r.mu.RLock()
	defer r.mu.RUnlock()
	return r.opts.Workspace
}

// Get 按名称取工具。
func (r *Registry) Get(name string) (Tool, bool) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	tool, ok := r.tools[name]
	return tool, ok
}

// SpecFor 按名称返回工具元数据。
func (r *Registry) SpecFor(name string) (Spec, bool) {
	tool, ok := r.Get(name)
	if !ok {
		return Spec{Name: name}, false
	}
	return tool.Spec(), true
}

// Register 动态注册工具（用于 MCP）。同名工具会被替换。
func (r *Registry) Register(tool Tool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.tools[tool.Spec().Name] = tool
}

// Unregister 移除动态注册的工具。
func (r *Registry) Unregister(name string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.tools, name)
}

// Specs 返回所有工具定义，按名称排序保证提示词稳定。
func (r *Registry) Specs() []Spec {
	r.mu.RLock()
	defer r.mu.RUnlock()
	return r.specsLocked()
}

// specsLocked 在已持有读锁的前提下收集并排序工具定义。
//
// 排序保证提示词里的工具顺序稳定，避免相同配置产生不同的模型输入。
func (r *Registry) specsLocked() []Spec {
	names := make([]string, 0, len(r.tools))
	for name := range r.tools {
		names = append(names, name)
	}
	sort.Strings(names)

	specs := make([]Spec, 0, len(names))
	for _, name := range names {
		specs = append(specs, r.tools[name].Spec())
	}
	return specs
}

// SpecsForMode 返回指定模式可用的工具。
//
// Work 模式只提供声明了 Work 的文件、文档与任务清单工具，
// 以及动态注册的 MCP 工具；不暴露 Shell 与 Git 能力。
func (r *Registry) SpecsForMode(mode string) []Spec {
	r.mu.RLock()
	defer r.mu.RUnlock()
	specs := r.specsLocked()
	if mode != "work" {
		return specs
	}
	filtered := make([]Spec, 0, len(specs))
	for _, spec := range specs {
		if spec.Work || strings.HasPrefix(spec.Name, "mcp__") {
			filtered = append(filtered, spec)
		}
	}
	return filtered
}

// Allows 判断某个工具在指定模式下是否可用。
func (r *Registry) Allows(mode, name string) bool {
	if mode != "work" {
		return true
	}
	spec, ok := r.SpecFor(name)
	if !ok {
		return strings.HasPrefix(name, "mcp__")
	}
	return spec.Work || strings.HasPrefix(name, "mcp__")
}

// Assess 解析调用目标并给出策略结论。
//
// 返回的 Call 可以直接交给 Execute，路径只解析一次。exists 为 false
// 表示工具未注册，调用方应拒绝执行。
func (r *Registry) Assess(name string, args map[string]any) (policy.Assessment, Call, bool) {
	r.mu.RLock()
	tool, exists := r.tools[name]
	workspace := r.opts.Workspace
	r.mu.RUnlock()

	call := Call{Name: name, Args: args, Workspace: workspace}
	if !exists {
		// 未注册的工具一律按最高常见风险处理，交由上层拒绝，避免绕过策略。
		return policy.Assessment{Risk: policy.R2, Reason: "执行命令"}, call, false
	}

	spec := tool.Spec()
	assessment := policy.Assessment{Risk: spec.Risk, Reason: policy.ReasonFor(spec.Risk)}

	// 命令类工具的风险由命令内容决定，例如 rm -rf 会被提升到 R4。
	if spec.CommandArg != "" {
		command := stringArg(args, spec.CommandArg)
		assessment.Target = command
		assessment.Risk, assessment.Reason = policy.ClassifyCommand(command)
	}

	// 路径类工具在这里完成唯一一次解析：越界直接判 R3 并记录错误，
	// 后续 Execute 使用已经解析好的 Target，不再信任前端传入的路径。
	if spec.PathArg != "" {
		path := stringArg(args, spec.PathArg)
		if path == "" {
			path = "."
		}
		resolved, err := resolvePath(workspace, path)
		if err != nil {
			assessment.Risk = policy.R3
			assessment.Reason = "路径超出工作区"
			call.Err = err
			return assessment, call, true
		}
		call.Target = resolved
		call.Display = displayPath(workspace, resolved)
		call.backup = r.backup
		// 同时带命令参数时，审批目标保留命令本身而不是工作目录。
		if spec.CommandArg == "" {
			assessment.Target = resolved
		}
		return assessment, call, true
	}

	call.backup = r.backup
	return assessment, call, true
}

// Execute 是工具执行的唯一出口。
//
// 输出在这里统一截断，错误统一补上错误码；MCP 等动态工具也走同一出口。
func (r *Registry) Execute(ctx context.Context, call Call) Result {
	// 路径解析阶段已经失败，不再给工具任何执行机会。
	if call.Err != nil {
		return errorResult("%v", call.Err)
	}
	tool, ok := r.Get(call.Name)
	if !ok {
		result := errorResult("未知工具：%s", call.Name)
		result.Code = CodeUnknownTool
		return result
	}
	result := tool.Execute(ctx, call)
	// 无论工具自身返回什么，出口都保证输出长度受限。
	result.Output = truncate(result.Output, maxOutputBytes)
	if result.IsError && result.Code == "" {
		result.Code = CodeToolError
	}
	return result
}

// 输出与读取上限。工具输出在统一出口截断，fs_read 单次读取也有上限，
// 防止一次调用把超大内容灌进模型上下文。
const (
	// maxOutputBytes 是单个工具输出允许保留的最大字节数。
	maxOutputBytes = 32 * 1024
	// maxReadBytes 是 fs_read 单次读取的最大字节数。
	maxReadBytes = 256 * 1024
	// maxReadLines 是 fs_read 单次返回的最大行数。
	maxReadLines = 2000

	// CodeToolError 是工具执行失败的默认错误码。
	CodeToolError = "tool_error"
	// CodeUnknownTool 表示模型调用了未注册的工具。
	CodeUnknownTool = "unknown_tool"
	// CodeModeDenied 表示当前模式不开放该工具。
	CodeModeDenied = "mode_denied"
)

// IsIgnoredDir 判断目录应被文件树与搜索忽略。
func IsIgnoredDir(name string) bool {
	return isIgnoredDir(name)
}

// truncate 按字节上限截断输出并注明原始长度。
//
// 只做字节切片，可能把多字节字符切成两半；这是可接受的取舍，
// 因为该出口主要用于限制日志与命令输出，模型侧仍能读懂提示。
func truncate(text string, limit int) string {
	if len(text) <= limit {
		return text
	}
	return text[:limit] + fmt.Sprintf("\n…（输出已截断，原始长度 %d 字节）", len(text))
}

// backup 在写入前保存原文件内容。
//
// 备份文件写入备份目录并记录哈希；配置了 RecordBackup 时再把记录
// 落到数据库，供审计与恢复使用。
func (r *Registry) backup(absPath string, data []byte) error {
	r.mu.RLock()
	backupDir := r.opts.BackupDir
	record := r.opts.RecordBackup
	r.mu.RUnlock()

	if backupDir == "" {
		return nil
	}
	if err := os.MkdirAll(backupDir, 0o700); err != nil {
		return fmt.Errorf("创建备份目录: %w", err)
	}
	backupPath := filepath.Join(backupDir, platform.BackupName(absPath))
	if err := os.WriteFile(backupPath, data, 0o600); err != nil {
		return fmt.Errorf("写入备份: %w", err)
	}
	hash := platform.Sha256Hex(data)
	if record != nil {
		if err := record(absPath, backupPath, hash, int64(len(data))); err != nil {
			return fmt.Errorf("记录备份: %w", err)
		}
	}
	return nil
}

// stringArg 读取字符串参数；缺失、nil 或类型不符时返回空字符串。
func stringArg(args map[string]any, key string) string {
	value, ok := args[key]
	if !ok || value == nil {
		return ""
	}
	text, _ := value.(string)
	return text
}

// intArg 读取整数参数。JSON 反序列化后数字是 float64，
// 这里同时兼容 float64、int 与 int64，其余类型回退到默认值。
func intArg(args map[string]any, key string, fallback int) int {
	value, ok := args[key]
	if !ok || value == nil {
		return fallback
	}
	switch typed := value.(type) {
	case float64:
		return int(typed)
	case int:
		return typed
	case int64:
		return int(typed)
	default:
		return fallback
	}
}

// boolArg 读取布尔参数；缺失或类型不符时按 false 处理。
func boolArg(args map[string]any, key string) bool {
	value, ok := args[key]
	if !ok {
		return false
	}
	result, _ := value.(bool)
	return result
}

// errorResult 构造一个带默认错误码的失败结果。
func errorResult(format string, args ...any) Result {
	return Result{Output: fmt.Sprintf(format, args...), IsError: true, Code: CodeToolError}
}
