// 设置中心 / 数据面板 / 命令面板 / 创建项目的 ui.Tester 走查。

package overlays

import (
	"testing"

	mygo "github.com/egoist/mygo/ui"

	cwstore "github.com/yangshifu1024/craftwork/internal/store"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// newOverlayTester 起一个只画浮层的测试 view（浮层状态是包级单例，
// 用例开始前重置）。
func newOverlayTester(t *testing.T) *mygo.Tester {
	t.Helper()
	store = &Store{}
	return mygo.NewTester(func(c *mygo.Context) { View(c) }, 1280, 800)
}

// ---------------------------------------------------------------------------
// 设置中心
// ---------------------------------------------------------------------------

// TestSettingsFullscreenLayout 核对全屏覆盖层形态（对齐 React e10fc65）：
// 设置铺满窗口——标题栏贴窗口顶、关闭按钮贴窗口右缘、导航贴窗口左缘，
// 而不是居中 920×660 模态。
func TestSettingsFullscreenLayout(t *testing.T) {
	tt := newOverlayTester(t)
	OpenSettings("")
	tt.Frame()
	title, ok := tt.Find("设置")
	if !ok {
		t.Fatalf("设置标题未出现: %v", tt.Texts())
	}
	// 标题在窗口顶部 48px 头部行内居中，而非垂直居中模态的 y≈94。
	if cy := title.Y + title.H/2; cy >= 40 {
		t.Errorf("设置标题应贴窗口顶（全屏头部行内），标题中心 y=%.1f", cy)
	}
	closeBtn, ok := tt.Find("关闭设置")
	if !ok {
		t.Fatalf("找不到关闭按钮: %v", tt.Texts())
	}
	// 关闭按钮贴窗口右缘（头部行右内边距 14）；模态形态下它在 ≈1086。
	if right := closeBtn.X + closeBtn.W; right < 1240 {
		t.Errorf("关闭按钮应贴窗口右缘（全屏覆盖层），右缘 x=%.1f", right)
	}
	nav, ok := tt.Find("高级")
	if !ok {
		t.Fatalf("导航缺高级分组: %v", tt.Texts())
	}
	// 导航列贴窗口左缘；「高级」只在导航出现（当前分组是通用）。
	// 模态形态下它在 x≈282（面板居中偏移），全屏后回到 ≈102。
	if nav.X >= 150 {
		t.Errorf("导航应贴窗口左缘，「高级」x=%.1f", nav.X)
	}
}

// TestSettingsContentCentered 核对内容列居中限宽（React e10fc65 的
// .settings-content-inner：max-width 880px 居中）：1280 宽下导航 240 +
// 内容区内边距 24，未限宽时行文本起点在 x≈264；限宽后内容列 880 居中，
// 文本起点右移到 x≈320。
func TestSettingsContentCentered(t *testing.T) {
	tt := newOverlayTester(t)
	OpenSettings(TabGeneral)
	tt.Frame()
	label, ok := tt.Find("界面语言")
	if !ok {
		t.Fatalf("通用分组未出现: %v", tt.Texts())
	}
	if label.X < 300 {
		t.Errorf("内容列应居中限宽 880，「界面语言」x=%.1f（期望 ≥300）", label.X)
	}
	// 窄窗口下内容区不足 880：限宽不生效，文本贴近导航列右缘。
	tt.SetSize(900, 700)
	tt.Frame()
	label, ok = tt.Find("界面语言")
	if !ok {
		t.Fatalf("窄窗下通用分组未出现: %v", tt.Texts())
	}
	if label.X >= 300 {
		t.Errorf("窄窗下不应限宽，「界面语言」x=%.1f（期望 <300）", label.X)
	}
}

func TestSettingsNavSwitchAndSearch(t *testing.T) {
	tt := newOverlayTester(t)
	OpenSettings("")
	tt.Frame()
	if !tt.HasText("模型与提供商") {
		t.Fatalf("设置中心未出现: %v", tt.Texts())
	}
	// 数据入口默认在导航里。
	if !tt.HasText("用量统计") {
		t.Fatalf("导航缺数据入口: %v", tt.Texts())
	}
	// 分组搜索只过滤导航分组；输入「外观」后其余分组与数据入口消失。
	if err := tt.Click("设置搜索"); err != nil {
		t.Fatalf("点击搜索框失败: %v", err)
	}
	tt.Type("外观")
	tt.Frame()
	// 分组按关键字过滤（数据入口保留，对齐 React 版）：当前分组是通用，
	// 过滤后「MCP 服务器」等未匹配分组应从导航消失。
	if tt.HasText("MCP 服务器") {
		t.Errorf("未匹配分组应从导航消失: %v", tt.Texts())
	}
	if !tt.HasText("外观") {
		t.Errorf("匹配的分组应保留")
	}
	// 清空关键字恢复。
	if err := tt.Click("设置搜索"); err != nil {
		t.Fatalf("点击搜索框失败: %v", err)
	}
	for range 2 {
		tt.Key(0, mygo.KeyBackspace)
	}
	tt.Frame()
	if !tt.HasText("用量统计") {
		t.Errorf("清空搜索后数据入口应恢复")
	}
}

// TestSettingsSearchNoMatch 空态落在内容区（对齐 React panel-empty）：
// 搜索无匹配分组时导航变空，内容区给图标 + 主句 + 关键词提示。
func TestSettingsSearchNoMatch(t *testing.T) {
	tt := newOverlayTester(t)
	OpenSettings("")
	tt.Frame()
	if err := tt.Click("设置搜索"); err != nil {
		t.Fatalf("点击搜索框失败: %v", err)
	}
	tt.Type("不存在的分组")
	tt.Frame()
	if !tt.HasText("没有匹配的设置分组") {
		t.Fatalf("搜索无匹配时应显示空态: %v", tt.Texts())
	}
	if !tt.HasText("试试「模型」「权限」「外观」等关键词。") {
		t.Errorf("空态缺关键词提示: %v", tt.Texts())
	}
}

func TestSettingsImmediateAppearance(t *testing.T) {
	tt := newOverlayTester(t)
	OpenSettings(TabAppearance)
	tt.Frame()
	// 服务未注入时切换主题仍立即生效（保存失败只保留本地观感）。
	if err := tt.Click("浅色"); err != nil {
		t.Fatalf("点击浅色主题卡失败: %v", err)
	}
	if theme.IsDark() {
		t.Errorf("切换浅色后当前主题仍为深色")
	}
	if err := tt.Click("深色"); err != nil {
		t.Fatalf("点击深色主题卡失败: %v", err)
	}
	if !theme.IsDark() {
		t.Errorf("切换深色后当前主题应为深色")
	}
}

func TestSettingsFreeLevelGateFromToolsTab(t *testing.T) {
	tt := newOverlayTester(t)
	OpenSettings(TabTools)
	tt.Frame()
	// 点「自由」档先弹风险确认门；勾选并确认后门关闭。
	if err := tt.Click("自由"); err != nil {
		t.Fatalf("点击自由档失败: %v", err)
	}
	if !tt.HasText("要启用完全访问吗？") {
		t.Fatalf("自由档确认门未弹出: %v", tt.Texts())
	}
	if err := tt.Click("我已了解风险，并对自己的数据安全负责。"); err != nil {
		t.Fatalf("点击勾选门失败: %v", err)
	}
	if err := tt.Click("确认启用"); err != nil {
		t.Fatalf("点击确认启用失败: %v", err)
	}
	if tt.HasText("要启用完全访问吗？") {
		t.Errorf("确认后勾选门应关闭")
	}
}

// TestSettingsDataPanelLayersAbove 核对浮层层级与 Esc 链的位置（全屏
// 覆盖层仍走 DialogBase，压在浮层体系最底层）：设置之上可再开数据面板；
// Esc 先关上层（数据面板）再关设置——窗口退出链只在无模态时参与。
func TestSettingsDataPanelLayersAbove(t *testing.T) {
	tt := newOverlayTester(t)
	OpenSettings("")
	tt.Frame()
	if err := tt.Click("用量统计"); err != nil {
		t.Fatalf("点击数据入口失败: %v", err)
	}
	tt.Frame()
	if !tt.HasText("还没有用量数据") {
		t.Fatalf("数据面板未打开在设置之上: %v", tt.Texts())
	}
	if !tt.HasText("模型与提供商") {
		t.Errorf("数据面板打开时设置应保持打开")
	}
	// 第一次 Esc 关上层（数据面板），设置保留。
	tt.Key(0, mygo.KeyEscape)
	tt.Frame()
	if tt.HasText("还没有用量数据") {
		t.Errorf("第一次 Esc 应关闭数据面板")
	}
	if !tt.HasText("模型与提供商") {
		t.Fatalf("数据面板关闭后设置应仍在")
	}
	// 第二次 Esc 才轮到设置本体。
	tt.Key(0, mygo.KeyEscape)
	tt.Frame()
	if tt.HasText("模型与提供商") {
		t.Errorf("第二次 Esc 应关闭设置")
	}
}

// TestSettingsCloseReturnsToUnderlyingView 核对「关闭返回原位」：设置打开
// 时盖住原视图（按压落在覆盖层上，背景控件不触发），Esc 关闭后原视图
// 恢复可交互；再次打开回到上次停留的分组。
func TestSettingsCloseReturnsToUnderlyingView(t *testing.T) {
	store = &Store{}
	var clicks int
	tt := mygo.NewTester(func(c *mygo.Context) {
		if mygo.Button(c, "背景按钮").Clicked() {
			clicks++
		}
		View(c)
	}, 1280, 800)
	OpenSettings(TabAppearance)
	tt.Frame()
	if !tt.HasText("界面字号") {
		t.Fatalf("设置（外观分组）未打开: %v", tt.Texts())
	}
	// 覆盖层吃掉按压：背景按钮不触发，设置也不被误关。
	if err := tt.Click("背景按钮"); err != nil {
		t.Fatalf("点击背景按钮失败: %v", err)
	}
	tt.Frame()
	if clicks != 0 {
		t.Fatalf("覆盖层打开时背景按钮不应触发，实得 %d 次点击", clicks)
	}
	if !tt.HasText("界面字号") {
		t.Fatalf("覆盖层不应被背景点击误关")
	}
	// Esc 关闭后原视图恢复可交互。
	tt.Key(0, mygo.KeyEscape)
	tt.Frame()
	if tt.HasText("界面字号") {
		t.Fatalf("Esc 应关闭设置")
	}
	if err := tt.Click("背景按钮"); err != nil {
		t.Fatalf("关闭后点击背景按钮失败: %v", err)
	}
	tt.Frame()
	if clicks != 1 {
		t.Fatalf("关闭后背景按钮应恢复可交互，实得 %d 次点击", clicks)
	}
	// 再次打开（不传分组）：回到上次停留的外观分组。
	OpenSettings("")
	tt.Frame()
	if !tt.HasText("界面字号") {
		t.Errorf("重开应回到上次停留的分组: %v", tt.Texts())
	}
}

// ---------------------------------------------------------------------------
// 数据面板
// ---------------------------------------------------------------------------

func TestDataPanelTabsAndFilter(t *testing.T) {
	tt := newOverlayTester(t)
	store.data.audit = []cwstore.AuditEntry{
		{ID: 1, TS: 1, Tool: "fs_read", Risk: "R0", Decision: "auto", Detail: "读取"},
		{ID: 2, TS: 2, Tool: "shell_exec", Risk: "R2", Decision: "allow", Detail: "用户允许"},
		{ID: 3, TS: 3, Tool: "fs_write", Risk: "R1", Decision: "deny", Detail: "用户拒绝"},
	}
	OpenData("")
	tt.Frame()
	if !tt.HasText("审计记录") {
		t.Fatalf("数据面板未出现: %v", tt.Texts())
	}
	if !tt.HasText("共 3 条记录") {
		t.Fatalf("审计计数错误: %v", tt.Texts())
	}
	// 过滤「允许」只剩 1 条（首个「允许」是过滤按钮，绘制顺序在表格前）。
	if err := tt.Click("允许"); err != nil {
		t.Fatalf("点击允许过滤失败: %v", err)
	}
	if !tt.HasText("共 1 条记录") {
		t.Fatalf("过滤后计数错误: %v", tt.Texts())
	}
	// 切到用量：无数据显示空态。
	if err := tt.Click("用量"); err != nil {
		t.Fatalf("切换用量分区失败: %v", err)
	}
	if !tt.HasText("还没有用量数据") {
		t.Errorf("用量空态缺失: %v", tt.Texts())
	}
	// 切到备份：无备份显示空态。
	if err := tt.Click("备份"); err != nil {
		t.Fatalf("切换备份分区失败: %v", err)
	}
	if !tt.HasText("还没有备份") {
		t.Errorf("备份空态缺失: %v", tt.Texts())
	}
}

// ---------------------------------------------------------------------------
// 命令面板
// ---------------------------------------------------------------------------

func TestPaletteCommandsAndFilter(t *testing.T) {
	tt := newOverlayTester(t)
	BindHooks(Hooks{
		NewThread:  func() {},
		OpenThread: func(string) {},
		OpenFile:   func(string, int) {},
	})
	TogglePalette()
	tt.Frame()
	if !tt.HasText("打开设置") {
		t.Fatalf("命令面板未出现: %v", tt.Texts())
	}
	if !tt.HasText("打开设置") || !tt.HasText("切换到 Work 模式") {
		t.Fatalf("命令缺失: %v", tt.Texts())
	}
	// 未接线的命令带「待接线」标注；已接线的不带。
	if !tt.HasText("⌘N") {
		t.Errorf("已接线命令应展示快捷键")
	}
	// 过滤：「用量」只剩用量统计。
	if err := tt.Click("面板搜索"); err != nil {
		t.Fatalf("点击面板输入框失败: %v", err)
	}
	tt.Type("用量")
	tt.Frame()
	if tt.HasText("打开设置") {
		t.Errorf("过滤后不应残留无关命令")
	}
	if !tt.HasText("打开用量统计") {
		t.Errorf("过滤后应保留用量统计")
	}
	// 回车执行：面板关闭，数据面板打开在用量分区。
	tt.Key(0, mygo.KeyEnter)
	tt.Frame()
	if tt.HasText("打开设置") {
		t.Errorf("执行后面板应关闭")
	}
	if !tt.HasText("用量统计") {
		t.Errorf("执行后应打开用量分区: %v", tt.Texts())
	}
}

func TestPaletteEnterExecutesHook(t *testing.T) {
	tt := newOverlayTester(t)
	var ran int
	BindHooks(Hooks{NewThread: func() { ran++ }})
	TogglePalette()
	tt.Frame()
	if err := tt.Click("面板搜索"); err != nil {
		t.Fatalf("点击面板输入框失败: %v", err)
	}
	tt.Type("新建会话")
	tt.Frame()
	tt.Key(0, mygo.KeyEnter)
	tt.Frame()
	if ran != 1 {
		t.Fatalf("回车应执行选中的新建会话，实得 %d", ran)
	}
}

// ---------------------------------------------------------------------------
// 创建项目
// ---------------------------------------------------------------------------

func TestCreateProjectTwoViews(t *testing.T) {
	tt := newOverlayTester(t)
	OpenCreateProject()
	tt.Frame()
	if !tt.HasText("创建项目") || !tt.HasText("源文件夹") {
		t.Fatalf("创建项目对话框未出现: %v", tt.Texts())
	}
	// 无源文件夹时「创建项目」禁用：点击不进入信任视图。
	// 按钮文案与标题同名，用「取消」按钮定位主按钮位置（actionsRow 右对齐，
	// 创建按钮在取消按钮右侧）。
	createAt := func(t *testing.T) {
		t.Helper()
		r, ok := tt.Find("取消")
		if !ok {
			t.Fatalf("找不到取消按钮: %v", tt.Texts())
		}
		tt.ClickAt(r.X+r.W+44, r.Y+r.H/2)
	}
	createAt(t)
	if tt.HasText("信任此文件夹吗？") {
		t.Fatalf("无文件夹时不应进入信任视图")
	}
	// 直接注入文件夹后重画（文件夹选择器是原生对话框，测试跳过）。
	store.project.folders = []string{"/tmp/demo-project", "/tmp/second"}
	tt.Frame()
	createAt(t)
	if !tt.HasText("信任这些文件夹吗？") {
		t.Fatalf("应进入信任确认视图: %v", tt.Texts())
	}
	if !tt.HasText("/tmp/demo-project") {
		t.Fatalf("信任视图应列出完整路径")
	}
	if tt.HasText("主要") {
		t.Errorf("信任视图不应显示主要徽标")
	}
	// 焦点默认在「取消」；Esc 回到表单（不直接关闭）。
	if !tt.Focused("取消") {
		t.Errorf("信任视图默认焦点应在取消")
	}
	tt.Key(0, mygo.KeyEscape)
	tt.Frame()
	if !tt.HasText("源文件夹") {
		t.Fatalf("Esc 应回到表单视图")
	}
	// 再进信任视图并直接确认：服务未注入时安全收尾（不崩溃、不出对话框外溢）。
	createAt(t)
	if err := tt.Click("信任文件夹并创建"); err != nil {
		t.Fatalf("点击信任按钮失败: %v", err)
	}
}
