// Package title 负责线程标题的本地清洗、长度限制与降级提取。
//
// 模型生成的标题与首轮消息降级标题都经过同一套清洗规则，保证界面上的
// 标题长度一致、没有 Markdown 标记与多余标点。
package title

import (
	"path/filepath"
	"strings"
	"unicode"
)

// Default 是新建线程的初始标题，也是允许自动命名的判断条件。
const Default = "新对话"

// MaxRunes 是自动标题的最大字符数。模型指令与本地截断都使用这个上限。
const MaxRunes = 20

// Derive 从首条消息提取降级标题。
//
// 优先使用第一行有效文本；纯附件消息使用第一个附件的文件名。返回值可能
// 为空，调用方应保持原标题不变。
func Derive(text string, attachments []string) string {
	candidate := firstMeaningfulLine(text)
	if candidate == "" {
		for _, path := range attachments {
			name := strings.TrimSpace(filepath.Base(path))
			if name != "" && name != "." && name != string(filepath.Separator) {
				candidate = name
				break
			}
		}
	}
	candidate = Normalize(candidate)
	if candidate == "" || !HasContent(candidate) {
		return ""
	}
	return Limit(candidate)
}

// Normalize 清理 Markdown 前缀、多余空白、引号与首尾标点。
func Normalize(value string) string {
	value = strings.TrimSpace(value)
	if index := strings.IndexAny(value, "\r\n"); index >= 0 {
		value = value[:index]
	}
	value = strings.Join(strings.Fields(value), " ")
	value = strings.TrimLeft(value, "#>-*_` ")
	value = strings.ReplaceAll(value, "`", "")
	value = strings.Trim(value, " \t\r\n，。！？；：、,.!?;:*_\"'“”‘’《》 ")
	return value
}

// Limit 把标题限制在 MaxRunes 个字符内；需要截断时保留省略号。
func Limit(value string) string {
	runes := []rune(value)
	if len(runes) <= MaxRunes {
		return value
	}
	trimmed := strings.TrimRight(string(runes[:MaxRunes-1]), " ，。！？；：、,.!?;:")
	return trimmed + "…"
}

// HasContent 判断标题是否至少包含一个文字或数字，避免只留下标点。
func HasContent(value string) bool {
	for _, char := range value {
		if unicode.IsLetter(char) || unicode.IsDigit(char) {
			return true
		}
	}
	return false
}

// firstMeaningfulLine 返回第一行非空、且不是代码围栏标记的文本。
func firstMeaningfulLine(text string) string {
	for _, line := range strings.Split(text, "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "```") {
			continue
		}
		return line
	}
	return ""
}
