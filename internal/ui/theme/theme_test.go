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

func TestDerivedVisualColors(t *testing.T) {
	// 收编自视图包就地写 hex 的派生色（差异增删文本色、文件图标着色与
	// 填充危险按钮文字色）：字面锚点与深浅差异在这里锁住。
	d := darkPalette()
	if d.DiffAddText != mygo.Hex("#a6e3c7") || d.DiffDelText != mygo.Hex("#f0b3b3") {
		t.Errorf("深色差异文本色错误: %v / %v", d.DiffAddText, d.DiffDelText)
	}
	if d.FileDirIcon != mygo.Hex("#d6a45c") {
		t.Errorf("深色目录图标色 = %v", d.FileDirIcon)
	}
	l := lightPalette()
	if l.DiffAddText != mygo.Hex("#146c43") || l.DiffDelText != mygo.Hex("#a12a2a") {
		t.Errorf("浅色差异文本色错误: %v / %v", l.DiffAddText, l.DiffDelText)
	}
	if l.FileDirIcon != mygo.Hex("#b07a24") {
		t.Errorf("浅色目录图标色 = %v", l.FileDirIcon)
	}
	// 扩展名着色不分深浅；填充危险按钮文字恒为白。
	for _, p := range []Palette{d, l} {
		if p.FileCodeIcon != mygo.Hex("#59b7d6") || p.FileDocIcon != mygo.Hex("#5b9dff") ||
			p.FileSheetIcon != mygo.Hex("#43c08d") || p.FilePdfIcon != mygo.Hex("#e07070") {
			t.Errorf("文件图标着色错误: %+v", p)
		}
		if p.OnDanger != mygo.Hex("#ffffff") {
			t.Errorf("OnDanger = %v, want #ffffff", p.OnDanger)
		}
	}
}

// TestMaterialPalette 验证窗口材质（under-window 毛玻璃，14 号票）的色板
// 派生：侧栏底与悬停行改半透明（材质从其下透出，对照 ZCode 侧栏不画
// 实底），其余取值与原色板同源不变；返回副本，不改写深浅单例。
func TestMaterialPalette(t *testing.T) {
	for _, base := range []*Palette{&darkPal, &lightPal} {
		mat := MaterialPalette(base)
		if mat == base {
			t.Fatal("MaterialPalette 应返回副本，不得改写深浅单例")
		}
		if mat.SidebarBg.A >= base.SidebarBg.A {
			t.Errorf("材质侧栏底应半透明: alpha %d >= %d", mat.SidebarBg.A, base.SidebarBg.A)
		}
		// 悬停行：zai 色板本就半透明（黑/白 5%），材质版原样保留——只要求
		// 不比基础版更实（相等即正确）。
		if mat.SidebarHover.A > base.SidebarHover.A {
			t.Errorf("材质侧栏悬停行不得比基础版更实: alpha %d > %d", mat.SidebarHover.A, base.SidebarHover.A)
		}
		if mat.ChatBg != base.ChatBg || mat.BgPanel != base.BgPanel || mat.Text != base.Text {
			t.Error("对话区、面板与文字色在材质模式下应保持原值（实底对照 ZCode workspace）")
		}
	}
	// 深浅单例不被污染。
	if darkPal.SidebarBg.A != 255 || lightPal.SidebarBg.A != 255 {
		t.Error("MaterialPalette 不得改写深浅单例的侧栏底")
	}
}
