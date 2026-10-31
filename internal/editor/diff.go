// Package editor 提供行级差异计算。结果直接给前端渲染统一或并排视图。
package editor

import (
	"fmt"
	"strings"
)

// Line 是差异中的一行。
type Line struct {
	Kind string `json:"kind"` // ctx / add / del
	Old  int    `json:"old,omitempty"`
	New  int    `json:"new,omitempty"`
	Text string `json:"text"`
}

// Hunk 是一组连续的差异行。
type Hunk struct {
	Header string `json:"header"`
	Lines  []Line `json:"lines"`
}

// Result 是完整的差异结果。
type Result struct {
	Additions int    `json:"additions"`
	Deletions int    `json:"deletions"`
	Hunks     []Hunk `json:"hunks"`
	Truncated bool   `json:"truncated"`
}

const (
	maxDiffLines = 20000
	maxDiffCells = 4_000_000
	contextLines = 3
)

// Diff 计算两段文本的行级差异。
func Diff(oldText, newText string) Result {
	oldLines := splitLines(oldText)
	newLines := splitLines(newText)

	result := Result{}
	if len(oldLines) > maxDiffLines || len(newLines) > maxDiffLines {
		result.Truncated = true
		return result
	}

	prefix := 0
	for prefix < len(oldLines) && prefix < len(newLines) && oldLines[prefix] == newLines[prefix] {
		prefix++
	}
	suffix := 0
	for suffix < len(oldLines)-prefix && suffix < len(newLines)-prefix &&
		oldLines[len(oldLines)-1-suffix] == newLines[len(newLines)-1-suffix] {
		suffix++
	}

	operations := make([]Line, 0, len(oldLines)+len(newLines))
	for index := 0; index < prefix; index++ {
		operations = append(operations, Line{Kind: "ctx", Old: index + 1, New: index + 1, Text: oldLines[index]})
	}
	operations = append(operations, diffMiddle(
		oldLines[prefix:len(oldLines)-suffix],
		newLines[prefix:len(newLines)-suffix],
		prefix, prefix,
	)...)
	for index := 0; index < suffix; index++ {
		oldIndex := len(oldLines) - suffix + index
		newIndex := len(newLines) - suffix + index
		operations = append(operations, Line{Kind: "ctx", Old: oldIndex + 1, New: newIndex + 1, Text: oldLines[oldIndex]})
	}

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

func diffMiddle(oldLines, newLines []string, oldOffset, newOffset int) []Line {
	n := len(oldLines)
	m := len(newLines)
	if n == 0 && m == 0 {
		return nil
	}
	if n == 0 {
		result := make([]Line, 0, m)
		for index, text := range newLines {
			result = append(result, Line{Kind: "add", New: newOffset + index + 1, Text: text})
		}
		return result
	}
	if m == 0 {
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

	table := make([][]int, n+1)
	for index := range table {
		table[index] = make([]int, m+1)
	}
	for i := n - 1; i >= 0; i-- {
		for j := m - 1; j >= 0; j-- {
			if oldLines[i] == newLines[j] {
				table[i][j] = table[i+1][j+1] + 1
			} else if table[i+1][j] >= table[i][j+1] {
				table[i][j] = table[i+1][j]
			} else {
				table[i][j] = table[i][j+1]
			}
		}
	}

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
	for ; i < n; i++ {
		result = append(result, Line{Kind: "del", Old: oldOffset + i + 1, Text: oldLines[i]})
	}
	for ; j < m; j++ {
		result = append(result, Line{Kind: "add", New: newOffset + j + 1, Text: newLines[j]})
	}
	return result
}

func buildHunks(operations []Line) []Hunk {
	var changed []int
	for index, operation := range operations {
		if operation.Kind != "ctx" {
			changed = append(changed, index)
		}
	}
	if len(changed) == 0 {
		return nil
	}

	type window struct{ start, end int }
	windows := make([]window, 0, len(changed))
	for _, index := range changed {
		start := index - contextLines
		if start < 0 {
			start = 0
		}
		end := index + contextLines
		if end >= len(operations) {
			end = len(operations) - 1
		}
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

func splitLines(text string) []string {
	text = strings.ReplaceAll(text, "\r\n", "\n")
	text = strings.TrimSuffix(text, "\n")
	if text == "" {
		return nil
	}
	return strings.Split(text, "\n")
}
