// Package tools 实现 Agent 可调用的内置工具。
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

	"craftwork/internal/policy"
)

// Result 是工具执行结果。
type Result struct {
	Output  string         `json:"output"`
	IsError bool           `json:"isError"`
	Code    string         `json:"code,omitempty"`
	Meta    map[string]any `json:"meta,omitempty"`
}

// Spec 是暴露给模型的工具定义，以及执行接缝需要的静态元数据。
//
// 新增一个工具只需要在这里声明一次风险、路径与摘要位置，注册表、
// 策略判断与审批展示都从同一份声明读取。
type Spec struct {
	Name        string
	Description string
	Schema      map[string]any
	Risk        policy.Risk
	PathArg     string // 需要解析并校验的路径参数名
	CommandArg  string // 需要分类的命令参数名
	Work        bool   // 是否在 Work 模式开放
	Title       string // 审批卡标题，空时使用名称
	SummaryArg  string // 摘要优先取用的参数
	SummaryText string // 参数为空时的固定摘要
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
	Spec() Spec
	Execute(ctx context.Context, call Call) Result
}

// Call 是一次已经准备好的工具调用：路径已经解析，备份出口已经绑定。
type Call struct {
	Name      string
	Args      map[string]any
	Target    string
	Display   string
	Workspace string
	// Err 表示路径解析失败；非空时 Registry 不会执行工具。
	Err    error
	backup func(abs string, data []byte) error
}

// Backup 在写入前保存目标文件的原始内容。
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
	// BackupDir 是写入前备份的目录。
	BackupDir string
	// RecordBackup 把备份记录写入数据库。
	RecordBackup func(path, backupPath, hash string, size int64) error
}

// Registry 持有当前工作区可用的工具集合。
type Registry struct {
	mu    sync.RWMutex
	opts  Options
	tools map[string]Tool
}

// NewRegistry 创建工具注册表。
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
		return policy.Assessment{Risk: policy.R2, Reason: "执行命令"}, call, false
	}

	spec := tool.Spec()
	assessment := policy.Assessment{Risk: spec.Risk, Reason: policy.ReasonFor(spec.Risk)}

	if spec.CommandArg != "" {
		command := stringArg(args, spec.CommandArg)
		assessment.Target = command
		assessment.Risk, assessment.Reason = policy.ClassifyCommand(command)
	}

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
	result.Output = truncate(result.Output, maxOutputBytes)
	if result.IsError && result.Code == "" {
		result.Code = CodeToolError
	}
	return result
}

const (
	maxOutputBytes = 32 * 1024
	maxReadBytes   = 256 * 1024
	maxReadLines   = 2000

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

// truncate 按字节上限截断输出并注明。
func truncate(text string, limit int) string {
	if len(text) <= limit {
		return text
	}
	return text[:limit] + fmt.Sprintf("\n…（输出已截断，原始长度 %d 字节）", len(text))
}

// backup 在写入前保存原文件内容。
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
	backupPath := filepath.Join(backupDir, backupName(absPath))
	if err := os.WriteFile(backupPath, data, 0o600); err != nil {
		return fmt.Errorf("写入备份: %w", err)
	}
	hash := sha256Hex(data)
	if record != nil {
		if err := record(absPath, backupPath, hash, int64(len(data))); err != nil {
			return fmt.Errorf("记录备份: %w", err)
		}
	}
	return nil
}

func stringArg(args map[string]any, key string) string {
	value, ok := args[key]
	if !ok || value == nil {
		return ""
	}
	text, _ := value.(string)
	return text
}

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

func boolArg(args map[string]any, key string) bool {
	value, ok := args[key]
	if !ok {
		return false
	}
	result, _ := value.(bool)
	return result
}

func errorResult(format string, args ...any) Result {
	return Result{Output: fmt.Sprintf(format, args...), IsError: true, Code: CodeToolError}
}
