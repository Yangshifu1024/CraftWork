// Package editor 提供行级差异计算。结果直接给前端渲染统一或并排视图。
//
// 本文件实现行级差异算法：先剪掉公共前缀与后缀，再对中间部分用最长公共
// 子序列（LCS）动态规划求最短编辑序列，最后按上下文行数切分成差异块
// （Hunk，即一组连续的差异行）。
package editor

import (
	"fmt"
	"strings"
)

// Line 是差异中的一行。
type Line struct {
	// Kind 是行类型：ctx 表示上下文、add 表示新增、del 表示删除。
	Kind string `json:"kind"` // ctx / add / del
	// Old 是这一行在旧文本中的行号，从 1 开始；新增行为 0。
	Old int `json:"old,omitempty"`
	// New 是这一行在新文本中的行号，从 1 开始；删除行为 0。
	New int `json:"new,omitempty"`
	// Text 是行内容，不含换行符。
	Text string `json:"text"`
}

// Hunk 是一组连续的差异行。
type Hunk struct {
	// Header 是差异块头，格式与统一差异格式一致，例如
	// “@@ -1,3 +1,4 @@”。两个数字分别是起始行号与行数。
	Header string `json:"header"`
	// Lines 是差异块内的行，包含变更行与前后若干上下文行。
	Lines []Line `json:"lines"`
}

// Result 是完整的差异结果。
type Result struct {
	// Additions 是新增行总数。
	Additions int `json:"additions"`
	// Deletions 是删除行总数。
	Deletions int `json:"deletions"`
	// Hunks 是按原文顺序排列的差异块。
	Hunks []Hunk `json:"hunks"`
	// Truncated 表示文本过大，未计算差异；此时 Hunks 为空。
	Truncated bool `json:"truncated"`
}

// 差异计算的规模限制。超过限制时退化处理，避免占用过多内存与时间。
const (
	// maxDiffLines 是参与计算的最大行数，任一侧超过即放弃计算。
	maxDiffLines = 20000
	// maxDiffCells 是动态规划表格的最大单元格数（旧行数乘新行数）。
	maxDiffCells = 4_000_000
	// contextLines 是每个差异块保留的上下文行数。
	contextLines = 3
)

// Diff 计算两段文本的行级差异。
//
// 处理步骤：
//  1. 按行切分并去掉末尾换行；
//  2. 任一侧超过行数上限时直接标记为截断，不计算差异；
//  3. 剪掉公共前缀与公共后缀，只对中间部分做动态规划；
//  4. 统计新增与删除行数，并按上下文行数切分成差异块。
func Diff(oldText, newText string) Result {
	oldLines := splitLines(oldText)
	newLines := splitLines(newText)

	result := Result{}
	if len(oldLines) > maxDiffLines || len(newLines) > maxDiffLines {
		result.Truncated = true
		return result
	}

	// 公共前缀通常是文件开头未改动的部分，剪掉后能显著缩小动态规划规模。
	prefix := 0
	for prefix < len(oldLines) && prefix < len(newLines) && oldLines[prefix] == newLines[prefix] {
		prefix++
	}
	// 公共后缀同理；比较范围要扣掉已剪掉的前缀，避免前后重叠。
	suffix := 0
	for suffix < len(oldLines)-prefix && suffix < len(newLines)-prefix &&
		oldLines[len(oldLines)-1-suffix] == newLines[len(newLines)-1-suffix] {
		suffix++
	}

	// operations 是完整的逐行操作序列，上下文行与变更行都在里面。
	operations := make([]Line, 0, len(oldLines)+len(newLines))
	for index := 0; index < prefix; index++ {
		operations = append(operations, Line{Kind: "ctx", Old: index + 1, New: index + 1, Text: oldLines[index]})
	}
	// 中间部分的行号偏移等于前缀长度，新旧两侧相同。
	operations = append(operations, diffMiddle(
		oldLines[prefix:len(oldLines)-suffix],
		newLines[prefix:len(newLines)-suffix],
		prefix, prefix,
	)...)
	// 补齐公共后缀。新旧两侧的后缀长度相同，但起止行号可能不同。
	for index := 0; index < suffix; index++ {
		oldIndex := len(oldLines) - suffix + index
		newIndex := len(newLines) - suffix + index
		operations = append(operations, Line{Kind: "ctx", Old: oldIndex + 1, New: newIndex + 1, Text: oldLines[oldIndex]})
	}

	// 统计变更行数，供界面显示“+N -M”。
	for _, operation := range operations {
		switch operation.Kind {
		case "add":
			result.Additions++
		case "del":
			result.Deletions++
		}
	}
	result.Hunks = buildHunks(operations)
	return result
}

// diffMiddle 对剪掉公共前后缀后的中间部分求差异。
//
// 先处理一侧为空、以及动态规划规模超限两种退化情况；否则用最长公共子序列
// 表格回溯出操作序列。oldOffset 与 newOffset 是中间部分在原文中的行号偏移。
func diffMiddle(oldLines, newLines []string, oldOffset, newOffset int) []Line {
	n := len(oldLines)
	m := len(newLines)
	if n == 0 && m == 0 {
		return nil
	}
	if n == 0 {
		// 旧文本为空：全部是新增行。
		result := make([]Line, 0, m)
		for index, text := range newLines {
			result = append(result, Line{Kind: "add", New: newOffset + index + 1, Text: text})
		}
		return result
	}
	if m == 0 {
		// 新文本为空：全部是删除行。
		result := make([]Line, 0, n)
		for index, text := range oldLines {
			result = append(result, Line{Kind: "del", Old: oldOffset + index + 1, Text: text})
		}
		return result
	}
	if n*m > maxDiffCells {
		// 超大改动退化为“整段删除 + 整段新增”，避免消耗过多内存。
		result := make([]Line, 0, n+m)
		for index, text := range oldLines {
			result = append(result, Line{Kind: "del", Old: oldOffset + index + 1, Text: text})
		}
		for index, text := range newLines {
			result = append(result, Line{Kind: "add", New: newOffset + index + 1, Text: text})
		}
		return result
	}

	// table[i][j] 表示 oldLines[i:] 与 newLines[j:] 的最长公共子序列长度。
	// 从右下角往左上填表，回溯时即可从左到右输出操作序列。
	table := make([][]int, n+1)
	for index := range table {
		table[index] = make([]int, m+1)
	}
	for i := n - 1; i >= 0; i-- {
		for j := m - 1; j >= 0; j-- {
			if oldLines[i] == newLines[j] {
				table[i][j] = table[i+1][j+1] + 1
			} else if table[i+1][j] >= table[i][j+1] {
				// 两行不同：继承下方或右侧的较大值；相等时优先走删除，
				// 保证同一输入每次得到相同的差异。
				table[i][j] = table[i+1][j]
			} else {
				table[i][j] = table[i][j+1]
			}
		}
	}

	// 回溯表格生成操作序列：行相同记上下文，否则按表格走向记删除或新增。
	result := make([]Line, 0, n+m)
	i, j := 0, 0
	for i < n && j < m {
		if oldLines[i] == newLines[j] {
			result = append(result, Line{Kind: "ctx", Old: oldOffset + i + 1, New: newOffset + j + 1, Text: oldLines[i]})
			i++
			j++
			continue
		}
		if table[i+1][j] >= table[i][j+1] {
			result = append(result, Line{Kind: "del", Old: oldOffset + i + 1, Text: oldLines[i]})
			i++
		} else {
			result = append(result, Line{Kind: "add", New: newOffset + j + 1, Text: newLines[j]})
			j++
		}
	}
	// 任意一侧先走完时，剩余行分别记为删除或新增。
	for ; i < n; i++ {
		result = append(result, Line{Kind: "del", Old: oldOffset + i + 1, Text: oldLines[i]})
	}
	for ; j < m; j++ {
		result = append(result, Line{Kind: "add", New: newOffset + j + 1, Text: newLines[j]})
	}
	return result
}

// buildHunks 把逐行操作序列切分成差异块。
//
// 每个变更行先扩展出前后 contextLines 行上下文，形成窗口；重叠或相邻的
// 窗口合并，最终每块输出一个统一差异格式的块头。
func buildHunks(operations []Line) []Hunk {
	// 先找出所有变更行的下标；没有变更时返回空表示两段文本相同。
	var changed []int
	for index, operation := range operations {
		if operation.Kind != "ctx" {
			changed = append(changed, index)
		}
	}
	if len(changed) == 0 {
		return nil
	}

	// window 是一个差异块在操作序列中的闭区间。
	type window struct{ start, end int }
	windows := make([]window, 0, len(changed))
	for _, index := range changed {
		// 每个变更行前后各留 contextLines 行上下文，并夹在序列边界内。
		start := index - contextLines
		if start < 0 {
			start = 0
		}
		end := index + contextLines
		if end >= len(operations) {
			end = len(operations) - 1
		}
		// 与上一个窗口重叠或紧邻时合并成一个，避免产生大量碎片块。
		if len(windows) > 0 && start <= windows[len(windows)-1].end+1 {
			if end > windows[len(windows)-1].end {
				windows[len(windows)-1].end = end
			}
			continue
		}
		windows = append(windows, window{start: start, end: end})
	}

	hunks := make([]Hunk, 0, len(windows))
	for _, item := range windows {
		lines := operations[item.start : item.end+1]
		// 统计块内新旧两侧的起始行号与行数，用于生成块头。
		oldStart, newStart := 1, 1
		oldCount, newCount := 0, 0
		for _, line := range lines {
			if line.Old > 0 {
				if oldCount == 0 {
					oldStart = line.Old
				}
				oldCount++
			}
			if line.New > 0 {
				if newCount == 0 {
					newStart = line.New
				}
				newCount++
			}
		}
		// 纯新增块没有旧行、纯删除块没有新行，起始行号按统一差异惯例写 0。
		if oldCount == 0 {
			oldStart = 0
		}
		if newCount == 0 {
			newStart = 0
		}
		hunks = append(hunks, Hunk{
			Header: fmt.Sprintf("@@ -%d,%d +%d,%d @@", oldStart, oldCount, newStart, newCount),
			Lines:  lines,
		})
	}
	return hunks
}

// splitLines 把文本切分成行。先把 Windows 换行统一成换行符，再去掉末尾
// 换行，避免最后多出一个空行；空文本返回 nil。
func splitLines(text string) []string {
	text = strings.ReplaceAll(text, "\r\n", "\n")
	text = strings.TrimSuffix(text, "\n")
	if text == "" {
		return nil
	}
	return strings.Split(text, "\n")
}
