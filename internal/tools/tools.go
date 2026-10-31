// Package tools 实现 Agent 可调用的内置工具。
//
// 所有文件路径都经过 policy.Resolve 校验；写入类工具在落盘前备份原文件。
// 工具输出默认截断到 32 KB，避免把大段二进制或日志塞进上下文。
package tools

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// Result 是工具执行结果。
type Result struct {
	Output  string         `json:"output"`
	IsError bool           `json:"isError"`
	Meta    map[string]any `json:"meta,omitempty"`
}

// Spec 是暴露给模型的工具定义。
type Spec struct {
	Name        string         `json:"name"`
	Description string         `json:"description"`
	Schema      map[string]any `json:"schema"`
}

// Tool 是内置工具的统一接口。
type Tool interface {
	Name() string
	Description() string
	Schema() map[string]any
	Execute(ctx context.Context, args map[string]any) Result
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
	opts  Options
	tools map[string]Tool
}

// NewRegistry 创建工具注册表。
func NewRegistry(opts Options) *Registry {
	registry := &Registry{opts: opts, tools: map[string]Tool{}}
	for _, tool := range []Tool{
		&fsListTool{registry: registry},
		&fsReadTool{registry: registry},
		&fsGlobTool{registry: registry},
		&fsGrepTool{registry: registry},
		&fsWriteTool{registry: registry},
		&fsEditTool{registry: registry},
		&shellExecTool{registry: registry},
		&gitStatusTool{registry: registry},
		&todoWriteTool{},
	} {
		registry.tools[tool.Name()] = tool
	}
	return registry
}

// SetWorkspace 切换工作区。
func (r *Registry) SetWorkspace(workspace string) {
	r.opts.Workspace = workspace
}

// Workspace 返回当前工作区。
func (r *Registry) Workspace() string {
	return r.opts.Workspace
}

// Get 按名称取工具。
func (r *Registry) Get(name string) (Tool, bool) {
	tool, ok := r.tools[name]
	return tool, ok
}

// Specs 返回所有工具定义，按名称排序保证提示词稳定。
func (r *Registry) Specs() []Spec {
	names := make([]string, 0, len(r.tools))
	for name := range r.tools {
		names = append(names, name)
	}
	sort.Strings(names)

	specs := make([]Spec, 0, len(names))
	for _, name := range names {
		tool := r.tools[name]
		specs = append(specs, Spec{Name: tool.Name(), Description: tool.Description(), Schema: tool.Schema()})
	}
	return specs
}

const (
	maxOutputBytes = 32 * 1024
	maxReadBytes   = 256 * 1024
	maxReadLines   = 2000
)

// IsIgnoredDir 判断目录是否应被文件树与搜索忽略。
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

// resolve 解析路径并返回绝对路径与展示用相对路径。
func (r *Registry) resolve(rel string) (abs string, display string, err error) {
	if strings.TrimSpace(rel) == "" {
		rel = "."
	}
	abs, err = resolvePath(r.opts.Workspace, rel)
	if err != nil {
		return "", "", err
	}
	return abs, displayPath(r.opts.Workspace, abs), nil
}

// backup 在写入前保存原文件内容。
func (r *Registry) backup(absPath string, data []byte) error {
	if r.opts.BackupDir == "" {
		return nil
	}
	if err := os.MkdirAll(r.opts.BackupDir, 0o700); err != nil {
		return fmt.Errorf("创建备份目录: %w", err)
	}
	backupPath := filepath.Join(r.opts.BackupDir, backupName(absPath))
	if err := os.WriteFile(backupPath, data, 0o600); err != nil {
		return fmt.Errorf("写入备份: %w", err)
	}
	hash := sha256Hex(data)
	if r.opts.RecordBackup != nil {
		if err := r.opts.RecordBackup(absPath, backupPath, hash, int64(len(data))); err != nil {
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
	return Result{Output: fmt.Sprintf(format, args...), IsError: true}
}
