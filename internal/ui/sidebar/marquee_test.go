package sidebar

// 13 号票：会话标题跑马灯的相位纯函数测试。view 侧只做接线（hover 判定、
// 测量与 Loop 驱动），滚动行程的全部形状都在 marqueePhase 里，可独立断言。

import (
	"math"
	"testing"
	"time"
)

func TestMarqueePhase(t *testing.T) {
	// 相位形状：头 20% 停在起点、30% 滚到尽头、20% 停在尽头、尾 30% 滚回。
	cases := []struct {
		cycle float32
		want  float32
	}{
		{0, 0},
		{0.1, 0},
		{0.19, 0},
		{0.35, 0.5}, // 滚出段中点
		{0.5, 1},
		{0.65, 1},
		{0.69, 1},
		{0.85, 0.5}, // 滚回段中点
		{0.99, 0},
	}
	for _, tc := range cases {
		got := marqueePhase(tc.cycle)
		if math.Abs(float64(got-tc.want)) > 0.02 {
			t.Errorf("marqueePhase(%.2f) = %.3f, want %.3f", tc.cycle, got, tc.want)
		}
	}
	// cycle 越界按取模处理（Loop 返回值理论在 0..1，防御负值与 ≥1）。
	for _, cycle := range []float32{-0.3, 1.0, 1.7} {
		if p := marqueePhase(cycle); p < 0 || p > 1 {
			t.Errorf("marqueePhase(%.2f) = %.3f, 应在 0..1", cycle, p)
		}
	}
}

func TestMarqueeOffset(t *testing.T) {
	// 偏移 = -溢出量 × 相位：行程不超过溢出量，方向恒向左。
	if got := marqueeOffset(0.35, 100); math.Abs(float64(got+50)) > 2 {
		t.Errorf("marqueeOffset(0.35, 100) = %.1f, want -50", got)
	}
	if got := marqueeOffset(0.65, 100); math.Abs(float64(got+100)) > 2 {
		t.Errorf("marqueeOffset(0.65, 100) = %.1f, want -100", got)
	}
	// 不超宽（溢出 ≤ 0）恒不滚动。
	for _, cycle := range []float32{0, 0.35, 0.65, 0.9} {
		if got := marqueeOffset(cycle, 0); got != 0 {
			t.Errorf("marqueeOffset(%.2f, 0) = %.1f, want 0", cycle, got)
		}
		if got := marqueeOffset(cycle, -10); got != 0 {
			t.Errorf("marqueeOffset(%.2f, -10) = %.1f, want 0", cycle, got)
		}
	}
}

func TestMarqueePeriod(t *testing.T) {
	// 周期随溢出加长：4s 底 + 每 40px 加 2s；不超宽时也有最短周期
	//（不触发滚动，周期值不被用到，但函数仍给正数便于防御）。
	if got := marqueePeriod(0); got != 4*time.Second {
		t.Errorf("marqueePeriod(0) = %v, want 4s", got)
	}
	if got := marqueePeriod(40); got != 6*time.Second {
		t.Errorf("marqueePeriod(40) = %v, want 6s", got)
	}
	if got := marqueePeriod(200); got != 14*time.Second {
		t.Errorf("marqueePeriod(200) = %v, want 14s", got)
	}
}
