// 本文件负责 apply_patch 工具：解析统一 diff 补丁，按补丁块匹配原文件，
// 支持预演（dry_run）与只应用指定块。匹配时先按声明的行号定位，
// 找不到再从文件开头搜索，容忍少量行号偏移。

package tools

import (
	"context"
	"fmt"
	"os"
	"regexp"
	"sort"
	"strconv"
	"strings"

	"github.com/yangshifu1024/craftwork/internal/platform"
	"github.com/yangshifu1024/craftwork/internal/policy"
)

// hunkHeaderPattern 匹配 @@ -旧起始,旧行数 +新起始,新行数 @@ 形式的块头，
// 行数部分可以省略，省略时按 1 行处理。
var hunkHeaderPattern = regexp.MustCompile(`^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@`)

// patchHunk 是解析后的一个补丁块，保存两侧的行内容与起止位置。
type patchHunk struct {
	// oldStart 是原文件中的起始行号（从 1 开始）。
	oldStart int
	// oldCount 是块头声明的原文件行数，用于校验解析结果。
	oldCount int
	// newStart 是应用后新文件的起始行号。
	newStart int
	// newCount 是块头声明的新文件行数。
	newCount int
	// oldLines 是块中属于原文件的行（上下文行与删除行）。
	oldLines []string
	// newLines 是块中属于新文件的行（上下文行与新增行）。
	newLines []string
	// additions 是新增行数，仅用于结果展示。
	additions int
	// deletions 是删除行数，仅用于结果展示。
	deletions int
}

// applyPatchTool 在单个文件上预演或应用统一 diff 补丁。
type applyPatchTool struct{}

// Spec 声明 apply_patch 的元数据：path 与 patch 必填，风险等级 R1。
func (t *applyPatchTool) Spec() Spec {
	return Spec{
		Name:        "apply_patch",
		Description: "把统一 diff 补丁应用到工作区内的一个文件。支持 dry_run 预演和 hunks 逐块应用；写入前会备份原文件。",
		Risk:        policy.R1,
		PathArg:     "path",
		Work:        true,
		Title:       "应用补丁",
		SummaryArg:  "path",
		Schema: map[string]any{
			"type":     "object",
			"required": []string{"path", "patch"},
			"properties": map[string]any{
				"path":    map[string]any{"type": "string", "description": "要修改的文件（相对工作区）"},
				"patch":   map[string]any{"type": "string", "description": "统一 diff，包含一个或多个 @@ 块"},
				"dry_run": map[string]any{"type": "boolean", "description": "只预演，不写入文件"},
				"hunks": map[string]any{
					"type":        "array",
					"items":       map[string]any{"type": "integer"},
					"description": "只应用指定补丁块（从 1 开始）；省略表示全部",
				},
			},
		},
	}
}

// Execute 解析补丁并尝试应用；dry_run 只报告能否应用，不写文件。
func (t *applyPatchTool) Execute(_ context.Context, call Call) Result {
	args := call.Args
	patch := stringArg(args, "patch")
	if strings.TrimSpace(patch) == "" {
		return errorResult("patch 不能为空")
	}
	// 先完整解析与选择块，任何格式错误都在读取文件之前暴露。
	hunks, err := parseUnifiedPatch(patch)
	if err != nil {
		return errorResult("解析补丁失败: %v", err)
	}
	selected, err := selectedHunks(args, len(hunks))
	if err != nil {
		return errorResult("%v", err)
	}

	abs, display := call.Target, call.Display
	// apply_patch 只修改已存在的文件，不创建新文件。
	data, err := os.ReadFile(abs)
	if err != nil {
		return errorResult("读取文件失败（apply_patch 只修改已有文件）: %v", err)
	}

	updated, applied, additions, deletions, err := applyHunks(string(data), hunks, selected)
	if err != nil {
		return errorResult("补丁无法应用: %v", err)
	}
	dryRun := boolArg(args, "dry_run")
	if dryRun {
		return Result{
			Output: fmt.Sprintf("补丁预演通过：%s 可应用 %d 个块（+%d −%d 行），未写入文件。",
				display, applied, additions, deletions),
			Meta: map[string]any{
				"path": display, "hunks": applied, "additions": additions,
				"deletions": deletions, "dryRun": true,
			},
		}
	}

	if err := call.Backup(data); err != nil {
		return errorResult("写入前备份失败: %v", err)
	}
	mode := os.FileMode(0o644)
	if info, statErr := os.Stat(abs); statErr == nil {
		mode = info.Mode().Perm()
	}
	if err := platform.WriteFileAtomic(abs, []byte(updated), mode); err != nil {
		return errorResult("写入失败: %v", err)
	}
	return Result{
		Output: fmt.Sprintf("已应用 %d 个补丁块到 %s（+%d −%d 行）。", applied, display, additions, deletions),
		Meta: map[string]any{
			"path": display, "hunks": applied, "additions": additions,
			"deletions": deletions,
		},
	}
}

// parseUnifiedPatch 把统一 diff 文本解析成补丁块列表。
//
// 统一换行符后逐行处理：块头开启新块，空格开头是上下文行，
// 减号开头是删除行，加号开头是新增行；以反斜杠开头的提示行忽略。
// 解析完成后校验每个块的行数与块头声明一致，防止补丁本身有误。
func parseUnifiedPatch(patch string) ([]patchHunk, error) {
	normalized := strings.ReplaceAll(patch, "\r\n", "\n")
	lines := strings.Split(normalized, "\n")
	if len(lines) > 0 && lines[len(lines)-1] == "" && strings.HasSuffix(normalized, "\n") {
		lines = lines[:len(lines)-1]
	}
	var hunks []patchHunk
	current := -1
	for _, line := range lines {
		if match := hunkHeaderPattern.FindStringSubmatch(line); match != nil {
			hunks = append(hunks, patchHunk{
				oldStart: atoi(match[1]),
				oldCount: countOrDefault(match[2]),
				newStart: atoi(match[3]),
				newCount: countOrDefault(match[4]),
			})
			current = len(hunks) - 1
			continue
		}
		if current < 0 {
			// 第一个 @@ 之前是文件头，忽略。
			continue
		}
		if strings.HasPrefix(line, `\`) {
			continue
		}
		if line == "" {
			hunks[current].oldLines = append(hunks[current].oldLines, "")
			hunks[current].newLines = append(hunks[current].newLines, "")
			continue
		}
		switch line[0] {
		case ' ':
			hunks[current].oldLines = append(hunks[current].oldLines, line[1:])
			hunks[current].newLines = append(hunks[current].newLines, line[1:])
		case '-':
			hunks[current].oldLines = append(hunks[current].oldLines, line[1:])
			hunks[current].deletions++
		case '+':
			hunks[current].newLines = append(hunks[current].newLines, line[1:])
			hunks[current].additions++
		default:
			return nil, fmt.Errorf("无法解析补丁行: %q", line)
		}
	}
	if len(hunks) == 0 {
		return nil, fmt.Errorf("没有找到 @@ 补丁块")
	}
	for index, hunk := range hunks {
		if hunk.oldCount > 0 && len(hunk.oldLines) != hunk.oldCount {
			return nil, fmt.Errorf("第 %d 个补丁块的旧内容行数不符（声明 %d，实际 %d）",
				index+1, hunk.oldCount, len(hunk.oldLines))
		}
		if hunk.newCount > 0 && len(hunk.newLines) != hunk.newCount {
			return nil, fmt.Errorf("第 %d 个补丁块的新内容行数不符（声明 %d，实际 %d）",
				index+1, hunk.newCount, len(hunk.newLines))
		}
	}
	return hunks, nil
}

// selectedHunks 解析 hunks 参数，返回去重排序后的块序号。
// 返回 nil 表示未指定，调用方应应用全部块。
func selectedHunks(args map[string]any, total int) ([]int, error) {
	raw, ok := args["hunks"].([]any)
	if !ok || len(raw) == 0 {
		return nil, nil
	}
	seen := map[int]bool{}
	for _, item := range raw {
		var index int
		switch value := item.(type) {
		case float64:
			index = int(value)
		case int:
			index = value
		case int64:
			index = int(value)
		default:
			return nil, fmt.Errorf("hunks 必须是整数数组")
		}
		if index < 1 || index > total {
			return nil, fmt.Errorf("补丁块序号 %d 超出范围 1–%d", index, total)
		}
		seen[index] = true
	}
	result := make([]int, 0, len(seen))
	for index := range seen {
		result = append(result, index)
	}
	sort.Ints(result)
	return result, nil
}

// applyHunks 按顺序应用选中的补丁块，返回新内容与增删统计。
//
// 每个块优先在声明的行号附近匹配，匹配不到再从文件开头重新搜索；
// 已经应用过的块会通过 delta 修正后续块的偏移量。
func applyHunks(content string, hunks []patchHunk, selected []int) (string, int, int, int, error) {
	chosen := map[int]bool{}
	for _, index := range selected {
		chosen[index] = true
	}
	applyAll := len(selected) == 0

	lines := strings.Split(content, "\n")
	applied := 0
	additions := 0
	deletions := 0
	delta := 0

	for index, hunk := range hunks {
		if !applyAll && !chosen[index+1] {
			continue
		}
		position := hunk.oldStart - 1 + delta
		if position < 0 {
			position = 0
		}
		if len(hunk.oldLines) == 0 {
			// 纯新增块没有上下文可匹配，直接按修正后的位置插入。
			if position > len(lines) {
				position = len(lines)
			}
			lines = insertLines(lines, position, hunk.newLines)
		} else {
			found := findLines(lines, hunk.oldLines, position)
			if found < 0 {
				// 行号有偏差时退回全文件搜索，但必须找到唯一且连续的匹配。
				found = findLines(lines, hunk.oldLines, 0)
			}
			if found < 0 {
				return "", 0, 0, 0, fmt.Errorf("第 %d 个补丁块找不到匹配的上下文", index+1)
			}
			lines = replaceLines(lines, found, len(hunk.oldLines), hunk.newLines)
		}
		applied++
		additions += hunk.additions
		deletions += hunk.deletions
		delta += len(hunk.newLines) - len(hunk.oldLines)
	}
	if applied == 0 {
		return "", 0, 0, 0, fmt.Errorf("没有可应用的补丁块")
	}
	return strings.Join(lines, "\n"), applied, additions, deletions, nil
}

// findLines 从 start 开始查找 target 连续出现的首个位置，找不到返回 -1。
func findLines(lines, target []string, start int) int {
	if len(target) == 0 {
		return start
	}
	if start < 0 {
		start = 0
	}
	for index := start; index+len(target) <= len(lines); index++ {
		matched := true
		for offset := range target {
			if lines[index+offset] != target[offset] {
				matched = false
				break
			}
		}
		if matched {
			return index
		}
	}
	return -1
}

// replaceLines 用 replacement 替换从 start 开始的 count 行。
func replaceLines(lines []string, start, count int, replacement []string) []string {
	result := make([]string, 0, len(lines)-count+len(replacement))
	result = append(result, lines[:start]...)
	result = append(result, replacement...)
	result = append(result, lines[start+count:]...)
	return result
}

// insertLines 在 position 处插入若干行，不删除原有内容。
func insertLines(lines []string, position int, inserted []string) []string {
	return replaceLines(lines, position, 0, inserted)
}

// atoi 把十进制字符串转成整数，解析失败时返回 0。
func atoi(value string) int {
	number, _ := strconv.Atoi(value)
	return number
}

// countOrDefault 解析块头里的行数，省略时按 1 行处理。
func countOrDefault(value string) int {
	if value == "" {
		return 1
	}
	return atoi(value)
}
