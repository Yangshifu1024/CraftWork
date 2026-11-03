package icons

// 图标登记表的单测：登记名全覆盖、别名与正名同源、回退图标存在。

import (
	"testing"

	mygo "github.com/egoist/mygo/ui"
)

func TestIconsRegistryComplete(t *testing.T) {
	if len(icons) != len(iconFiles) {
		t.Fatalf("解析出的图标数 %d 与登记表 %d 不一致", len(icons), len(iconFiles))
	}
	for name := range iconFiles {
		if icons[name] == nil {
			t.Errorf("登记名 %q 未解析出 SVG", name)
		}
	}
}

func TestIconsAliasesShareSource(t *testing.T) {
	for alias, file := range map[string]string{
		"branch":      "git-branch",
		"list-filter": "filter",
	} {
		if icons[alias] != icons[file] {
			t.Errorf("别名 %q 与 %q 未指向同一 SVG", alias, file)
		}
	}
}

func TestIconsFallback(t *testing.T) {
	if fallbackSVG == nil {
		t.Fatalf("回退图标 %q 缺失", fallbackIcon)
	}
	if _, known := iconFiles["unknown-icon-name"]; known {
		t.Fatalf("未知名字不应在登记表里")
	}
	// 未知登记名在一帧 view 里画回退图标而不是 panic。
	view := func(c *mygo.Context) {
		Icon(c, "unknown-icon-name")
		Icon(c, "search")
	}
	tt := mygo.NewTester(view, 100, 100)
	_ = tt
}

func TestIconsMatchFrontendRegistry(t *testing.T) {
	// 登记表对齐 frontend/src/components/Icon.tsx：86 个登记名
	// （84 个 lucide 文件，branch 与 list-filter 是别名）。
	if len(iconFiles) != 86 {
		t.Errorf("登记名数量 = %d, want 86（对齐 Icon.tsx）", len(iconFiles))
	}
}
