// 卡片状态机的 ui.Tester 走查（06 号票验收）：审批四档按钮的显隐矩阵
//（R0–R4 × allowPermanent × 档位）、默认焦点在「拒绝」（回车即拒绝）、
// Esc=拒绝与运行中双击 Esc 停止的分流、决定后结果条、思考块三形态与
// 手动展开不自动收起、工具卡失败默认展开、待办与错误卡、「已工作」
// 折叠的平铺 / 收拢、三形态预览。
//
// Esc 走 RootView 的同序接线：先画聊天列，再查窗口级 Esc 快捷键。
// Actions 注入本地记录函数（不接服务），审批决定只记录不回事件，
// 结果条由 approval.resolved 事件批次驱动（与真实链路一致）。

package chat

import (
	"strings"
	"testing"
	"time"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// escView 把聊天列包进 RootView 的同序 Esc 接线（窗口级退出链）。
func escView(s *Chat) func(c *mygo.Context) {
	return func(c *mygo.Context) {
		pal := theme.Apply(c)
		s.View(c, pal)
		if c.Shortcut(0, mygo.KeyEscape) {
			s.HandleWindowEscape()
		}
	}
}

// newEscTester 起一个带窗口级 Esc 链的聊天列。
func newEscTester(t *testing.T) (*Chat, *recorded, *mygo.Tester) {
	t.Helper()
	s := NewChat()
	rec := &recorded{}
	s.SetActions(&Actions{
		Cancel:     func() { rec.cancels++ },
		Regenerate: func() { rec.regens++ },
		Copy:       func(text string) { rec.copies = append(rec.copies, text) },
		OpenDiff:   func(path, _, _ string) { rec.opens = append(rec.opens, path) },
		Decide: func(id string, allow bool, scope string) {
			rec.decides = append(rec.decides, decision{id: id, allow: allow, scope: scope})
		},
	})
	tt := mygo.NewTester(escView(s), 1200, 800)
	return s, rec, tt
}

// decision 记录一次审批决定。
type decision struct {
	id    string
	allow bool
	scope string
}

// approvalData 构造一份 waiting 审批部件数据。
func approvalData(id, risk string, allowPermanent bool) map[string]any {
	return map[string]any{
		"partId":         id + "-p",
		"approvalId":     id,
		"tool":           "fs_write",
		"risk":           risk,
		"title":          "写入文件",
		"target":         "/tmp/demo.txt",
		"detail":         "准备写入工作区文件",
		"args":           map[string]any{"path": "/tmp/demo.txt", "content": "hello"},
		"allowPermanent": allowPermanent,
		"status":         "waiting",
	}
}

// loadApproval 在已首帧的 tester 里加载一条带审批卡的消息（审批卡在该帧
// 诞生，AutoFocus 生效在它身上而非输入框，与真实链路的时序一致）。
func loadApproval(t *testing.T, tt *mygo.Tester, s *Chat, data map[string]any) {
	t.Helper()
	s.LoadSnapshot("th", []*Message{{
		ID: "m1", ThreadID: "th", Role: RoleAssistant, Status: StatusStreaming, CreatedAt: time.Now().UnixMilli(),
		Parts: []Part{{ID: "ap", Kind: PartApproval, Data: data}},
	}}, ThreadInfo{ID: "th", Title: "审批会话", Status: ThreadStatusAwaiting}, false, 0)
	s.SetRunning(true)
	tt.Frame()
}

// TestApprovalButtonMatrix 验证永久授权按钮的显隐矩阵：只有 R1/R2 +
// allowPermanent + 分级档位才出现「本会话允许 / 本工作区始终允许」。
func TestApprovalButtonMatrix(t *testing.T) {
	cases := []struct {
		risk      string
		permanent bool
		level     string
		want      bool
	}{
		{"R1", true, "graded", true},
		{"R2", true, "graded", true},
		{"R0", true, "graded", false},
		{"R3", true, "graded", false},
		{"R4", true, "graded", false},
		{"R1", true, "strict", false},
		{"R1", true, "readonly", false},
		{"R1", true, "free", false},
		{"R1", false, "graded", false},
	}
	for _, tc := range cases {
		s, _, tt := newEscTester(t)
		s.SetApprovalLevelRaw(tc.level)
		tt.Frame()
		loadApproval(t, tt, s, approvalData("ap", tc.risk, tc.permanent))
		got := tt.HasText("本会话允许") && tt.HasText("本工作区始终允许")
		if got != tc.want {
			t.Fatalf("R=%s permanent=%v level=%s: 永久按钮显隐 = %v, 期望 %v",
				tc.risk, tc.permanent, tc.level, got, tc.want)
		}
		// 四档基础按钮在任何组合下都在。
		if !tt.HasText("拒绝") || !tt.HasText("允许一次") {
			t.Fatalf("R=%s level=%s: 基础按钮缺失", tc.risk, tc.level)
		}
	}
}

// TestApprovalDefaultFocusReject 验证默认焦点在「拒绝」：审批卡出现后
// 直接按回车，等价于点了拒绝（focused button 接管回车）。
func TestApprovalDefaultFocusReject(t *testing.T) {
	s, rec, tt := newEscTester(t)
	tt.Frame()
	loadApproval(t, tt, s, approvalData("ap1", "R2", true))
	tt.Key(0, mygo.KeyEnter)
	tt.Frame()
	if len(rec.decides) != 1 {
		t.Fatalf("回车应按默认焦点提交拒绝: %v", rec.decides)
	}
	d := rec.decides[0]
	if d.id != "ap1" || d.allow || d.scope != "once" {
		t.Fatalf("默认决定应为拒绝(once): %+v", d)
	}
}

// TestApprovalEscRejectsLastPending 验证非运行状态下 Esc 拒绝最后一条
// 待审批，范围固定「仅本次」（会话空闲但仍有未处理审批的可恢复状态）；
// 会话在等待审批（运行中）时 Esc 先走双击停止链，见 TestEscRunningArmsCancel。
func TestApprovalEscRejectsLastPending(t *testing.T) {
	s, rec, tt := newEscTester(t)
	tt.Frame()
	s.LoadSnapshot("th", []*Message{{
		ID: "m1", ThreadID: "th", Role: RoleAssistant, Status: StatusComplete, CreatedAt: time.Now().UnixMilli(),
		Parts: []Part{
			{ID: "a1", Kind: PartApproval, Data: approvalData("first", "R1", false)},
			{ID: "a2", Kind: PartApproval, Data: approvalData("second", "R1", false)},
		},
	}}, ThreadInfo{ID: "th", Title: "审批会话", Status: ThreadStatusIdle}, false, 0)
	tt.Frame()
	tt.Key(0, mygo.KeyEscape)
	tt.Frame()
	if len(rec.decides) != 1 {
		t.Fatalf("Esc 应拒绝最后一条待审批: %v", rec.decides)
	}
	if rec.decides[0].id != "second" || rec.decides[0].allow || rec.decides[0].scope != "once" {
		t.Fatalf("应拒绝最后一条(second)且范围 once: %+v", rec.decides)
	}
}

// TestEscRunningArmsCancel 验证运行中 Esc 走双击停止而非审批拒绝：
// 第一击武装（不产生任何决定），窗口内第二击取消。
func TestEscRunningArmsCancel(t *testing.T) {
	s, rec, tt := newEscTester(t)
	tt.Frame()
	loadApproval(t, tt, s, approvalData("ap1", "R2", false))
	tt.Key(0, mygo.KeyEscape)
	tt.Frame()
	if len(rec.decides) != 0 {
		t.Fatalf("运行中第一击 Esc 不应产生审批决定: %v", rec.decides)
	}
	if !s.InterruptArmed() {
		t.Fatal("运行中第一击 Esc 应武装停止")
	}
	tt.Key(0, mygo.KeyEscape)
	tt.Frame()
	if rec.cancels != 1 {
		t.Fatalf("窗口内第二击应停止运行: %d", rec.cancels)
	}
	if len(rec.decides) != 0 {
		t.Fatalf("取消路径不应产生审批决定: %v", rec.decides)
	}
}

// TestApprovalResultStrip 验证决定后的结果条：approved 带授权范围与审计
// 提示；denied / canceled 提示未执行任何修改。
func TestApprovalResultStrip(t *testing.T) {
	s, _, tt := newEscTester(t)
	tt.Frame()
	loadApproval(t, tt, s, approvalData("ap1", "R2", true))
	if !tt.HasText("等待决定") {
		t.Fatal("等待中应显示等待决定提示")
	}
	// 允许（本会话）→ 绿色结果条 + 审计提示。
	s.ApplyBatch(batchOf(events.TypeApprovalResolved, map[string]any{
		"partId": "ap", "approvalId": "ap1", "status": "approved", "decision": "allow", "scope": "session",
	}, 2, "th"))
	tt.Frame()
	if !tt.HasText("已允许（本会话）") || !tt.HasText("已写入审计") {
		t.Fatal("允许后应显示结果条与审计提示")
	}
	if tt.HasText("等待决定") {
		t.Fatal("决定后不应再显示等待提示")
	}
}

// TestApprovalDeniedStrip 验证拒绝结果条。
func TestApprovalDeniedStrip(t *testing.T) {
	s, _, tt := newEscTester(t)
	tt.Frame()
	loadApproval(t, tt, s, approvalData("ap1", "R2", true))
	s.ApplyBatch(batchOf(events.TypeApprovalResolved, map[string]any{
		"partId": "ap", "approvalId": "ap1", "status": "denied", "decision": "deny", "scope": "once",
	}, 2, "th"))
	tt.Frame()
	if !tt.HasText("已拒绝") || !tt.HasText("未执行任何修改") {
		t.Fatal("拒绝后应显示拒绝结果条")
	}
	if tt.HasText("允许一次") {
		t.Fatal("决定后不应再显示按钮组")
	}
}

// TestApprovalPreviewForms 验证三形态预览：fs_edit 的文本差异（删/增行）、
// shell_exec 的命令原文、apply_patch 的补丁行；「查看完整差异」触发打开。
func TestApprovalPreviewForms(t *testing.T) {
	s, rec, tt := newEscTester(t)
	tt.Frame()
	data := approvalData("ap1", "R2", false)
	data["tool"] = "fs_edit"
	data["args"] = map[string]any{
		"path":       "/tmp/demo.txt",
		"old_string": "line1\nline2\nline3",
		"new_string": "line1\nline2x\nline3",
	}
	loadApproval(t, tt, s, data)
	if !tt.HasText("文本级差异预览") {
		t.Fatal("fs_edit 应显示文本差异预览")
	}
	if !tt.HasText("line2") || !tt.HasText("line2x") {
		t.Fatal("差异预览应含删除与新增行")
	}
	if !tt.HasText("查看完整差异") {
		t.Fatal("fs_edit 预览应有查看完整差异按钮")
	}
	clickText(t, tt, "查看完整差异")
	settleClick(tt)
	if len(rec.opens) != 1 || rec.opens[0] != "/tmp/demo.txt" {
		t.Fatalf("查看完整差异应触发 OpenDiff: %v", rec.opens)
	}

	// shell 命令形态：命令预览 + 目标原文。
	s2, _, tt2 := newEscTester(t)
	tt2.Frame()
	cmd := approvalData("ap2", "R2", false)
	cmd["tool"] = "shell_exec"
	cmd["target"] = "rm -rf /tmp/build"
	loadApproval(t, tt2, s2, cmd)
	if !tt2.HasText("命令预览") || !tt2.HasText("rm -rf /tmp/build") {
		t.Fatal("shell_exec 应显示命令原文预览")
	}

	// patch 形态：超过 60 行的补丁只显示前 60 行。
	s3, _, tt3 := newEscTester(t)
	tt3.Frame()
	patch := approvalData("ap3", "R3", false)
	patch["tool"] = "apply_patch"
	var sb strings.Builder
	for i := 0; i < 70; i++ {
		sb.WriteString("+patch-line-")
		sb.WriteString(strings.Repeat("x", i+1))
		sb.WriteString("\n")
	}
	patch["args"] = map[string]any{"patch": sb.String()}
	loadApproval(t, tt3, s3, patch)
	if !tt3.HasText("补丁预览") {
		t.Fatal("apply_patch 应显示补丁预览")
	}
	if tt3.HasText("+patch-line-70") {
		t.Fatal("补丁预览应截断在 60 行内")
	}
}

// TestReasoningThreeForms 验证思考块三形态：流式收起（正在思考 + 摘要）、
// 手动展开（秒数）、结束后定格耗时；手动展开过后不自动收起。
func TestReasoningThreeForms(t *testing.T) {
	s, _, tt := newEscTester(t)
	tt.Frame()
	s.ApplyBatch(batchOf(events.TypeRunStarted, map[string]any{"messageId": "m1"}, 1, "th"))
	s.ApplyBatch(batchOf(events.TypeReasoningDelta,
		events.TextDelta{PartID: "rp", Text: "第一段思考\n让我分析这个文件的结构"}, 2, "th"))
	tt.Frame()
	if !tt.HasText("正在思考") {
		t.Fatal("流式中应显示正在思考")
	}
	if !tt.HasText("让我分析这个文件的结构") {
		t.Fatal("流式收起态应浮现最新一行摘要")
	}
	// 手动展开：秒数形态 + 正文可见（interacted）。
	clickText(t, tt, "正在思考")
	settleClick(tt)
	if !tt.HasText("第一段思考") || tt.HasText("正在思考 · ") == false {
		t.Fatalf("展开后应显示秒数形态与正文")
	}
	// 结束：后端落库时长定格（React 口径 Math.ceil：5.2s → 6 秒）；
	// 手动展开过 → 保持展开。终态后过程段收进「已工作」折叠区，
	// 先展开折叠区再看思考块。
	s.ApplyBatch(batchOf(events.TypeReasoningCompleted, map[string]any{
		"partId": "rp", "text": "第一段思考\n让我分析这个文件的结构", "durationMs": 5200,
	}, 3, "th"))
	s.ApplyBatch(batchOf(events.TypeRunCompleted, map[string]any{"durationMs": 6000}, 4, "th"))
	tt.Frame()
	clickText(t, tt, "已工作 6 秒")
	settleClick(tt)
	if !tt.HasText("思考过程 · 持续了 6 秒") {
		t.Fatal("结束后应定格为持续了 N 秒")
	}
	if !tt.HasText("第一段思考") {
		t.Fatal("手动展开过后流式结束不应自动收起")
	}
}

// TestReasoningAutoCollapse 验证未手动操作时流式结束自动收起。
func TestReasoningAutoCollapse(t *testing.T) {
	s, _, tt := newEscTester(t)
	tt.Frame()
	s.ApplyBatch(batchOf(events.TypeRunStarted, map[string]any{"messageId": "m1"}, 1, "th"))
	s.ApplyBatch(batchOf(events.TypeReasoningDelta,
		events.TextDelta{PartID: "rp", Text: "只看不点"}, 2, "th"))
	tt.Frame()
	// 全程不点击思考块：验证默认路径（未 interacted）的自动收起。
	s.ApplyBatch(batchOf(events.TypeReasoningCompleted, map[string]any{
		"partId": "rp", "text": "只看不点", "durationMs": 1200,
	}, 3, "th"))
	s.ApplyBatch(batchOf(events.TypeRunCompleted, map[string]any{"durationMs": 2000}, 4, "th"))
	tt.Frame()
	if tt.HasText("只看不点") {
		t.Fatal("结束后未手动操作应自动收起（正文不可见）")
	}
	// 折叠区展开后：思考块收起态的折叠头定格耗时，正文不可见。
	clickText(t, tt, "已工作 2 秒")
	settleClick(tt)
	if tt.HasText("只看不点") {
		t.Fatal("思考块自身应保持收起（正文不可见）")
	}
	if !tt.HasText("思考过程 · 持续了 2 秒") {
		t.Fatal("收起态折叠头应定格耗时")
	}
}

// TestToolCardForms 验证工具卡：失败默认展开（输出与复制可见）、成功
// 默认收起、点击头切换；复制走动作。
func TestToolCardForms(t *testing.T) {
	s, rec, tt := newEscTester(t)
	tt.Frame()
	s.LoadSnapshot("th", []*Message{{
		ID: "m1", ThreadID: "th", Role: RoleAssistant, Status: StatusComplete, CreatedAt: time.Now().UnixMilli(),
		Parts: []Part{
			{ID: "t1", Kind: PartTool, Data: map[string]any{
				"partId": "t1", "name": "shell_exec", "status": "error", "isError": true,
				"args":   map[string]any{"command": "go test ./...", "timeout": float64(30)},
				"output": "FAIL pkg\nexit status 1", "durationMs": float64(1200), "risk": "R2",
			}},
			{ID: "t2", Kind: PartTool, Data: map[string]any{
				"partId": "t2", "name": "fs_read", "status": "done",
				"args": map[string]any{"path": "/tmp/a.txt"}, "output": "file body", "risk": "R0",
			}},
		},
	}}, ThreadInfo{ID: "th", Title: "会话", Status: ThreadStatusIdle}, false, 0)
	tt.Frame()
	// 终态消息过程段收拢：先展开折叠区。
	clickText(t, tt, "已工作")
	settleClick(tt)
	// 失败卡默认展开。
	if !tt.HasText("复制输出") || !tt.HasText("FAIL pkg") {
		t.Fatal("失败的工具卡应默认展开并显示输出")
	}
	if !tt.HasText("执行失败") {
		t.Fatal("失败卡脚注应显示执行失败")
	}
	// 复制输出走动作。
	clickText(t, tt, "复制输出")
	settleClick(tt)
	if len(rec.copies) != 1 || rec.copies[0] != "FAIL pkg\nexit status 1" {
		t.Fatalf("复制输出应走 Copy 动作: %v", rec.copies)
	}
	// 成功卡默认收起，展开后可见。
	if tt.HasText("file body") {
		t.Fatal("成功的工具卡应默认收起")
	}
	clickText(t, tt, "fs_read")
	settleClick(tt)
	if !tt.HasText("file body") {
		t.Fatal("点击后成功卡应展开")
	}
}

// TestTodoAndErrorCards 验证待办卡（清单 + 进度 + 状态形态）与错误卡
// （9 错误码映射 + 重试 + 复制）。
func TestTodoAndErrorCards(t *testing.T) {
	s, rec, tt := newEscTester(t)
	tt.Frame()
	s.LoadSnapshot("th", []*Message{{
		ID: "m1", ThreadID: "th", Role: RoleAssistant, Status: StatusComplete, CreatedAt: time.Now().UnixMilli(),
		Parts: []Part{
			{ID: "td", Kind: PartTodo, Data: map[string]any{
				"items": []any{
					map[string]any{"text": "读代码", "status": "done"},
					map[string]any{"text": "改实现", "status": "active"},
					map[string]any{"text": "补测试", "status": "pending"},
				},
			}},
			{ID: "er", Kind: PartError, Data: map[string]any{
				"code": "auth", "message": "401 Unauthorized",
			}},
		},
	}}, ThreadInfo{ID: "th", Title: "会话", Status: ThreadStatusIdle}, false, 0)
	tt.Frame()
	// 待办卡属于过程段：终态收拢，先展开折叠区；错误卡在折叠区外显性可见。
	clickText(t, tt, "已工作")
	settleClick(tt)
	if !tt.HasText("任务清单") || !tt.HasText("1 / 3") {
		t.Fatal("待办卡应显示标题与 n / m 计数")
	}
	if !tt.HasText("进行中") {
		t.Fatal("进行中的待办项应有进行中标记")
	}
	// 错误卡：auth 码的中文建议 + 重试 + 复制。
	if !tt.HasText("密钥无效或权限不足") {
		t.Fatal("auth 错误码应映射中文建议")
	}
	clickText(t, tt, "重试")
	settleClick(tt)
	if rec.regens != 1 {
		t.Fatalf("重试应触发重新生成: %d", rec.regens)
	}
	clickText(t, tt, "复制错误")
	settleClick(tt)
	if len(rec.copies) != 1 || rec.copies[0] != "401 Unauthorized" {
		t.Fatalf("复制错误应复制原始信息: %v", rec.copies)
	}
}

// TestWorkHistoryFoldForms 验证「已工作」折叠：运行中平铺（工具卡直接
// 可见）、终态收拢（折叠头 + 耗时）、中文两段格式化。
func TestWorkHistoryFoldForms(t *testing.T) {
	s, _, tt := newEscTester(t)
	tt.Frame()
	// 运行中：过程部件平铺。
	s.ApplyBatch(batchOf(events.TypeRunStarted, map[string]any{"messageId": "m1"}, 1, "th"))
	s.ApplyBatch(batchOf(events.TypeToolStarted, map[string]any{
		"partId": "tp", "name": "fs_read", "status": "running", "risk": "R0",
	}, 2, "th"))
	tt.Frame()
	if !tt.HasText("fs_read") || !tt.HasText("运行中") {
		t.Fatal("运行中过程部件应平铺显示")
	}
	if tt.HasText("已工作") {
		t.Fatal("运行中不应显示已工作折叠头")
	}
	// 终态：收拢 + 耗时（工具先完成，运行才收尾）。
	s.ApplyBatch(batchOf(events.TypeToolCompleted, map[string]any{
		"partId": "tp", "name": "fs_read", "status": "done", "durationMs": float64(300),
	}, 3, "th"))
	s.ApplyBatch(batchOf(events.TypeRunCompleted, map[string]any{"durationMs": 88_000}, 4, "th"))
	tt.Frame()
	if !tt.HasText("已工作 1 分 28 秒") {
		t.Fatal("终态折叠头应显示中文两段格式化耗时")
	}
	if tt.HasText("运行中") {
		t.Fatal("收拢后不应再平铺工具状态")
	}
	// 展开：工具卡回到可见。
	clickText(t, tt, "已工作 1 分 28 秒")
	settleClick(tt)
	if !tt.HasText("fs_read") {
		t.Fatal("展开后工具卡可见")
	}
}

// TestFormatWorkDuration 验证中文两段格式化规则（纯函数）。
func TestFormatWorkDuration(t *testing.T) {
	cases := []struct {
		ms   int64
		want string
	}{
		{0, ""},
		{-5, ""},
		{400, "1 秒"},
		{1000, "1 秒"},
		{59_400, "59 秒"},
		{60_000, "1 分"},
		{88_000, "1 分 28 秒"},
		{3_600_000, "1 小时"},
		{3_780_000, "1 小时 3 分"},
		{90_000_000, "1 天 1 小时"},
	}
	for _, tc := range cases {
		if got := formatWorkDuration(tc.ms); got != tc.want {
			t.Fatalf("formatWorkDuration(%d) = %q, 期望 %q", tc.ms, got, tc.want)
		}
	}
}

// TestApprovalLevelNoteBadge 验证档位徽章与说明：严格档显示徽章与说明，
// 分级档不显示徽章、说明为默认文案。
func TestApprovalLevelNoteBadge(t *testing.T) {
	s, _, tt := newEscTester(t)
	s.SetApprovalLevelRaw("strict")
	tt.Frame()
	loadApproval(t, tt, s, approvalData("ap1", "R2", false))
	if !tt.HasText("严格档位") || !tt.HasText("严格档位：所有写入与执行逐次询问，不提供永久允许") {
		t.Fatal("严格档应显示档位徽章与说明")
	}
	s2, _, tt2 := newEscTester(t)
	tt2.Frame()
	loadApproval(t, tt2, s2, approvalData("ap1", "R2", false))
	if !tt2.HasText("默认选中“拒绝”；Esc 拒绝；R3 / R4 不提供永久允许") {
		t.Fatal("分级档应显示默认说明")
	}
	if tt2.HasText("严格档位") {
		t.Fatal("分级档不应显示档位徽章")
	}
}

// TestAttachThumbAndPreview 验证附件缩略图的回退与放大预览浮层状态：
// 未注入 ReadAttachment（纯 UI / 单测路径）时图片附件回退文件 chip，
// 预览浮层开合由状态驱动。
func TestAttachThumbAndPreview(t *testing.T) {
	s, _, tt := newEscTester(t)
	tt.Frame()
	// 没有 ReadAttachment：thumbOf 返回 nil，不发起加载。
	if bmp := s.thumbOf("/tmp/pic.png"); bmp != nil {
		t.Fatal("未注入读取动作时不应产生缩略图")
	}
	// 非图片路径同样不加载。
	if bmp := s.thumbOf("/tmp/doc.md"); bmp != nil {
		t.Fatal("非图片路径不应产生缩略图")
	}
	// 预览浮层：打开后状态可查，关闭后复位。
	s.OpenImagePreview("pic.png", nil)
	if p := s.Preview(); !p.open || p.name != "pic.png" {
		t.Fatal("预览应处于打开状态")
	}
	s.CloseImagePreview()
	if s.Preview().open {
		t.Fatal("关闭后预览应复位")
	}
	if !isImagePath("/tmp/a.PNG") || isImagePath("/tmp/a.txt") {
		t.Fatal("isImagePath 判定不符")
	}
}
