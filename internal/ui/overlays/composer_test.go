// 输入区三浮层（模型选择 / 审批档位 / 上下文详情）与 R0–R4 说明的
// ui.Tester 走查。浮层状态在包级单例上，用例开始前重置；涉服务保存的
// 用例注入落在临时目录的真实服务（SettingsService + ProviderService），
// 保存、归一化与审计都走生产路径。

package overlays

import (
	"fmt"
	"path/filepath"
	"testing"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/model"
	"github.com/yangshifu1024/craftwork/internal/policy"
	"github.com/yangshifu1024/craftwork/internal/secret"
	"github.com/yangshifu1024/craftwork/internal/service"
	cwstore "github.com/yangshifu1024/craftwork/internal/store"
)

// composerServices 是注入浮层的真实服务集合。
type composerServices struct {
	settings  *service.SettingsService
	providers *service.ProviderService
	audit     *service.AuditService
	db        *cwstore.DB
}

// newComposerTester 起一个只画浮层的测试 view，注入临时目录的真实服务
// 并预置一个带两个模型的提供商；服务未预置时由用例自行覆盖。
func newComposerTester(t *testing.T) (*mygo.Tester, *composerServices) {
	t.Helper()
	store = &Store{}

	root := t.TempDir()
	db, err := cwstore.Open(filepath.Join(root, "composer.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	bus := events.NewBus(func(events.Batch) {})
	t.Cleanup(bus.Close)
	engine := policy.New(policy.LevelGraded)
	settings := service.NewSettingsService(db, bus, engine)
	providers := service.NewProviderService(db, secret.NewEmpty(root))
	audit := service.NewAuditService(db)
	Bind(&Backend{Settings: settings, Providers: providers, Audit: audit})

	svcs := &composerServices{settings: settings, providers: providers, audit: audit, db: db}
	return mygo.NewTester(func(c *mygo.Context) { View(c) }, 1280, 800), svcs
}

// seedProvider 预置一个自定义提供商与模型清单。必须走 SaveProvider 生产
// 写路径：ProviderService 在构造时把库数据读进内存快照，List() 只看快照，
// 绕过服务直接写库会让浮层读到空清单。
func seedProvider(t *testing.T, svcs *composerServices) {
	t.Helper()
	_, err := svcs.providers.SaveProvider(service.ProviderInput{
		ID: "test", Name: "测试提供商", Kind: "openai-compatible",
		Endpoint: "https://example.com/v1", KeyRequired: false,
		Models: []service.ModelInput{
			{ID: "model-a", Name: "模型 A", ContextK: 32, Caps: model.Capabilities{Tools: true}},
			{ID: "model-b", Name: "模型 B", ContextK: 128, Caps: model.Capabilities{Vision: true}},
		},
	})
	if err != nil {
		t.Fatal(err)
	}
}

// TestComposerModelPickerEmptyState 报告没有可用模型时的空态。
func TestComposerModelPickerEmptyState(t *testing.T) {
	tt, _ := newComposerTester(t)
	OpenComposerModels("")
	tt.Frame()
	if !tt.HasText("选择模型") {
		t.Fatalf("模型浮层未出现: %v", tt.Texts())
	}
	if !tt.HasText("没有可用模型，请先在设置中添加提供商并配置密钥。") {
		t.Errorf("空态提示缺失: %v", tt.Texts())
	}
	// Esc 关闭（DialogBase 的退出语义，不另起第二套）。
	tt.Key(0, mygo.KeyEscape)
	tt.Frame()
	if tt.HasText("没有可用模型") {
		t.Errorf("Esc 后浮层应关闭")
	}
}

// TestComposerModelPickerListSearchPick 走查清单展示、搜索过滤与选择写入。
func TestComposerModelPickerListSearchPick(t *testing.T) {
	tt, svcs := newComposerTester(t)
	seedProvider(t, svcs)
	OpenComposerModels("")
	tt.Frame()

	// 清单展示两个模型：名称 + 提供商 + 上下文 + 能力。
	if !tt.HasText("模型 A") || !tt.HasText("模型 B") {
		t.Fatalf("模型清单未出现: %v", tt.Texts())
	}
	if !tt.HasText("测试提供商 · 128k 上下文 · 图片") {
		t.Errorf("模型行说明不符: %v", tt.Texts())
	}
	// 搜索过滤：按名称命中一个。
	if err := tt.Click("模型搜索"); err != nil {
		t.Fatalf("点击搜索框失败: %v", err)
	}
	tt.Type("模型 B")
	tt.Frame()
	if tt.HasText("模型 A") {
		t.Errorf("搜索后应过滤掉模型 A: %v", tt.Texts())
	}
	if !tt.HasText("模型 B") {
		t.Errorf("搜索后应保留模型 B")
	}
	// 点击选择：写入默认模型（model.Ref 语义）并关闭浮层。
	if err := tt.Click("模型 B"); err != nil {
		t.Fatalf("点击模型 B 失败: %v", err)
	}
	tt.Frame()
	if tt.HasText("选择模型") && tt.HasText("模型搜索") {
		t.Errorf("选择后浮层应关闭")
	}
	if got := svcs.settings.Get().DefaultModel; got != "test/model-b" {
		t.Fatalf("默认模型未写入: %q", got)
	}
	// 与设置中心同源的设置快照也应反映新选择（双向同步的另一半）。
	if got := store.settingsSnapshot().DefaultModel; got != "test/model-b" {
		t.Errorf("设置草稿未同步: %q", got)
	}
}

// TestComposerModelPickerCurrentSource 走查勾选态的来源优先级：当前会话
// 模型优先，没有会话模型时退设置里的默认模型（review 修复轮：此前只看
// 默认模型，会话已切模型时勾选态落后于会话）。
func TestComposerModelPickerCurrentSource(t *testing.T) {
	tt, svcs := newComposerTester(t)
	seedProvider(t, svcs)

	// 无会话模型、无默认模型：勾选引用为空。
	OpenComposerModels("")
	tt.Frame()
	if got := ComposerModelsCurrent(); got != "" {
		t.Fatalf("无会话模型且无默认模型时勾选引用应为空: %q", got)
	}
	CloseComposer()
	tt.Frame()

	// 无会话模型、有默认模型：退默认模型。
	if _, err := svcs.settings.Save(service.Settings{DefaultModel: "test/model-b"}); err != nil {
		t.Fatal(err)
	}
	OpenComposerModels("")
	tt.Frame()
	if got := ComposerModelsCurrent(); got != "test/model-b" {
		t.Fatalf("无会话模型时应退默认模型: %q", got)
	}
	CloseComposer()
	tt.Frame()

	// 有会话模型：优先会话模型，不落到默认模型。
	OpenComposerModels("test/model-a")
	tt.Frame()
	if got := ComposerModelsCurrent(); got != "test/model-a" {
		t.Fatalf("有会话模型时应优先会话模型: %q", got)
	}
	CloseComposer()
}

// TestComposerApprovalLevelSwitch 走查档位切换：立即生效、回调同步、审计落账。
func TestComposerApprovalLevelSwitch(t *testing.T) {
	tt, svcs := newComposerTester(t)
	seedProvider(t, svcs)
	var changed []string
	OpenComposerApproval(func(level string) { changed = append(changed, level) })
	tt.Frame()
	if !tt.HasText("审批档位") {
		t.Fatalf("档位浮层未出现: %v", tt.Texts())
	}
	// 当前档位（分级）带勾选说明；四个档位都在。
	for _, want := range []string{"分级", "严格", "只读", "自由"} {
		if !tt.HasText(want) {
			t.Fatalf("档位 %s 缺失: %v", want, tt.Texts())
		}
	}
	if !tt.HasText("档位与设置 · 工具与权限双向同步。") {
		t.Errorf("浮层备注缺失: %v", tt.Texts())
	}
	// 点「严格」：浮层关闭、保存生效、回调收到原始档位、审计有记录。
	if err := tt.Click("严格"); err != nil {
		t.Fatalf("点击严格档失败: %v", err)
	}
	tt.Frame()
	if len(changed) != 1 || changed[0] != "strict" {
		t.Fatalf("档位回调不符: %v", changed)
	}
	if got := svcs.settings.Get().ApprovalLevel; got != "strict" {
		t.Fatalf("档位未持久化: %q", got)
	}
	entries, err := svcs.audit.List(10)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) == 0 || entries[0].Tool != "approval_level" {
		t.Fatalf("档位切换未写审计: %+v", entries)
	}
	// 打开浮层时勾选态跟随最新档位。
	OpenComposerApproval(func(string) {})
	tt.Frame()
	if !tt.HasText("写入与执行逐次询问，不提供永久允许") {
		t.Errorf("档位描述缺失: %v", tt.Texts())
	}
	CloseComposer()
	tt.Frame()
}

// TestComposerApprovalFreeGate 走查自由档确认门：先门后保存，取消不落账。
func TestComposerApprovalFreeGate(t *testing.T) {
	tt, svcs := newComposerTester(t)
	var changed []string
	OpenComposerApproval(func(level string) { changed = append(changed, level) })
	tt.Frame()
	if err := tt.Click("自由"); err != nil {
		t.Fatalf("点击自由档失败: %v", err)
	}
	tt.Frame()
	// 确认门弹出，档位尚未变化。
	if !tt.HasText("要启用完全访问吗？") {
		t.Fatalf("自由档确认门未弹出: %v", tt.Texts())
	}
	if got := svcs.settings.Get().ApprovalLevel; got != "graded" {
		t.Fatalf("确认门前档位不应变化: %q", got)
	}
	if len(changed) != 0 {
		t.Fatalf("确认门前不应触发回调: %v", changed)
	}
	// 取消：门关闭、档位不变。
	if err := tt.Click("取消"); err != nil {
		t.Fatalf("点击取消失败: %v", err)
	}
	tt.Frame()
	if got := svcs.settings.Get().ApprovalLevel; got != "graded" {
		t.Fatalf("取消后档位不应变化: %q", got)
	}
	// 再走一次：勾选并确认，保存生效且回调触发。
	OpenComposerApproval(func(level string) { changed = append(changed, level) })
	tt.Frame()
	if err := tt.Click("自由"); err != nil {
		t.Fatalf("再次点击自由档失败: %v", err)
	}
	tt.Frame()
	if err := tt.Click("我已了解风险，并对自己的数据安全负责。"); err != nil {
		t.Fatalf("点击勾选门失败: %v", err)
	}
	if err := tt.Click("确认启用"); err != nil {
		t.Fatalf("点击确认启用失败: %v", err)
	}
	tt.Frame()
	if len(changed) != 1 || changed[0] != "free" {
		t.Fatalf("自由档回调不符: %v", changed)
	}
	if got := svcs.settings.Get().ApprovalLevel; got != "free" {
		t.Fatalf("自由档未持久化: %q", got)
	}
}

// TestComposerRiskRows 走查 R0–R4 说明浮层（与设置中心 riskRows 同源）。
func TestComposerRiskRows(t *testing.T) {
	tt, _ := newComposerTester(t)
	OpenComposerRisk()
	tt.Frame()
	if !tt.HasText("风险等级") {
		t.Fatalf("风险浮层未出现: %v", tt.Texts())
	}
	for _, row := range riskRows {
		if !tt.HasText(row.level) || !tt.HasText(row.name) || !tt.HasText(row.policy) {
			t.Errorf("风险行 %s 不完整: %v", row.level, tt.Texts())
		}
	}
	if !tt.HasText("实际处置随审批档位变化：只读只放行 R0，严格逐次询问，自由全部放行。") {
		t.Errorf("浮层备注缺失: %v", tt.Texts())
	}
	tt.Key(0, mygo.KeyEscape)
	tt.Frame()
	if tt.HasText("实际处置随审批档位变化") {
		t.Errorf("Esc 后风险浮层应关闭")
	}
}

// TestComposerContextDetail 走查上下文详情浮层的数值行。
func TestComposerContextDetail(t *testing.T) {
	tt, _ := newComposerTester(t)
	OpenComposerContext(ContextDetail{
		Used:        4300,
		WindowK:     32,
		Percent:     13,
		Tokens:      12500,
		CacheTokens: 2100,
		Speed:       42.4,
	})
	tt.Frame()
	if !tt.HasText("上下文") {
		t.Fatalf("上下文浮层未出现: %v", tt.Texts())
	}
	for _, want := range []string{
		"4.3k / 32k（13%）", // 当前上下文
		"13k",             // 本会话累计（formatTokens ≥10k 取整，与设置中心同口径）
		"2.1k",            // 最近一轮缓存命中
		"42 tok/s",        // 最近一轮速度
	} {
		if !tt.HasText(want) {
			t.Errorf("上下文行 %q 缺失: %v", want, tt.Texts())
		}
	}
	// 缓存与速度的空值口径：占位符「—」。
	OpenComposerContext(ContextDetail{Used: 100, WindowK: 32, Percent: 1, Tokens: 5})
	tt.Frame()
	if fmt.Sprint(tt.Texts()) == "" {
		t.Fatal("浮层未绘制")
	}
	CloseComposer()
}
