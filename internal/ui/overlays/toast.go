// 浮层通知：语气化 toast（info / ok / warn / danger），4.2 秒自动消失。
//
// Notify 把通知排队，由通知层（ToastViewportBase 自绘观感）在下一帧消费
// 并经 c.AddToast 展示。排队而不是直接 c.Toast：确认对话框的回调在用户
// 做出选择的那一帧执行，调用方手里没有可用的 *mygo.Context（立即模式不
// 留存上一帧的上下文），排队让回调里也能安心弹通知。生命周期（超时、
// 悬停暂停、Esc 关闭、最多同显 3 条）由 mygo 管理。
package overlays

import (
	"fmt"
	"time"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// Tone 是通知语气，对齐 React 版 Toast 的 tone。
type Tone string

// 四种语气：普通信息、成功、警告、错误。
const (
	ToneInfo   Tone = "info"
	ToneOK     Tone = "ok"
	ToneWarn   Tone = "warn"
	ToneDanger Tone = "danger"
)

// toastTimeout 对齐 React 版的 4.2 秒自动消失。
const toastTimeout = 4200 * time.Millisecond

type queuedToast struct {
	title       string
	description string
	tone        Tone
}

// toastSeq 生成通知的唯一 ID（主线程内递增即可）。
var toastSeq int

// Notify 弹一条浮层通知。可在 view 任意位置调用；确认回调、后台收尾等
// 迟到上下文里调用同样安全（内部排队，下一帧展示）。
func Notify(title, description string, tone Tone) {
	if tone == "" {
		tone = ToneInfo
	}
	store.toastQueue = append(store.toastQueue, queuedToast{title, description, tone})
}

// flushToasts 把队列里的通知交给 mygo；同帧多次入队的通知会依次展示。
func (s *Store) flushToasts(c *mygo.Context) {
	for _, q := range s.toastQueue {
		toastSeq++
		c.AddToast(mygo.Toast{
			ID:          fmt.Sprintf("toast_%d", toastSeq),
			Title:       q.title,
			Description: q.description,
			Type:        string(q.tone),
			Timeout:     toastTimeout,
		})
	}
	s.toastQueue = s.toastQueue[:0]
}

// toastLook 是语气的图标与颜色（对齐 Toasts.tsx 的 toneIcon 与语气配色）。
func toastLook(c *mygo.Context, tone string) (string, mygo.Color, mygo.Color) {
	p := pal(c)
	switch Tone(tone) {
	case ToneOK:
		return "check-circle", p.OK, p.OKSoft
	case ToneWarn:
		return "alert", p.Warn, p.WarnSoft
	case ToneDanger:
		return "x-circle", p.Danger, p.DangerSoft
	default:
		return "info", p.Text2, p.BgHover
	}
}

// drawToastViewport 在右下角画通知层（.toast-layer：right 16 / bottom 42），
// 每帧调用一次；没有通知时 mygo 会为退出过渡保留视口。
func drawToastViewport(c *mygo.Context) {
	store.flushToasts(c)
	mygo.ToastViewportBase(c, func(viewport mygo.Element, toasts []mygo.Toast) {
		viewport.Padding(0, 16, 42, 0).Justify(mygo.End).AlignItems(mygo.End).Gap(7)
		for _, t := range toasts {
			iconName, color, soft := toastLook(c, t.Type)
			p := pal(c)
			toast := mygo.ToastBase(c, t)
			toast.Root.Row().AlignItems(mygo.Start).Gap(9).
				Padding(9, 11).Radius(9).
				Background(p.BgFloat).Border(1, p.BorderStrong).
				Shadow(p.ShadowMd.X, p.ShadowMd.Y, p.ShadowMd.Blur, p.ShadowMd.Spread, p.ShadowMd.Color)
			toast.Root.Children(func() {
				mygo.Box(c).Width(17).Height(17).Radius(5).Center().Background(soft).Children(func() {
					icons.Icon(c, iconName).Size(11, 11).TextColor(color)
				})
				mygo.Column(c).Gap(1).Children(func() {
					mygo.Text(c, t.Title)
					if t.Description != "" {
						mygo.Text(c, t.Description).FontSize(theme.Fonts.SM()).TextColor(p.Text3)
					}
				})
				toast.CloseButton().Label("关闭通知").Children(func() {
					icons.Icon(c, "x").Size(12, 12).TextColor(p.Text4)
				})
			})
		}
	})
}
