// 数据面板：审计记录（决定过滤 + ui.Table）、用量统计（指标卡 + 按模型 /
// 按日图表）与备份列表（恢复走危险确认）。三个分区共用一个模态，切换
// 分区或点击刷新时重新读取对应数据。

package overlays

import (
	"fmt"
	"strings"

	mygo "github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/chart"

	cwstore "github.com/yangshifu1024/craftwork/internal/store"
	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// dataState 是数据面板的界面状态。
type dataState struct {
	// open 是面板是否打开；每次打开默认回到审计分区。
	open bool
	// tab 是当前分区。
	tab DataTab
	// loadedTab 记录上帧分区，用于检测切换时重新读取。
	loadedTab DataTab

	// auditFilter 是审计的决定过滤：all / auto / allow / deny。
	auditFilter string
	audit       []cwstore.AuditEntry
	usage       *cwstore.UsageSummary
	backups     []cwstore.Backup
}

// auditFilters 是审计过滤器的选项与展示文案。
var auditFilters = []struct{ id, label string }{
	{"all", "全部"},
	{"auto", "自动"},
	{"allow", "允许"},
	{"deny", "拒绝"},
}

// openData 打开数据面板；tab 为空时回到审计分区，并读取对应数据。
func (s *Store) openData(tab DataTab) {
	st := &s.data
	st.open = true
	if tab == "" {
		tab = TabAudit
	}
	st.tab = tab
	s.loadData(st)
}

// loadData 读取当前分区的数据（SQLite 本地查询，主线程安全）。
func (s *Store) loadData(st *dataState) {
	if s.be == nil || s.be.Audit == nil {
		return
	}
	switch st.tab {
	case TabAudit:
		if rows, err := s.be.Audit.List(200); err == nil {
			st.audit = rows
		}
	case TabUsage:
		if usage, err := s.be.Audit.Usage(); err == nil {
			st.usage = usage
		}
	case TabBackup:
		if rows, err := s.be.Audit.Backups(50); err == nil {
			st.backups = rows
		}
	}
	st.loadedTab = st.tab
}

// drawData 画数据面板模态。
func (s *Store) drawData(c *mygo.Context) {
	st := &s.data
	if !st.open {
		return
	}
	p := pal(c)
	w, h := c.Size()

	// 分区切换时重新读取。
	if st.loadedTab != st.tab {
		s.loadData(st)
	}

	panel := mygo.DialogBase(c, &st.open, func(backdrop, panel mygo.Element) {
		styleBackdrop(backdrop)
		stylePanel(c, panel)
		panel.Width(860).MaxWidth(w - 80).Height(620).MaxHeight(h - 90).
			Radius(14)
		panel.Gap(0).Children(func() {
			// 头部：标题随分区变化，刷新与关闭。
			title := map[DataTab]string{TabAudit: "审计记录", TabUsage: "用量统计", TabBackup: "备份与恢复"}[st.tab]
			mygo.Row(c).Height(46).Padding(0, theme.Space7, 0, theme.Space7).
				AlignItems(mygo.Center).Gap(theme.Space5).
				BorderWidth(0, 0, 1, 0).BorderColor(p.Border).Children(func() {
				icons.Icon(c, "database").Size(16, 16).TextColor(p.Text2)
				mygo.Text(c, title).FontSize(theme.Fonts.XL()).Bold()
				mygo.Spacer(c)
				if iconBtn(c, "refresh", "刷新").Clicked() {
					s.loadData(st)
				}
				if iconBtn(c, "x", "关闭数据面板").Clicked() {
					st.open = false
				}
			})
			// 分区行。
			mygo.Row(c).Height(36).Gap(theme.Space2).Padding(theme.Space4, theme.Space7).
				BorderWidth(0, 0, 1, 0).BorderColor(p.Border).Children(func() {
				for _, t := range []struct {
					id   DataTab
					icon string
				}{{TabAudit, "shield-check"}, {TabUsage, "zap"}, {TabBackup, "archive"}} {
					label := map[DataTab]string{TabAudit: "审计", TabUsage: "用量", TabBackup: "备份"}[t.id]
					active := st.tab == t.id
					btn := mygo.ButtonBase(c)
					btn.Height(theme.HeightBtnSmall).Padding(0, theme.Space6).Radius(theme.RadiusCtl).
						AlignItems(mygo.Center).Gap(theme.Space3)
					if active {
						btn.Background(p.BgActive)
					}
					btn.Children(func() {
						icons.Icon(c, t.icon).Size(13, 13).TextColor(p.Text2)
						mygo.Text(c, label).FontSize(theme.Fonts.SM())
					})
					if btn.Clicked() && !active {
						st.tab = t.id
					}
				}
				mygo.Spacer(c)
			})
			// 内容区。
			body := mygo.Scroll(c).Grow(1).Padding(theme.Space6, theme.Space7)
			body.Children(func() {
				switch st.tab {
				case TabAudit:
					s.tabAudit(c, st)
				case TabUsage:
					s.tabUsage(c, st)
				case TabBackup:
					s.tabBackup(c, st)
				}
			})
		})
	})
	if panel.Dismissed() {
		st.open = false
	}
}

// ---------------------------------------------------------------------------
// 审计
// ---------------------------------------------------------------------------

// tabAudit 画审计分区：决定过滤 + 记录表格。
func (s *Store) tabAudit(c *mygo.Context, st *dataState) {
	p := pal(c)
	if st.auditFilter == "" {
		st.auditFilter = "all"
	}
	rows := make([]cwstore.AuditEntry, 0, len(st.audit))
	for _, e := range st.audit {
		if st.auditFilter == "all" || e.Decision == st.auditFilter {
			rows = append(rows, e)
		}
	}

	mygo.Column(c).Gap(theme.Space5).Children(func() {
		mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space2).Children(func() {
			for _, f := range auditFilters {
				active := st.auditFilter == f.id
				btn := mygo.ButtonBase(c)
				btn.Height(theme.HeightBtnSmall).Padding(0, theme.Space6).Radius(theme.RadiusCtl).
					Center()
				if active {
					btn.Background(p.AccentSoft).BorderWidth(1).BorderColor(p.AccentLine)
				} else {
					btn.Background(p.BgElevated).BorderWidth(1).BorderColor(p.Border)
				}
				btn.Children(func() {
					mygo.Text(c, f.label).FontSize(theme.Fonts.SM())
				})
				if btn.Clicked() && !active {
					st.auditFilter = f.id
				}
			}
			mygo.Spacer(c)
			mutedText(c, fmt.Sprintf("共 %d 条记录", len(rows)))
		})

		if len(rows) == 0 {
			emptyState(c, "shield-check", "还没有审计记录", "")
			return
		}
		cols := []mygo.TableColumn{
			{Title: "时间", Width: 110},
			{Title: "工具", Width: 150},
			{Title: "风险", Width: 56, Align: mygo.Center},
			{Title: "决定", Width: 64},
			{Title: "范围", Width: 120},
			{Title: "详情"},
		}
		mygo.Table(c, nil, cols, len(rows), func(row, col int) {
			e := rows[row]
			p := pal(c)
			switch col {
			case 0:
				mygo.Text(c, formatTime(e.TS)).TextColor(p.Text3)
			case 1:
				mygo.Text(c, e.Tool).SingleLine()
			case 2:
				color := p.Risk(policyLevelRisk(e.Risk))
				mygo.Box(c).FillHeight().Center().Children(func() {
					badge(c, e.Risk, color, color.Alpha(0.13))
				})
			case 3:
				label := "自动"
				switch e.Decision {
				case "allow":
					label = "允许"
				case "deny":
					label = "拒绝"
				}
				color := p.Text3
				switch e.Decision {
				case "allow":
					color = p.OK
				case "deny":
					color = p.Danger
				}
				mygo.Text(c, label).TextColor(color)
			case 4:
				v := e.Scope
				if v == "" {
					v = "—"
				}
				mygo.Text(c, v).TextColor(p.Text2).SingleLine()
			case 5:
				v := e.Detail
				if v == "" {
					v = "—"
				}
				mygo.Text(c, v).TextColor(p.Text2).SingleLine()
			}
		}).BorderWidth(1).BorderColor(p.BorderSoft).Radius(theme.RadiusRow).Height(auditTableHeight(len(rows)))
	})
}

// ---------------------------------------------------------------------------
// 用量
// ---------------------------------------------------------------------------

// auditTableHeight 估算审计表高度：表头 + 每行约 32，最多到面板可视高度。
func auditTableHeight(n int) float32 {
	h := float32(36 + n*32)
	if h > 430 {
		h = 430
	}
	return h
}

// tabUsage 画用量分区：四张指标卡 + 按模型 / 按日两张柱状图。
func (s *Store) tabUsage(c *mygo.Context, st *dataState) {
	p := pal(c)
	usage := st.usage
	if usage == nil {
		emptyState(c, "zap", "还没有用量数据", "")
		return
	}
	mygo.Column(c).Fill().Gap(theme.Space5).Children(func() {
		// 指标卡行。
		mygo.Row(c).Fill().Gap(theme.Space5).Children(func() {
			usageCard(c, "今天", &usage.Today)
			usageCard(c, "近 7 天", &usage.Week)
			usageCard(c, "近 30 天", &usage.Month)
			card := mygo.Column(c).Grow(1).Gap(theme.Space2).Padding(theme.Space6).
				Radius(theme.RadiusCard).Background(p.BgElevated).BorderWidth(1).BorderColor(p.BorderSoft)
			card.Children(func() {
				mygo.Text(c, "模型数量").FontSize(theme.Fonts.SM()).TextColor(p.Text3)
				mygo.Textf(c, "%d", len(usage.ByModel)).FontSize(theme.Fonts.XL()).Bold()
				mygo.Text(c, "已产生用量的模型").FontSize(theme.Fonts.SM()).TextColor(p.Text4)
			})
		})
		mutedText(c, "费用按 models.dev 目录价格估算，单位为美元，不是提供商账单。")

		// 按模型。
		mygo.Text(c, "按模型").FontSize(theme.Fonts.LG()).Bold()
		if len(usage.ByModel) == 0 {
			mutedText(c, "还没有用量记录。")
		} else {
			labels := make([]string, 0, len(usage.ByModel))
			values := make([]float64, 0, len(usage.ByModel))
			for _, m := range usage.ByModel {
				labels = append(labels, m.ModelID)
				values = append(values, float64(m.InputTokens+m.OutputTokens))
			}
			s.usageChart(c, labels, values, 200, p.Accent)
		}

		// 每日趋势。
		mygo.Text(c, "每日趋势").FontSize(theme.Fonts.LG()).Bold()
		if len(usage.ByDay) == 0 {
			mutedText(c, "还没有用量记录。")
		} else {
			labels := make([]string, 0, len(usage.ByDay))
			values := make([]float64, 0, len(usage.ByDay))
			for _, d := range usage.ByDay {
				labels = append(labels, d.Day[5:]) // MM-DD
				values = append(values, float64(d.InputTokens+d.OutputTokens))
			}
			s.usageChart(c, labels, values, 180, p.Cyan)
		}
	})
}

// usageCard 是一张用量概览卡。
func usageCard(c *mygo.Context, label string, totals *cwstore.UsageTotals) {
	p := pal(c)
	mygo.Column(c).Grow(1).Gap(theme.Space2).Padding(theme.Space6).
		Radius(theme.RadiusCard).Background(p.BgElevated).BorderWidth(1).BorderColor(p.BorderSoft).Children(func() {
		mygo.Text(c, label).FontSize(theme.Fonts.SM()).TextColor(p.Text3)
		mygo.Row(c).AlignItems(mygo.Center).Gap(theme.Space2).Children(func() {
			mygo.Text(c, formatTokens(totals.InputTokens+totals.OutputTokens)).FontSize(theme.Fonts.XL()).Bold()
			mygo.Text(c, "tokens").FontSize(theme.Fonts.XS()).TextColor(p.Text4)
		})
		mygo.Textf(c, "%d 次 · %s", totals.Runs, formatCost(totals.CostMicros)).
			FontSize(theme.Fonts.SM()).TextColor(p.Text4)
	})
}

// usageChart 用 ui/chart 画一张柱状图：x 为类目带，y 为线性值轴。
func (s *Store) usageChart(c *mygo.Context, labels []string, values []float64, height float32, color mygo.Color) {
	x := chart.NewBand(labels...).Padding(0.3, 0.15)
	_, hi := chart.Extent(values...)
	y := chart.NewLinear(0, hi).Nice(4)
	ticks := y.Ticks(4)
	plot := chart.Plot(c, chart.Axes{Left: ticks, Bottom: x.Ticks()}, func(area mygo.Element) {
		for i, v := range values {
			chart.Rect(c, x.Band(i), y.Span(0, v)).Radius(3).Background(color)
		}
	}).Label("用量柱状图").Height(height).Grow(1)
	_ = plot
}

// ---------------------------------------------------------------------------
// 备份
// ---------------------------------------------------------------------------

// tabBackup 画备份分区：备份列表 + 恢复（危险确认）。
func (s *Store) tabBackup(c *mygo.Context, st *dataState) {
	p := pal(c)
	if len(st.backups) == 0 {
		emptyState(c, "archive", "还没有备份", "写入文件前会自动备份到数据目录 backups/ 下。")
		return
	}
	for i := range st.backups {
		b := &st.backups[i]
		mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space5).
			Padding(theme.Space5, theme.Space6).Margin(0, 0, theme.Space2, 0).
			Radius(theme.RadiusCard).Background(p.BgElevated).BorderWidth(1).BorderColor(p.BorderSoft).Children(func() {
			icons.Icon(c, "file").Size(15, 15).TextColor(p.Text3)
			mygo.Column(c).Grow(1).Gap(1).Children(func() {
				mygo.Text(c, b.Path).SingleLine()
				mygo.Textf(c, "%s · %s · %s", formatTime(b.CreatedAt), formatBytes(b.Size), clipHash(b.Hash)).
					FontSize(theme.Fonts.SM()).TextColor(p.Text4).SingleLine()
			})
			if smallGhostBtn(c, "恢复").Clicked() {
				s.confirmRestore(c, b)
			}
		})
	}
}

// confirmRestore 危险确认后用备份覆盖目标文件。
func (s *Store) confirmRestore(c *mygo.Context, b *cwstore.Backup) {
	Confirm(c, ConfirmOpts{
		Title:       "恢复这个备份？",
		Description: "目标文件会被备份内容覆盖。恢复前请确认当前文件没有需要保留的修改。",
		Detail:      b.Path,
		ConfirmText: "恢复",
		Danger:      true,
	}, func(ok bool) {
		if !ok {
			return
		}
		if err := s.be.Audit.Restore(b.ID); err != nil {
			Notify("恢复失败", err.Error(), ToneDanger)
			return
		}
		Notify("已恢复备份", b.Path, ToneOK)
	})
}

// clipHash 取哈希前 12 位。
func clipHash(hash string) string {
	if len(hash) > 12 {
		return hash[:12]
	}
	return strings.TrimSuffix(hash, "")
}
