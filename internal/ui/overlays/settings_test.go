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
		OpenFile:   func(string, string) {},
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
