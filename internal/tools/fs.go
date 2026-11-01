package tools

import (
	"context"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"

	"craftwork/internal/platform"
	"craftwork/internal/policy"
)

// ---------- fs_list ----------

type fsListTool struct{}

func (t *fsListTool) Spec() Spec {
	return Spec{
		Name:        "fs_list",
		Description: "列出工作区目录下的文件与子目录，默认只列一层。",
		Risk:        policy.R0,
		PathArg:     "path",
		Work:        true,
		SummaryArg:  "path",
		Schema: map[string]any{
			"type": "object",
			"properties": map[string]any{
				"path":  map[string]any{"type": "string", "description": "相对工作区的路径，默认为 ."},
				"depth": map[string]any{"type": "integer", "minimum": 1, "maximum": 3, "description": "递归深度，默认 1"},
			},
		},
	}
}

func (t *fsListTool) Execute(_ context.Context, call Call) Result {
	abs, display := call.Target, call.Display
	depth := intArg(call.Args, "depth", 1)
	if depth < 1 {
		depth = 1
	}
	if depth > 3 {
		depth = 3
	}

	info, err := os.Stat(abs)
	if err != nil {
		return errorResult("读取路径失败: %v", err)
	}
	if !info.IsDir() {
		return Result{Output: fmt.Sprintf("%s\t%s", display, humanSize(info.Size()))}
	}

	lines := []string{display + "/"}
	if err := walkList(abs, "  ", 1, depth, &lines); err != nil {
		return errorResult("列出目录失败: %v", err)
	}
	return Result{Output: strings.Join(lines, "\n")}
}

func walkList(dir, prefix string, level, maxLevel int, lines *[]string) error {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return err
	}
	sort.Slice(entries, func(i, j int) bool {
		if entries[i].IsDir() != entries[j].IsDir() {
			return entries[i].IsDir()
		}
		return entries[i].Name() < entries[j].Name()
	})

	for _, entry := range entries {
		name := entry.Name()
		if strings.HasPrefix(name, ".") && name != ".env.example" {
			continue
		}
		if entry.IsDir() {
			if isIgnoredDir(name) {
				continue
			}
			*lines = append(*lines, prefix+name+"/")
			if level < maxLevel {
				if err := walkList(filepath.Join(dir, name), prefix+"  ", level+1, maxLevel, lines); err != nil {
					return err
				}
			}
			continue
		}
		info, err := entry.Info()
		if err != nil {
			continue
		}
		*lines = append(*lines, fmt.Sprintf("%s%s\t%s", prefix, name, humanSize(info.Size())))
	}
	return nil
}

// ---------- fs_read ----------

type fsReadTool struct{}

func (t *fsReadTool) Spec() Spec {
	return Spec{
		Name:        "fs_read",
		Description: "读取工作区内的文本文件，返回带行号的内容。大文件请用 offset 与 limit 分段读取。",
		Risk:        policy.R0,
		PathArg:     "path",
		Work:        true,
		SummaryArg:  "path",
		Schema: map[string]any{
			"type":     "object",
			"required": []string{"path"},
			"properties": map[string]any{
				"path":   map[string]any{"type": "string", "description": "相对工作区的文件路径"},
				"offset": map[string]any{"type": "integer", "minimum": 0, "description": "起始行，从 0 开始"},
				"limit":  map[string]any{"type": "integer", "minimum": 1, "maximum": maxReadLines, "description": "读取行数，默认 2000"},
			},
		},
	}
}

func (t *fsReadTool) Execute(_ context.Context, call Call) Result {
	abs, display := call.Target, call.Display
	args := call.Args
	info, err := os.Stat(abs)
	if err != nil {
		return errorResult("读取文件失败: %v", err)
	}
	if info.IsDir() {
		return errorResult("%s 是目录，请使用 fs_list", display)
	}
	if info.Size() > 8*1024*1024 {
		return errorResult("文件过大（%s），请使用 offset 与 limit 分段读取", humanSize(info.Size()))
	}

	data, err := os.ReadFile(abs)
	if err != nil {
		return errorResult("读取文件失败: %v", err)
	}
	if platform.IsBinary(data) {
		return Result{Output: fmt.Sprintf("二进制文件 %s\n大小: %s\nSHA-256: %s", display, humanSize(info.Size()), platform.Sha256Hex(data))}
	}

	text := string(data)
	if len(text) > maxReadBytes {
		text = text[:maxReadBytes]
	}
	lines := strings.Split(text, "\n")
	offset := intArg(args, "offset", 0)
	if offset < 0 {
		offset = 0
	}
	if offset >= len(lines) {
		return Result{Output: fmt.Sprintf("%s 只有 %d 行，offset 超出范围", display, len(lines))}
	}
	limit := intArg(args, "limit", maxReadLines)
	if limit <= 0 || limit > maxReadLines {
		limit = maxReadLines
	}
	end := offset + limit
	if end > len(lines) {
		end = len(lines)
	}

	var builder strings.Builder
	for index := offset; index < end; index++ {
		fmt.Fprintf(&builder, "%5d│ %s\n", index+1, lines[index])
	}
	if end < len(lines) {
		fmt.Fprintf(&builder, "…（还有 %d 行，使用 offset=%d 继续读取）", len(lines)-end, end)
	}
	return Result{Output: builder.String()}
}

// ---------- fs_glob ----------

type fsGlobTool struct{}

func (t *fsGlobTool) Spec() Spec {
	return Spec{
		Name:        "fs_glob",
		Description: "按通配符查找工作区内的文件，支持 *.go、**/*.ts 等模式，最多返回 200 条。",
		Risk:        policy.R0,
		PathArg:     "path",
		Work:        true,
		SummaryArg:  "pattern",
		Schema: map[string]any{
			"type":     "object",
			"required": []string{"pattern"},
			"properties": map[string]any{
				"pattern": map[string]any{"type": "string", "description": "通配符模式，例如 **/*.go"},
				"path":    map[string]any{"type": "string", "description": "起始目录，默认工作区根"},
			},
		},
	}
}

func (t *fsGlobTool) Execute(_ context.Context, call Call) Result {
	pattern := stringArg(call.Args, "pattern")
	if pattern == "" {
		return errorResult("pattern 不能为空")
	}
	root := call.Target

	var matches []string
	walkErr := filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if entry.IsDir() {
			if path != root && isIgnoredDir(entry.Name()) {
				return filepath.SkipDir
			}
			return nil
		}
		rel := displayPath(call.Workspace, path)
		if matchGlob(pattern, rel) {
			matches = append(matches, rel)
			if len(matches) >= 200 {
				return fs.SkipAll
			}
		}
		return nil
	})
	if walkErr != nil {
		return errorResult("查找文件失败: %v", walkErr)
	}
	if len(matches) == 0 {
		return Result{Output: "没有匹配的文件"}
	}
	sort.Strings(matches)
	return Result{Output: strings.Join(matches, "\n")}
}

func matchGlob(pattern, rel string) bool {
	pattern = filepath.ToSlash(pattern)
	rel = filepath.ToSlash(rel)
	if index := strings.Index(pattern, "**/"); index >= 0 {
		suffix := pattern[index+3:]
		if matched, _ := filepath.Match(suffix, filepath.Base(rel)); matched {
			return true
		}
		return strings.HasSuffix(rel, strings.TrimPrefix(suffix, "*"))
	}
	if matched, _ := filepath.Match(pattern, rel); matched {
		return true
	}
	matched, _ := filepath.Match(pattern, filepath.Base(rel))
	return matched
}

// ---------- fs_grep ----------

type fsGrepTool struct{}

func (t *fsGrepTool) Spec() Spec {
	return Spec{
		Name:        "fs_grep",
		Description: "在工作区文本文件中搜索字符串或正则，返回 path:line: 内容，最多 200 条。",
		Risk:        policy.R0,
		PathArg:     "path",
		Work:        true,
		SummaryArg:  "pattern",
		Schema: map[string]any{
			"type":     "object",
			"required": []string{"pattern"},
			"properties": map[string]any{
				"pattern": map[string]any{"type": "string", "description": "要搜索的字符串或正则"},
				"path":    map[string]any{"type": "string", "description": "搜索目录，默认工作区根"},
				"regex":   map[string]any{"type": "boolean", "description": "是否按正则解释，默认 false"},
			},
		},
	}
}

func (t *fsGrepTool) Execute(_ context.Context, call Call) Result {
	args := call.Args
	pattern := stringArg(args, "pattern")
	if pattern == "" {
		return errorResult("pattern 不能为空")
	}
	root := call.Target

	var matcher *regexp.Regexp
	if boolArg(args, "regex") {
		compiled, err := regexp.Compile(pattern)
		if err != nil {
			return errorResult("正则无效: %v", err)
		}
		matcher = compiled
	}

	var lines []string
	walkErr := filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if entry.IsDir() {
			if path != root && isIgnoredDir(entry.Name()) {
				return filepath.SkipDir
			}
			return nil
		}
		info, err := entry.Info()
		if err != nil || info.Size() > 1024*1024 {
			return nil
		}
		data, err := os.ReadFile(path)
		if err != nil || platform.IsBinary(data) {
			return nil
		}
		rel := displayPath(call.Workspace, path)
		for index, line := range strings.Split(string(data), "\n") {
			matched := false
			if matcher != nil {
				matched = matcher.MatchString(line)
			} else {
				matched = strings.Contains(line, pattern)
			}
			if !matched {
				continue
			}
			lines = append(lines, fmt.Sprintf("%s:%d: %s", rel, index+1, strings.TrimSpace(line)))
			if len(lines) >= 200 {
				return fs.SkipAll
			}
		}
		return nil
	})
	if walkErr != nil {
		return errorResult("搜索失败: %v", walkErr)
	}
	if len(lines) == 0 {
		return Result{Output: "没有匹配内容"}
	}
	return Result{Output: truncate(strings.Join(lines, "\n"), maxOutputBytes)}
}

// ---------- fs_write ----------

type fsWriteTool struct{}

func (t *fsWriteTool) Spec() Spec {
	return Spec{
		Name:        "fs_write",
		Description: "写入工作区内的文件。已存在的文件会先备份，写入使用临时文件原子替换。",
		Risk:        policy.R1,
		PathArg:     "path",
		Work:        true,
		Title:       "写入文件",
		SummaryArg:  "path",
		Schema: map[string]any{
			"type":     "object",
			"required": []string{"path", "content"},
			"properties": map[string]any{
				"path":    map[string]any{"type": "string", "description": "相对工作区的文件路径"},
				"content": map[string]any{"type": "string", "description": "完整文件内容"},
			},
		},
	}
}

func (t *fsWriteTool) Execute(_ context.Context, call Call) Result {
	content := stringArg(call.Args, "content")
	abs, display := call.Target, call.Display

	mode := os.FileMode(0o644)
	if existing, err := os.ReadFile(abs); err == nil {
		mode = 0o644
		if info, statErr := os.Stat(abs); statErr == nil {
			mode = info.Mode().Perm()
		}
		if err := call.Backup(existing); err != nil {
			return errorResult("写入前备份失败: %v", err)
		}
	}
	if err := os.MkdirAll(filepath.Dir(abs), 0o755); err != nil {
		return errorResult("创建目录失败: %v", err)
	}
	if err := platform.WriteFileAtomic(abs, []byte(content), mode); err != nil {
		return errorResult("写入失败: %v", err)
	}
	return Result{
		Output: fmt.Sprintf("已写入 %s（%s）", display, humanSize(int64(len(content)))),
		Meta:   map[string]any{"path": display, "bytes": len(content)},
	}
}

// ---------- fs_edit ----------

type fsEditTool struct{}

func (t *fsEditTool) Spec() Spec {
	return Spec{
		Name:        "fs_edit",
		Description: "在文件中精确替换一段文本。old_string 必须在文件中唯一匹配；写入前会备份原文件。",
		Risk:        policy.R1,
		PathArg:     "path",
		Work:        true,
		Title:       "修改文件",
		SummaryArg:  "path",
		Schema: map[string]any{
			"type":     "object",
			"required": []string{"path", "old_string", "new_string"},
			"properties": map[string]any{
				"path":       map[string]any{"type": "string", "description": "相对工作区的文件路径"},
				"old_string": map[string]any{"type": "string", "description": "要被替换的原文，必须唯一"},
				"new_string": map[string]any{"type": "string", "description": "替换后的文本"},
			},
		},
	}
}

func (t *fsEditTool) Execute(_ context.Context, call Call) Result {
	args := call.Args
	oldText := stringArg(args, "old_string")
	newText := stringArg(args, "new_string")
	if oldText == "" {
		return errorResult("old_string 不能为空")
	}
	abs, display := call.Target, call.Display
	data, err := os.ReadFile(abs)
	if err != nil {
		return errorResult("读取文件失败: %v", err)
	}
	content := string(data)
	count := strings.Count(content, oldText)
	if count == 0 {
		return errorResult("在 %s 中没有找到要替换的内容；请先读取文件确认原文", display)
	}
	if count > 1 {
		return errorResult("在 %s 中匹配到 %d 处相同内容，请提供更长的唯一片段", display, count)
	}
	if err := call.Backup(data); err != nil {
		return errorResult("写入前备份失败: %v", err)
	}
	updated := strings.Replace(content, oldText, newText, 1)
	mode := os.FileMode(0o644)
	if info, statErr := os.Stat(abs); statErr == nil {
		mode = info.Mode().Perm()
	}
	if err := platform.WriteFileAtomic(abs, []byte(updated), mode); err != nil {
		return errorResult("写入失败: %v", err)
	}
	return Result{
		Output: fmt.Sprintf("已替换 %s 中的 1 处内容（%s → %s）", display, humanSize(int64(len(content))), humanSize(int64(len(updated)))),
		Meta:   map[string]any{"path": display},
	}
}

// ---------- todo_write ----------

type todoWriteTool struct{}

func (t *todoWriteTool) Spec() Spec {
	return Spec{
		Name:        "todo_write",
		Description: "更新当前任务的待办清单，用于向用户展示进度。",
		Risk:        policy.R0,
		Work:        true,
		SummaryText: "更新任务清单",
		Schema: map[string]any{
			"type":     "object",
			"required": []string{"items"},
			"properties": map[string]any{
				"items": map[string]any{
					"type": "array",
					"items": map[string]any{
						"type":     "object",
						"required": []string{"text", "status"},
						"properties": map[string]any{
							"text":   map[string]any{"type": "string"},
							"status": map[string]any{"type": "string", "enum": []string{"pending", "active", "done"}},
						},
					},
				},
			},
		},
	}
}

func (t *todoWriteTool) Execute(_ context.Context, call Call) Result {
	items, _ := call.Args["items"].([]any)
	var lines []string
	for _, item := range items {
		object, ok := item.(map[string]any)
		if !ok {
			continue
		}
		status, _ := object["status"].(string)
		text, _ := object["text"].(string)
		mark := "○"
		switch status {
		case "done":
			mark = "●"
		case "active":
			mark = "◐"
		}
		lines = append(lines, mark+" "+text)
	}
	if len(lines) == 0 {
		return errorResult("items 不能为空")
	}
	return Result{Output: strings.Join(lines, "\n")}
}
