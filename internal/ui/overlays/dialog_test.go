// 全局对话框语义的单测（ui.Tester）：Esc / 回车 / 遮罩 / 默认焦点 / 覆盖
// 收尾，对齐 React 版 ConfirmDialog / PromptDialog / FreeLevelDialog 的行为。

package overlays

import (
	"testing"

	mygo "github.com/egoist/mygo/ui"
)

// harness 起一个「按钮 + 浮层」的测试 view：点击按钮发起对应请求，
// 结果记录进切片供断言。
type harness struct {
	tt      *mygo.Tester
	confirm []bool
	prompt  []promptResult
	free    []bool
}

type promptResult struct {
	value    string
	accepted bool
}

func newHarness(t *testing.T) *harness {
	t.Helper()
	// 浮层状态是包级单例：每个用例重置，避免上一个用例遗留的打开状态
	// 拦截本用例的点击（模态遮罩会吃掉落到按钮上的按压）。
	store = &Store{}
	h := &harness{}
	view := func(c *mygo.Context) {
		if mygo.Button(c, "开确认").Clicked() {
			Confirm(c, ConfirmOpts{
				Title: "删除会话？", Description: "不可恢复。",
				ConfirmText: "删除", CancelText: "保留", Danger: true,
			}, func(ok bool) {
				h.confirm = append(h.confirm, ok)
			})
		}
		if mygo.Button(c, "开输入").Clicked() {
			Prompt(c, PromptOpts{Title: "重命名", Label: "给会话起个新名字", Value: "旧名", Placeholder: "会话名"}, func(value string, accepted bool) {
				h.prompt = append(h.prompt, promptResult{value, accepted})
			})
		}
		if mygo.Button(c, "开空输入").Clicked() {
			Prompt(c, PromptOpts{Title: "设置密钥"}, func(value string, accepted bool) {
				h.prompt = append(h.prompt, promptResult{value, accepted})
			})
		}
		if mygo.Button(c, "开自由").Clicked() {
			FreeConfirm(c, func(ok bool) {
				h.free = append(h.free, ok)
			})
		}
		if mygo.Button(c, "连发两次").Clicked() {
			// 同帧发起两个请求：后者覆盖前者，前者按取消收尾。
			Confirm(c, ConfirmOpts{Title: "请求一"}, func(ok bool) {
				h.confirm = append(h.confirm, ok)
			})
			Confirm(c, ConfirmOpts{Title: "请求二"}, func(ok bool) {
				h.confirm = append(h.confirm, ok)
			})
		}
		View(c)
	}
	h.tt = mygo.NewTester(view, 800, 600)
	return h
}

// click 是带错误检查的点击：找不到目标时直接失败，不静默跳过。
func (h *harness) click(t *testing.T, s string) {
	t.Helper()
	if err := h.tt.Click(s); err != nil {
		t.Fatalf("点击 %q 失败: %v（可见文本 %v）", s, err, h.tt.Texts())
	}
}

func TestConfirmDialogEnterConfirms(t *testing.T) {
	h := newHarness(t)
	h.click(t, "开确认")
	if !h.tt.HasText("删除会话？") {
		t.Fatalf("确认框未出现: %v", h.tt.Texts())
	}
	// 焦点默认在确认按钮上，回车即通过。
	if !h.tt.Focused("删除") {
		t.Errorf("确认按钮未持有焦点")
	}
	h.tt.Key(0, mygo.KeyEnter)
	if len(h.confirm) != 1 || !h.confirm[0] {
		t.Fatalf("回车应收尾为确认，实得 %v", h.confirm)
	}
	if h.tt.HasText("删除会话？") {
		t.Errorf("确认后对话框未关闭")
	}
}

func TestConfirmDialogEscapeCancels(t *testing.T) {
	h := newHarness(t)
	h.click(t, "开确认")
	h.tt.Key(0, mygo.KeyEscape)
	if len(h.confirm) != 1 || h.confirm[0] {
		t.Fatalf("Esc 应收尾为取消，实得 %v", h.confirm)
	}
}

func TestConfirmDialogButtonsAndDetail(t *testing.T) {
	h := newHarness(t)
	h.click(t, "开确认")
	// 点保留（取消）→ false；再次发起 → 点删除（确认）→ true。
	h.click(t, "保留")
	if len(h.confirm) != 1 || h.confirm[0] {
		t.Fatalf("取消按钮应收尾 false，实得 %v", h.confirm)
	}
	h.click(t, "开确认")
	h.click(t, "删除")
	if len(h.confirm) != 2 || !h.confirm[1] {
		t.Fatalf("确认按钮应收尾 true，实得 %v", h.confirm)
	}
}

func TestNewConfirmCancelsPendingOne(t *testing.T) {
	h := newHarness(t)
	// 同帧发起两个请求：请求一被请求二覆盖，按取消收尾。
	h.click(t, "连发两次")
	if len(h.confirm) != 1 || h.confirm[0] {
		t.Fatalf("被覆盖的请求应按取消收尾，实得 %v", h.confirm)
	}
	if !h.tt.HasText("请求二") {
		t.Fatalf("应展示后发起的请求二: %v", h.tt.Texts())
	}
	// 新请求正常工作：回车确认一次。
	h.tt.Key(0, mygo.KeyEnter)
	if len(h.confirm) != 2 || !h.confirm[1] {
		t.Fatalf("覆盖后的请求应正常收尾，实得 %v", h.confirm)
	}
}

func TestPromptDialogSubmitAndCancel(t *testing.T) {
	h := newHarness(t)
	h.click(t, "开输入")
	// 焦点默认在输入框；Esc 收尾为取消。
	if !h.tt.Focused("输入内容") {
		t.Errorf("输入框未持有焦点")
	}
	h.tt.Key(0, mygo.KeyEscape)
	if len(h.prompt) != 1 || h.prompt[0].accepted {
		t.Fatalf("Esc 应收尾为取消，实得 %v", h.prompt)
	}

	// 再次发起，清空初始值后输入并回车提交。
	h.click(t, "开输入")
	for range "旧名" {
		h.tt.Key(0, mygo.KeyBackspace)
	}
	h.tt.Type("新名字")
	h.tt.Key(0, mygo.KeyEnter)
	if len(h.prompt) != 2 || h.prompt[1].value != "新名字" || !h.prompt[1].accepted {
		t.Fatalf("回车应提交输入，实得 %v", h.prompt)
	}
}

func TestPromptDialogEmptySubmitStaysOpen(t *testing.T) {
	h := newHarness(t)
	// 初始值为空的输入框：直接回车（空内容）不提交、不关框。
	h.click(t, "开空输入")
	h.tt.Key(0, mygo.KeyEnter)
	if !h.tt.HasText("设置密钥") {
		t.Fatalf("空提交后对话框应保持打开")
	}
	// 取消收尾，结果为取消。
	h.click(t, "取消")
	if len(h.prompt) != 1 || h.prompt[0].accepted {
		t.Fatalf("空内容不应提交，取消应收尾 false: %v", h.prompt)
	}
}

func TestFreeDialogAckGate(t *testing.T) {
	h := newHarness(t)
	h.click(t, "开自由")
	if !h.tt.HasText("要启用完全访问吗？") {
		t.Fatalf("自由档位确认未出现: %v", h.tt.Texts())
	}
	// 焦点默认在「取消」：信任必须是明确选择，回车不直通确认。
	if !h.tt.Focused("取消") {
		t.Errorf("取消按钮未持有默认焦点")
	}
	h.tt.Key(0, mygo.KeyEnter)
	if len(h.free) != 1 || h.free[0] {
		t.Fatalf("回车应走默认焦点（取消），实得 %v", h.free)
	}

	// 勾选门：未勾选时点确认不生效；勾选后点确认放行。
	h.click(t, "开自由")
	h.click(t, "确认启用")
	if len(h.free) != 1 {
		t.Fatalf("未勾选时确认不应生效，实得 %v", h.free)
	}
	h.click(t, "我已了解风险，并对自己的数据安全负责。")
	h.click(t, "确认启用")
	if len(h.free) != 2 || !h.free[1] {
		t.Fatalf("勾选后确认应收尾 true，实得 %v", h.free)
	}

	// 再次打开：勾选门重置，确认按钮回到禁用。
	h.click(t, "开自由")
	h.click(t, "确认启用")
	if len(h.free) != 2 {
		t.Fatalf("重开后勾选门应重置，实得 %v", h.free)
	}
}

func TestFreeDialogEscapeCancels(t *testing.T) {
	h := newHarness(t)
	h.click(t, "开自由")
	h.tt.Key(0, mygo.KeyEscape)
	if len(h.free) != 1 || h.free[0] {
		t.Fatalf("Esc 应收尾为取消，实得 %v", h.free)
	}
}
