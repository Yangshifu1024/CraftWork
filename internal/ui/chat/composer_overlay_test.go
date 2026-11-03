// composer 接线走查（11 号票）：输入区的档位 / 上下文 / 模型三处展示点击
// 后打开对应浮层，R0–R4 说明入口直达风险对照。浮层 view 与聊天列同帧叠加
//（与 RootView 的组合同构）；浮层本体与服务路径的完整走查在 overlays 包
//（composer_test.go），这里只断言接线。

package chat

import (
	"testing"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/overlays"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// newComposerOverlayTester 在聊天列之上叠加浮层 view（RootView 同构）。
// 用例结束关闭浮层，包级单例不串扰后续用例。
func newComposerOverlayTester(t *testing.T) (*Chat, *mygo.Tester) {
	t.Helper()
	s := NewChat()
	view := func(c *mygo.Context) {
		pal := theme.Apply(c)
		s.View(c, pal)
		overlays.View(c)
	}
	t.Cleanup(func() { overlays.CloseComposer() })
	return s, mygo.NewTester(view, 1200, 800)
}

// TestComposerBarOpensModelPicker 模型名点击打开模型选择浮层（无服务注入
// 时为空态提示，浮层语义不变）。
func TestComposerBarOpensModelPicker(t *testing.T) {
	s, tt := newComposerOverlayTester(t)
	tt.Frame()
	clickText(t, tt, "切换模型")
	tt.Frame()
	if !tt.HasText("选择模型") {
		t.Fatalf("模型浮层未打开: %v", tt.Texts())
	}
	if !tt.HasText("没有可用模型，请先在设置中添加提供商并配置密钥。") {
		t.Errorf("空态提示缺失: %v", tt.Texts())
	}
	_ = s
}

// TestComposerBarOpensApprovalAndSyncs 档位展示点击打开档位浮层；选新档位
// 后输入区只读展示同步（回调链，无服务注入时同步语义仍成立）。初始档位走
// 装配同款 SetApprovalLevel 注入。
func TestComposerBarOpensApprovalAndSyncs(t *testing.T) {
	s, tt := newComposerOverlayTester(t)
	s.SetApprovalLevel("分级")
	tt.Frame()
	if !tt.HasText("分级") {
		t.Fatalf("档位初始展示缺失: %v", tt.Texts())
	}
	clickText(t, tt, "切换审批档位")
	tt.Frame()
	if !tt.HasText("审批档位") {
		t.Fatalf("档位浮层未打开: %v", tt.Texts())
	}
	clickText(t, tt, "严格")
	settleClick(tt)
	if got := s.levelLabel(); got != "严格" {
		t.Fatalf("档位展示未同步: %q", got)
	}
	// 浮层关闭判据用浮层特有备注（输入区按钮「切换审批档位」含同子串，
	// 不能作判据）。
	if tt.HasText("档位与设置 · 工具与权限双向同步。") {
		t.Errorf("保存后浮层应关闭")
	}
}

// TestComposerBarOpensContextDetail 上下文占用点击打开详情浮层（缺省 32k
// 窗口恒有占用提示）。
func TestComposerBarOpensContextDetail(t *testing.T) {
	_, tt := newComposerOverlayTester(t)
	tt.Frame()
	clickText(t, tt, "上下文详情")
	tt.Frame()
	if !tt.HasText("上下文") {
		t.Fatalf("上下文浮层未打开: %v", tt.Texts())
	}
	if !tt.HasText("当前上下文") {
		t.Errorf("详情行缺失: %v", tt.Texts())
	}
}

// TestComposerBarOpensRiskRows 风险等级说明入口直达 R0–R4 对照浮层。
func TestComposerBarOpensRiskRows(t *testing.T) {
	_, tt := newComposerOverlayTester(t)
	tt.Frame()
	clickText(t, tt, "风险等级说明")
	tt.Frame()
	if !tt.HasText("风险等级") {
		t.Fatalf("风险浮层未打开: %v", tt.Texts())
	}
	for _, want := range []string{"R0", "R4"} {
		if !tt.HasText(want) {
			t.Errorf("风险行 %s 缺失: %v", want, tt.Texts())
		}
	}
}
