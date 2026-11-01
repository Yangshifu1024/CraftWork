// 本文件负责文件系统类工具：列目录、读文件、通配符匹配、内容搜索、
// 写入与精确替换。读工具只访问工作区内的路径；写工具在落盘前备份，
// 并使用临时文件原子替换，避免中途失败留下半截文件。

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

// fsListTool 列出工作区目录内容，只读、自动放行。
type fsListTool struct{}

// Spec 声明 fs_list 的元数据：路径参数为 path，属于只读操作。
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

// Execute 输出目录树或单个文件的尺寸信息。
func (t *fsListTool) Execute(_ context.Context, call Call) Result {
	abs, display := call.Target, call.Display
	// depth 由模型传入，这里再次夹取到 1–3，防止一次展开过深。
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

// walkList 递归收集目录条目，隐藏文件与忽略目录会被跳过。
//
// 目录排在文件之前、同类按名称排序，保证同一次调用的输出顺序稳定。
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

// fsReadTool 读取文本文件并输出带行号的内容，只读。
type fsReadTool struct{}

// Spec 声明 fs_read 的元数据：必须提供 path，offset 与 limit 用于分页。
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

// Execute 读取文件内容，按 offset/limit 截取后加上行号返回。
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
	// 超大文件直接拒绝，避免读入内存后立刻被截断，浪费时间和内存。
	if info.Size() > 8*1024*1024 {
		return errorResult("文件过大（%s），请使用 offset 与 limit 分段读取", humanSize(info.Size()))
	}

	data, err := os.ReadFile(abs)
	if err != nil {
		return errorResult("读取文件失败: %v", err)
	}
	// 二进制文件只返回大小与哈希，不把不可读内容塞进上下文。
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

// fsGlobTool 按通配符查找文件，只读。
type fsGlobTool struct{}

// Spec 声明 fs_glob 的元数据：pattern 是必填参数，path 指定起始目录。
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

// Execute 遍历起始目录，返回最多 200 条相对路径匹配结果。
func (t *fsGlobTool) Execute(_ context.Context, call Call) Result {
	pattern := stringArg(call.Args, "pattern")
	if pattern == "" {
		return errorResult("pattern 不能为空")
	}
	root := call.Target

	var matches []string
	// 遍历时跳过依赖与构建产物目录；收集到 200 条立即停止，
	// 避免在大仓库里做无意义的全量扫描。
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

// matchGlob 判断相对路径是否匹配通配符。
//
// 先按完整相对路径匹配，再退回按文件名匹配；包含 **/ 的模式会拆成
// 后缀匹配，满足“任意层级下的某类文件”这种常见需求。
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

// fsGrepTool 在工作区文本文件中搜索字符串或正则，只读。
type fsGrepTool struct{}

// Spec 声明 fs_grep 的元数据：pattern 必填，regex 控制是否按正则解释。
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

// Execute 遍历文本文件逐行匹配，返回 path:line: 内容，最多 200 条。
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
		// 跳过目录项大小未知、超过 1 MB 的文件与二进制文件，
		// 搜索只在适合放进上下文的纯文本上进行。
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

// fsWriteTool 写入整份文件内容，属于写工作区操作。
type fsWriteTool struct{}

// Spec 声明 fs_write 的元数据：path 与 content 必填，风险等级 R1。
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

// Execute 写入文件：已存在时先备份并保留原权限，再用临时文件原子替换。
func (t *fsWriteTool) Execute(_ context.Context, call Call) Result {
	content := stringArg(call.Args, "content")
	abs, display := call.Target, call.Display

	mode := os.FileMode(0o644)
	if existing, err := os.ReadFile(abs); err == nil {
		mode = 0o644
		// 保留原文件权限，避免覆盖后把可执行脚本变成普通文件。
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
	// 先写同目录临时文件再重命名，写入中断不会留下半截内容。
	if err := platform.WriteFileAtomic(abs, []byte(content), mode); err != nil {
		return errorResult("写入失败: %v", err)
	}
	return Result{
		Output: fmt.Sprintf("已写入 %s（%s）", display, humanSize(int64(len(content)))),
		Meta:   map[string]any{"path": display, "bytes": len(content)},
	}
}

// ---------- fs_edit ----------

// fsEditTool 在文件中精确替换一段文本，属于写工作区操作。
type fsEditTool struct{}

// Spec 声明 fs_edit 的元数据：old_string 必须唯一匹配，风险等级 R1。
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

// Execute 校验唯一匹配后替换一次；匹配不到或匹配多处都直接报错，
// 避免模型在不确定的上下文里误改代码。
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

// todoWriteTool 接收并回显任务清单，用于向用户展示当前进度。
type todoWriteTool struct{}

// Spec 声明 todo_write 的元数据：items 是必填数组，只读操作。
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

// Execute 把待办项渲染成带状态符号的文本；空清单视为参数错误。
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
		// 用符号区分待办、进行中与已完成，前端与模型都能直接读懂。
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
