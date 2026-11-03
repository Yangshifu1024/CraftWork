// 全局对话框：确认、单行输入与自由档位确认。
//
// 语义对齐 React 版 ConfirmDialog / PromptDialog / FreeLevelDialog（Promise
// 语义移植为回调）：
//   - 确认框：Esc / 取消 / 遮罩 → false；Enter / 确认 → true；焦点默认在
//     确认按钮上（键盘用户回车即通过），危险操作用危险样式与警示图标。
//   - 输入框：Esc / 取消 → 取消；回车提交（去首尾空白，空内容不提交、
//     不关框）；焦点默认在输入框。
//   - 自由档位确认：能力清单固定 + 「我已了解风险」勾选门；未勾选时确认
//     按钮禁用、回车不生效；焦点默认在「取消」——信任必须是明确选择。
//   - 三者共用：遮罩点击视为取消；同类请求同时最多一个，新请求覆盖旧的
//     （旧回调按取消收尾，React 版是 Promise 永久悬挂）。
package overlays

import (
	"strings"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// ConfirmOpts 描述一次确认请求（对齐 React ConfirmRequest）。
type ConfirmOpts struct {
	// Title 是主标题。
	Title string
	// Description 是说明文字。
	Description string
	// Detail 是单行细节，例如文件路径，等宽底块展示。
	Detail string
	// ConfirmText 是确认按钮文案，缺省「确认」。
	ConfirmText string
	// CancelText 是取消按钮文案，缺省「取消」。
	CancelText string
	// Danger 为真时确认按钮用危险样式，标题配警示图标。
	Danger bool
}

type confirmState struct {
	req  *ConfirmOpts
	done func(bool)
	open bool
}

// PromptOpts 描述一次单行输入请求（对齐 React PromptRequest）。
type PromptOpts struct {
	// Title 是主标题。
	Title string
	// Label 是输入框上方的说明文字。
	Label string
	// Value 是输入框初始值。
	Value string
	// Placeholder 是占位提示。
	Placeholder string
	// ConfirmText 是确认按钮文案，缺省「确定」。
	ConfirmText string
}

type promptState struct {
	req  *PromptOpts
	done func(value string, accepted bool)
	open bool
	// value 是输入框内容；请求打开期间由 view 编辑。
	value string
}

type freeState struct {
	open bool
	done func(bool)
	// acked 是勾选门：每次打开都重置，确认门不允许「记住上一次」。
	acked bool
}

// Confirm 弹出全局确认框。done 在用户做出选择后于主线程执行：
// 确认传 true；取消、Esc、遮罩或被新请求覆盖传 false。
func Confirm(c *mygo.Context, opts ConfirmOpts, done func(ok bool)) {
	s := &store.confirm
	if s.open && s.done != nil {
		s.done(false)
	}
	*s = confirmState{req: &opts, done: done, open: true}
}

// Prompt 弹出全局单行输入框。done 在流程结束后于主线程执行：
// 提交传去首尾空白的内容与 true；取消、Esc、遮罩或被覆盖传 "" 与 false。
func Prompt(c *mygo.Context, opts PromptOpts, done func(value string, accepted bool)) {
	s := &store.prompt
	if s.open && s.done != nil {
		s.done("", false)
	}
	*s = promptState{req: &opts, done: done, open: true, value: opts.Value}
}

// FreeConfirm 弹出自由档位确认（能力清单 + 勾选门），done 收用户是否确认。
func FreeConfirm(c *mygo.Context, done func(ok bool)) {
	s := &store.free
	if s.open && s.done != nil {
		s.done(false)
	}
	*s = freeState{open: true, done: done, acked: false}
}

// resolve 結束一次对话框流程：先关框再回调并清空回调，回调里立刻发起新
// 请求不会被旧状态干扰（对齐 React resolveConfirm 的次序）。
func resolveConfirm(st *confirmState, ok bool) {
	st.open = false
	if done := st.done; done != nil {
		st.done = nil
		done(ok)
	}
}

func resolvePrompt(st *promptState, value string, accepted bool) {
	st.open = false
	if done := st.done; done != nil {
		st.done = nil
		done(value, accepted)
	}
}

func resolveFree(st *freeState, ok bool) {
	st.open = false
	if done := st.done; done != nil {
		st.done = nil
		done(ok)
	}
}

// drawConfirm 画待处理的确认框。Dismissed（Esc / 遮罩）按取消收尾。
func (s *Store) drawConfirm(c *mygo.Context) {
	st := &s.confirm
	if !st.open || st.req == nil {
		return
	}
	req := st.req
	p := pal(c)
	panel := mygo.DialogBase(c, &st.open, func(backdrop, panel mygo.Element) {
		styleBackdrop(backdrop)
		stylePanel(c, panel)
		placePanel(c, backdrop, panel, 440, 0.18)
		panel.Padding(16, 18, 14, 18).Gap(7).Children(func() {
			mygo.Row(c).AlignItems(mygo.Center).Gap(8).Children(func() {
				name := "info"
				if req.Danger {
					name = "alert"
				}
				icons.Icon(c, name).Size(16, 16).TextColor(p.Warn)
				mygo.Text(c, req.Title).Bold()
			})
			if req.Description != "" {
				mygo.Text(c, req.Description).TextColor(p.Text3)
			}
			if req.Detail != "" {
				mygo.Box(c).Fill().
					Padding(8, 10).Radius(theme.RadiusRow).
					Background(p.BgSunken).
					BorderWidth(1).BorderColor(p.Border).Children(func() {
					mygo.Text(c, req.Detail).FontSize(theme.Fonts.SM()).TextColor(p.Text2)
				})
			}
			actionsRow(c, func() {
				cancelText := req.CancelText
				if cancelText == "" {
					cancelText = "取消"
				}
				if mygo.Button(c, cancelText).Clicked() {
					resolveConfirm(st, false)
				}
				confirmText := req.ConfirmText
				if confirmText == "" {
					confirmText = "确认"
				}
				var confirm mygo.Element
				if req.Danger {
					confirm = dangerBtn(c, confirmText, false)
				} else {
					confirm = mygo.PrimaryButton(c, confirmText)
				}
				if confirm.AutoFocus().Clicked() {
					resolveConfirm(st, true)
				}
			})
		})
	})
	if panel.Dismissed() {
		resolveConfirm(st, false)
	}
}

// drawPrompt 画待处理的输入框：输入框自动聚焦，回车提交非空内容。
func (s *Store) drawPrompt(c *mygo.Context) {
	st := &s.prompt
	if !st.open || st.req == nil {
		return
	}
	req := st.req
	p := pal(c)
	panel := mygo.DialogBase(c, &st.open, func(backdrop, panel mygo.Element) {
		styleBackdrop(backdrop)
		stylePanel(c, panel)
		placePanel(c, backdrop, panel, 440, 0.18)
		panel.Padding(16, 18, 14, 18).Gap(7).Children(func() {
			mygo.Row(c).AlignItems(mygo.Center).Gap(8).Children(func() {
				icons.Icon(c, "edit").Size(16, 16).TextColor(p.Warn)
				mygo.Text(c, req.Title).Bold()
			})
			if req.Label != "" {
				mygo.Text(c, req.Label).TextColor(p.Text3)
			}
			input := mygo.TextInput(c, &st.value).
				Fill().
				Height(theme.HeightInput).
				Background(p.BgInput).Border(1, p.Border).Radius(theme.RadiusBtn).
				Label("输入内容").
				AutoFocus()
			submit := func() {
				trimmed := strings.TrimSpace(st.value)
				if trimmed == "" {
					return
				}
				resolvePrompt(st, trimmed, true)
			}
			if input.Submitted() {
				submit()
			}
			actionsRow(c, func() {
				if mygo.Button(c, "取消").Clicked() {
					resolvePrompt(st, "", false)
				}
				confirmText := req.ConfirmText
				if confirmText == "" {
					confirmText = "确定"
				}
				if primaryBtn(c, confirmText).Clicked() {
					submit()
				}
			})
		})
	})
	if panel.Dismissed() {
		resolvePrompt(st, "", false)
	}
}

// freeCapability 是自由档位放行的三类能力（对齐 FreeLevelDialog 文案）。
type freeCapability struct {
	icon  string
	title string
	desc  string
}

var freeCapabilities = []freeCapability{
	{"folder", "文件和文件夹", "读取、创建、修改和删除可访问的文件。"},
	{"terminal", "终端命令", "运行命令、安装软件或修改系统配置。"},
	{"globe", "网络和应用", "访问网络、发送数据，以及调用已连接的应用。"},
}

// drawFree 画自由档位确认：能力清单、风险提示与勾选确认门。
func (s *Store) drawFree(c *mygo.Context) {
	st := &s.free
	if !st.open {
		return
	}
	p := pal(c)
	panel := mygo.DialogBase(c, &st.open, func(backdrop, panel mygo.Element) {
		styleBackdrop(backdrop)
		stylePanel(c, panel)
		placePanel(c, backdrop, panel, 480, 0.16)
		panel.Padding(20, 22, 16, 22).Gap(10).Children(func() {
			mygo.Row(c).AlignItems(mygo.Center).Gap(9).Children(func() {
				icons.Icon(c, "alert").Size(17, 17).TextColor(p.Warn)
				mygo.Text(c, "要启用完全访问吗？").Bold()
			})
			mygo.Text(c, "启用后，任务可以直接执行以下操作，无需逐次请求你确认。").TextColor(p.Text2)
			for _, item := range freeCapabilities {
				mygo.Row(c).Fill().AlignItems(mygo.Start).Gap(10).
					Padding(10, 12).Radius(theme.RadiusCard).
					Background(p.BgSunken).BorderWidth(1).BorderColor(p.Border).Children(func() {
					icons.Icon(c, item.icon).Size(16, 16).TextColor(p.Text2)
					mygo.Column(c).Gap(theme.Space1).Children(func() {
						mygo.Text(c, item.title)
						mygo.Text(c, item.desc).FontSize(theme.Fonts.SM()).TextColor(p.Text3)
					})
				})
			}
			mygo.Text(c, "这些操作可能造成数据丢失或泄露。请仅在信任任务内容时启用。你可以随时切回其他权限模式。").
				FontSize(theme.Fonts.SM()).TextColor(p.Text3)
			mygo.Checkbox(c, &st.acked, "我已了解风险，并对自己的数据安全负责。")
			actionsRow(c, func() {
				cancel := mygo.Button(c, "取消").AutoFocus()
				if cancel.Clicked() {
					resolveFree(st, false)
				}
				confirm := dangerBtn(c, "确认启用", true).Disabled(!st.acked)
				if confirm.Clicked() {
					resolveFree(st, true)
				}
			})
		})
	})
	if panel.Dismissed() {
		resolveFree(st, false)
	}
}
