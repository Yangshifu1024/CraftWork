package theme

// 主题派生函数的单测：字号档、色板 → ui.Theme 的语义映射、R0–R4 与模式
// 强调色映射、窗口底色。映射走 ui.Tester 真实 view 帧验证（含深浅实时切换）。

import (
	"testing"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/policy"
)

func TestFontLadderDerivation(t *testing.T) {
	// base.css 默认基准 15px：25/19/17/15/14/13/11。
	l := FontLadder{Size: 15}
	for name, got := range map[string]struct {
		want float32
		have float32
	}{
		"Title":   {25, l.Title()},
		"XL":      {19, l.XL()},
		"LG":      {17, l.LG()},
		"Base":    {15, l.Base()},
		"Caption": {14, l.Caption()},
		"SM":      {13, l.SM()},
		"XS":      {11, l.XS()},
	} {
		if got.have != got.want {
			t.Errorf("%s = %v, want %v", name, got.have, got.want)
		}
	}
	// 派生公式语义：基准变了全档跟随（设置改界面字号后整站缩放）。
	big := FontLadder{Size: 16}
	if big.SM() != 14 || big.Title() != 26 {
		t.Errorf("基准 16 时派生错误: SM=%v Title=%v", big.SM(), big.Title())
	}
	if Fonts.Size != BaseFontSize {
		t.Errorf("全局字号档基准 = %v, want %v", Fonts.Size, BaseFontSize)
	}
}

// snapTheme 用 Tester 跑一帧外壳 view，截获 Apply 之后的窗口主题与色板。
func snapTheme(t *testing.T, dark bool) (mygo.Theme, *Palette) {
	t.Helper()
	var snap mygo.Theme
	var pal *Palette
	view := func(c *mygo.Context) {
		pal = Apply(c)
		snap = *c.Theme()
	}
	tt := mygo.NewTester(view, 800, 600)
	tt.SetDark(dark)
	return snap, pal
}

func TestApplyMapsPaletteToUITheme(t *testing.T) {
	for _, dark := range []bool{true, false} {
		snap, pal := snapTheme(t, dark)
		p := darkPalette()
		if !dark {
			p = lightPalette()
		}
		// 语义映射的完整契约：与独立重建的派生主题逐字段一致。
		want := uiTheme(p, dark)
		if snap != *want {
			t.Fatalf("dark=%v 主题映射与派生结果不一致", dark)
		}
		// 关键字段的字面锚点（改动 base.css 时这里会红，属预期，改断言）。
		if pal != &darkPal && pal != &lightPal {
			t.Fatalf("Apply 返回了未知色板")
		}
		if dark && pal != &darkPal {
			t.Fatalf("深色系统未返回深色色板")
		}
		if !dark && pal != &lightPal {
			t.Fatalf("浅色系统未返回浅色色板")
		}
		if snap.FontSize != BaseFontSize || snap.Radius != RadiusBtn || snap.ScrollbarWidth != 6 {
			t.Errorf("dark=%v 字号/圆角/滚动条宽度映射错误: %v/%v/%v",
				dark, snap.FontSize, snap.Radius, snap.ScrollbarWidth)
		}
	}
	if darkPal.Accent != mygo.Hex("#5b9dff") {
		t.Errorf("深色 accent = %v, want #5b9dff", darkPal.Accent)
	}
	if lightPal.Accent != mygo.Hex("#2563eb") {
		t.Errorf("浅色 accent = %v, want #2563eb", lightPal.Accent)
	}
	if darkPaletteUI.Dark != true || lightPaletteUI.Dark != false {
		t.Errorf("主题单例的 Dark 标志错误")
	}
}

func TestApplySwitchesWithSystemAppearance(t *testing.T) {
	// 跟随系统的实时性：同一帧循环里切换系统外观，下一帧主题即换。
	var snap mygo.Theme
	view := func(c *mygo.Context) {
		Apply(c)
		snap = *c.Theme()
	}
	tt := mygo.NewTester(view, 800, 600)
	tt.SetDark(true)
	if !snap.Dark || snap.Accent != mygo.Hex("#5b9dff") {
		t.Fatalf("切到深色后 accent = %v", snap.Accent)
	}
	tt.SetDark(false)
	if snap.Dark || snap.Accent != mygo.Hex("#2563eb") {
		t.Fatalf("切到浅色后 accent = %v", snap.Accent)
	}
	tt.SetDark(true)
	if snap.Accent != mygo.Hex("#5b9dff") {
		t.Fatalf("切回深色后 accent = %v", snap.Accent)
	}
}

func TestRiskColors(t *testing.T) {
	p := darkPalette()
	want := []mygo.Color{
		mygo.Hex("#7c8798"), // R0 只读
		mygo.Hex("#e8b04a"), // R1 写工作区
		mygo.Hex("#ec9350"), // R2 执行
		mygo.Hex("#ee7070"), // R3 网络或越界
		mygo.Hex("#e0475f"), // R4 破坏性
	}
	for level, color := range want {
		if got := p.Risk(policy.Risk(level)); got != color {
			t.Errorf("R%d = %v, want %v", level, got, color)
		}
	}
	// 越界回退 R0 中性色，不让 view 因未知等级画出黑块。
	if got := p.Risk(policy.Risk(9)); got != want[0] {
		t.Errorf("越界风险级 = %v, want R0 %v", got, want[0])
	}
	if got := p.Risk(policy.Risk(-1)); got != want[0] {
		t.Errorf("负风险级 = %v, want R0 %v", got, want[0])
	}
	// 浅色体系独立成套。
	lp := lightPalette()
	if lp.Risk(policy.Risk(4)) != mygo.Hex("#b3122f") {
		t.Errorf("浅色 R4 = %v, want #b3122f", lp.Risk(policy.Risk(4)))
	}
}

func TestModeAccent(t *testing.T) {
	p := darkPalette()
	work := p.ModeAccent(ModeWork)
	if work.Accent != mygo.Hex("#f0b354") || work.On != mygo.Hex("#1d1405") {
		t.Errorf("深色 Work 强调色错误: %v / %v", work.Accent, work.On)
	}
	code := p.ModeAccent(ModeCode)
	if code.Accent != p.Accent || code.Soft != p.AccentSoft || code.On != p.OnAccent {
		t.Errorf("Code 模式未随主题 accent: %+v", code)
	}
	lp := lightPalette()
	if lp.ModeAccent(ModeWork).Accent != mygo.Hex("#b06f0c") {
		t.Errorf("浅色 Work 强调色 = %v, want #b06f0c", lp.ModeAccent(ModeWork).Accent)
	}
	// Code 是默认档：未知模式回退到 accent 而不是 Work 琥珀。
	var unknown Mode = 42
	if p.ModeAccent(unknown) != code {
		t.Errorf("未知模式未回退 Code 档")
	}
}

func TestWindowBackgroundHex(t *testing.T) {
	cases := map[string]string{
		"light":  "#eef0f4",
		"dark":   "#0b0d11",
		"system": "#0b0d11",
		"":       "#0b0d11",
	}
	for setting, want := range cases {
		if got := WindowBackgroundHex(setting); got != want {
			t.Errorf("WindowBackgroundHex(%q) = %q, want %q", setting, got, want)
		}
	}
}
