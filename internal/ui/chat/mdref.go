// mdref.go 移植 frontend/src/features/chat/markdownRef.ts 与 Markdown.tsx 的
// preprocess：「路径:行号」文件引用统一识别为内部链接。
//
// 模型在消息里引用源码位置有三种写法：裸文本（foo.go:12）、空目标链接
// （[foo.go:12]()）和反引号行内代码（`foo.go:12`）。围栏代码块内的内容
// 不做转换。三种写法统一替换成 [path:line](craftwork://path:line)，点击
// 经 OnLink（markdown.go）换算回路径与行号交给 OnOpenFile 打开。
//
// 与 React 版的差异：React 在渲染层分别处理三种写法（代码样式保留、只加
// 点击行为）；原生侧统一替换为链接，功能一致（点击定位），视觉统一为
// 链接色。判定规则与 markdownRef.ts 同源。

package chat

import (
	"regexp"
	"strconv"
	"strings"
)

// pathLinePattern 是路径与行号的核心匹配片段（markdownRef.ts 同源）。
//
// 路径允许多层目录、目录与文件名中的点、以及 1-8 位扩展名；行号为 1-5 位
// 数字。刻意不匹配：无扩展名文件（Makefile:3）、带列号（a.go:12:34）、
// 行号超过 5 位——宁可不转也不要误伤。
const pathLinePattern = `((?:[\w.-]+/)*[\w.-]+\.[A-Za-z0-9]{1,8}):(\d{1,5})`

// exactRefPattern 整体匹配：文本必须恰好是一个引用。
var exactRefPattern = regexp.MustCompile(`^` + pathLinePattern + `$`)

// PathLineRef 是识别结果。行号为 0 时调用方按「不定位到行」处理。
type PathLineRef struct {
	Path string
	Line int
}

// MatchPathLineRef 判断一段文本是否整体恰好是一个「路径:行号」引用。
// 混合内容返回 nil，避免把普通代码片段误判成引用。
func MatchPathLineRef(text string) (PathLineRef, bool) {
	m := exactRefPattern.FindStringSubmatch(text)
	if m == nil {
		return PathLineRef{}, false
	}
	line, _ := strconv.Atoi(m[2])
	return PathLineRef{Path: m[1], Line: line}, true
}

// fenceLine 报告一行是否是代码围栏的边界（``` 或缩进的 ```）。
func fenceLine(line string) bool {
	return strings.HasPrefix(strings.TrimLeft(line, " \t"), "```")
}

// 内部链接形式：craftwork://path:line。
func internalLink(path string, line int) string {
	return "[" + path + ":" + strconv.Itoa(line) + "](craftwork://" + path + ":" + strconv.Itoa(line) + ")"
}

// 预处理的三条替换（围栏外的行依次执行）。
var (
	// 空目标链接 [path:line]() → 内部链接（模型常把引用输出成空目标）。
	emptyLinkPattern = regexp.MustCompile(`\[` + pathLinePattern + `\]\(\)`)
	// 行内代码引用 `path:line` → 内部链接（React 版按渲染后的 code span
	// 整体判定；这里按整段反引号近似，嵌套反引号属罕见写法不做支持）。
	inlineCodePattern = regexp.MustCompile("`" + pathLinePattern + "`")
	// 裸文本引用：行首或空白 / 括号 / 中文引号之后（React 版 bareRefPattern
	// 同源，先行断言以词边界近似）。
	bareRefPattern = regexp.MustCompile(`(^|[\s(（「“])` + pathLinePattern)
)

// PreprocessRefs 把正文里的 path:line 引用替换为内部链接。
// 代码围栏内的内容不做转换。已转换的链接与 URL 不会再被后两条规则命中
// （方括号与斜杠都不在裸文本规则的起始字符集里）。
func PreprocessRefs(text string) string {
	lines := strings.Split(text, "\n")
	inFence := false
	for i, line := range lines {
		if fenceLine(line) {
			inFence = !inFence
			continue
		}
		if inFence {
			continue
		}
		line = emptyLinkPattern.ReplaceAllStringFunc(line, func(s string) string {
			m := emptyLinkPattern.FindStringSubmatch(s)
			return internalLink(m[1], mustAtoi(m[2]))
		})
		line = inlineCodePattern.ReplaceAllStringFunc(line, func(s string) string {
			m := inlineCodePattern.FindStringSubmatch(s)
			return internalLink(m[1], mustAtoi(m[2]))
		})
		line = bareRefPattern.ReplaceAllStringFunc(line, func(s string) string {
			m := bareRefPattern.FindStringSubmatch(s)
			return m[1] + internalLink(m[2], mustAtoi(m[3]))
		})
		lines[i] = line
	}
	return strings.Join(lines, "\n")
}

// ParseRefLink 从内部链接地址（craftwork://path:line）换算回引用；
// 不是内部链接形式时返回 false。
func ParseRefLink(url string) (PathLineRef, bool) {
	const scheme = "craftwork://"
	if !strings.HasPrefix(url, scheme) {
		return PathLineRef{}, false
	}
	rest := strings.TrimPrefix(url, scheme)
	idx := strings.LastIndex(rest, ":")
	if idx < 0 {
		return PathLineRef{}, false
	}
	line, err := strconv.Atoi(rest[idx+1:])
	if err != nil {
		return PathLineRef{}, false
	}
	return PathLineRef{Path: rest[:idx], Line: line}, true
}

// mustAtoi 是预处理替换里的行号换算：正则已保证是数字，出错时按 0 处理
// （调用方对 0 按「不定位到行」处理）。
func mustAtoi(s string) int {
	n, _ := strconv.Atoi(s)
	return n
}
