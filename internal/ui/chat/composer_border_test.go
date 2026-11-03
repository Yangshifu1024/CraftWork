// composer_border_test 锁定 composer 描边两态的取色（失焦传播在 Tester
// 里无法稳定模拟——AutoFocus 每帧拉回焦点，实机走查确认）。
package chat

import (
	"testing"

	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

func TestComposerBorderColors(t *testing.T) {
	pal := theme.Apply(nil)
	if got := composerBorder(pal, true); got != pal.ComposerBorderFocus {
		t.Error("聚焦态描边应取 ComposerBorderFocus（zai 聚焦灰加深，非模式色）")
	}
	if got := composerBorder(pal, false); got != pal.ComposerBorder {
		t.Error("失焦描边应取常态描边")
	}
}
