// 本文件是原生 UI 的图标登记表与统一图标组件。
//
// 登记表逐项对齐 frontend/src/components/Icon.tsx（双轨期的清单来源）：键是
// 业务代码书写的图标名（含别名），值是 internal/ui/icons/svg/ 下的 lucide SVG
// 文件（lucide-static@1.66.0，与 lucide-react 1.66.0 同版本）。SVG 在包初始化
// 时解析一次并缓存，view 每帧只查表，不做任何解析。
//
// lucide 图标以 stroke-width 2 发布，前端 Icon 组件默认 1.75；解析时把根元素
// 的 stroke-width 改写为 1.75，保持与双轨期一致的观感。
//
// 新增图标：把 lucide-static 同名 SVG 放进 icons/svg/ 并在此登记，不要在
// 业务 view 里手写 SVG 路径。
package icons

import (
	"bytes"
	"embed"

	mygo "github.com/egoist/mygo/ui"
)

//go:embed svg/*.svg
var iconFS embed.FS

// iconFiles 是登记名到 SVG 文件名的映射（不带 .svg 后缀）。
// 别名（branch、list-filter 等）与正名指向同一文件。
var iconFiles = map[string]string{
	"code":             "code",
	"briefcase":        "briefcase",
	"chevron-down":     "chevron-down",
	"chevron-right":    "chevron-right",
	"chevron-left":     "chevron-left",
	"chevron-up":       "chevron-up",
	"minus":            "minus",
	"type":             "type",
	"file-plus":        "file-plus",
	"plus":             "plus",
	"search":           "search",
	"settings":         "settings",
	"command":          "command",
	"folder":           "folder",
	"folder-open":      "folder-open",
	"folder-plus":      "folder-plus",
	"file":             "file",
	"file-code":        "file-code",
	"file-text":        "file-text",
	"git-branch":       "git-branch",
	"branch":           "git-branch", // 别名
	"diff":             "git-compare",
	"terminal":         "terminal",
	"check":            "check",
	"check-circle":     "circle-check",
	"x":                "x",
	"x-circle":         "circle-x",
	"square-minus":     "square-minus",
	"alert":            "triangle-alert",
	"info":             "info",
	"stop":             "square",
	"arrow-up":         "arrow-up",
	"arrow-down":       "arrow-down",
	"arrow-right":      "arrow-right",
	"corner-down-left": "corner-down-left",
	"paperclip":        "paperclip",
	"image":            "image",
	"copy":             "copy",
	"refresh":          "refresh-cw",
	"more":             "ellipsis",
	"shield":           "shield",
	"lock":             "lock",
	"key":              "key-round",
	"eye":              "eye",
	"eye-off":          "eye-off",
	"edit":             "square-pen",
	"save":             "save",
	"trash":            "trash-2",
	"archive":          "archive",
	"download":         "download",
	"external-link":    "external-link",
	"play":             "play",
	"clock":            "clock",
	"sparkles":         "sparkles",
	"list-todo":        "list-todo",
	"user":             "user",
	"layers":           "layers",
	"sun":              "sun",
	"moon":             "moon",
	"monitor":          "monitor",
	"sidebar":          "panel-left",
	"panel":            "panel-right",
	"panel-close":      "panel-right-close",
	"panel-open":       "panel-right-open",
	"maximize":         "expand",
	"minimize":         "shrink",
	"hash":             "hash",
	"quote":            "quote",
	"shield-check":     "shield-check",
	"check-square":     "square-check-big",
	"list-filter":      "filter", // 别名：与 filter 同指 Filter 图标（对齐前端）
	"table":            "table",
	"globe":            "globe",
	"bell":             "bell",
	"database":         "database",
	"cpu":              "cpu",
	"zap":              "zap",
	"sliders":          "sliders-vertical",
	"mail":             "mail",
	"calendar":         "calendar",
	"message":          "message-square",
	"filter":           "filter",
	"rotate-ccw":       "rotate-ccw",
	"loader":           "loader-circle",
	"hard-drive":       "hard-drive",
	"undo":             "undo-2",
}

// fallbackIcon 是登记名未知时画出的回退图标（circle.svg，不在登记表内）。
const fallbackIcon = "circle"

// iconStrokeWidth 保留前端 Icon 组件的默认描边宽度。
const iconStrokeWidth = "1.75"

var (
	icons       = buildIcons()
	fallbackSVG = mustIconSVG(fallbackIcon)
)

// mustIconSVG 读取并解析一个登记 SVG；缺文件是编程错误，直接 panic 暴露。
func mustIconSVG(file string) *mygo.SVG {
	data, err := iconFS.ReadFile("svg/" + file + ".svg")
	if err != nil {
		panic("ui: 图标资产缺失 " + file + ".svg: " + err.Error())
	}
	return mygo.MustParseSVG(bytes.ReplaceAll(data,
		[]byte(`stroke-width="2"`), []byte(`stroke-width="`+iconStrokeWidth+`"`)))
}

// buildIcons 在包初始化时解析全部登记 SVG；同一文件的别名共享解析结果。
func buildIcons() map[string]*mygo.SVG {
	parsed := make(map[string]*mygo.SVG, len(iconFiles))
	m := make(map[string]*mygo.SVG, len(iconFiles))
	for name, file := range iconFiles {
		if _, dup := m[name]; dup {
			panic("ui: 图标登记名重复: " + name)
		}
		svg, ok := parsed[file]
		if !ok {
			svg = mustIconSVG(file)
			parsed[file] = svg
		}
		m[name] = svg
	}
	return m
}

// Icon 按登记名在 c 上画一个 lucide 图标：颜色随文字色，尺寸随字号
// （FontSize/.Size 可调，与 mygo ui.Icon 一致）。未知登记名回退为圆圈，
// 不让 view 因笔误崩溃。
func Icon(c *mygo.Context, name string) mygo.Element {
	svg, ok := icons[name]
	if !ok {
		svg = fallbackSVG
	}
	return mygo.Icon(c, svg)
}
