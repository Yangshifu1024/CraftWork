// 本文件是设置相关的前端绑定：读写应用设置，并在保存时校验取值、
// 同步审批档位到策略引擎、切换活动工作区。设置保存在 SQLite 的
// 键值表中，读取失败时回退到默认设置。

package service

import (
	"fmt"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/model"
	"github.com/yangshifu1024/craftwork/internal/policy"
	"github.com/yangshifu1024/craftwork/internal/store"
)

// settingsKey 是应用整体设置在键值表中的键名。
const settingsKey = "app"

// Settings 是前端可读写的应用设置。
type Settings struct {
	// Theme 是界面主题：system（跟随系统）、dark 或 light。
	Theme string `json:"theme"`
	// Mode 是默认对话模式：code 或 work。
	Mode string `json:"mode"`
	// PanelWidth 是右侧面板宽度（像素）。
	PanelWidth int `json:"panelWidth"`
	// ApprovalLevel 是审批档位：graded、strict、readonly 或 free。
	ApprovalLevel string `json:"approvalLevel"`
	// DefaultModel 是默认模型标识。
	DefaultModel string `json:"defaultModel"`
	// FallbackModel 是主模型失败时使用的备用模型，空表示关闭。
	FallbackModel string `json:"fallbackModel"`
	// ActiveWorkspace 是当前活动工作区标识。
	ActiveWorkspace string `json:"activeWorkspace"`
	// Language 是界面语言。
	Language string `json:"language"`
	// FontSize 是界面基准字号，驱动前端 --ui-font-size 缩放（styles/base.css 的 --text-ui-* 档位）。
	FontSize int `json:"fontSize"`
	// CodeTheme 是代码高亮主题。
	CodeTheme string `json:"codeTheme"`
	// TerminalFontSize 是终端字号。
	TerminalFontSize int `json:"terminalFontSize"`
	// AutoUpdate 表示是否自动检查更新。
	AutoUpdate bool `json:"autoUpdate"`
	// Notifications 表示是否启用系统通知。
	Notifications bool `json:"notifications"`
	// UpdateChannel 是更新通道：stable 等。
	UpdateChannel string `json:"updateChannel"`
	// Telemetry 表示是否允许发送匿名用量统计。
	Telemetry bool `json:"telemetry"`
	// MaxIterations 是单次运行最大模型往返次数，超过后停止本次运行；0 表示不限制。
	MaxIterations int `json:"maxIterations"`
}

// clampMaxIterations 校正单次运行最大往返数：0 合法（不限制），
// 负数与超过 1000 的值按脏数据回退默认 300。
func clampMaxIterations(v int) int {
	if v < 0 || v > 1000 {
		return 300
	}
	return v
}

// DefaultSettings 返回首次启动的默认设置。
//
// 首次启动没有任何提供商，因此默认模型为空；用户添加提供商并选择模型后
// 才会写入具体值。
func DefaultSettings() Settings {
	return Settings{
		Theme:            "system",
		Mode:             "code",
		PanelWidth:       320,
		ApprovalLevel:    string(policy.LevelGraded),
		DefaultModel:     "",
		Language:         "zh-CN",
		FontSize:         15,
		CodeTheme:        "dark",
		TerminalFontSize: 12,
		AutoUpdate:       true,
		Notifications:    true,
		UpdateChannel:    "stable",
		Telemetry:        false,
		MaxIterations:    300,
	}
}

// SettingsService 读写设置，并把审批档位同步到策略引擎。
type SettingsService struct {
	// db 是设置的存储入口。
	db *store.DB
	// bus 推送设置变更事件。
	bus *events.Bus
	// engine 是策略引擎，保存设置时同步审批档位。
	engine *policy.Engine
}

// NewSettingsService 创建设置服务。
func NewSettingsService(db *store.DB, bus *events.Bus, engine *policy.Engine) *SettingsService {
	return &SettingsService{db: db, bus: bus, engine: engine}
}

// Get 返回当前设置。
//
// 存储里的模型可能是旧的原始 ID；读取时统一转换成“提供商/模型”引用。
// 指向已删除提供商的模型会返回空，而不是回退到别的模型。
func (s *SettingsService) Get() Settings {
	settings := DefaultSettings()
	_ = s.db.GetSetting(settingsKey, &settings)
	normalizeModelSettings(&settings)
	return settings
}

// normalizeModelSettings 把默认模型与失败转移模型转换成规范引用。
//
// 返回是否发生了修改。无法解析的模型会被清空，避免界面继续展示一个
// 已经不可用的选择。
func normalizeModelSettings(settings *Settings) bool {
	changed := false
	if settings.DefaultModel != "" {
		if _, info, ok := model.Resolve(settings.DefaultModel); ok {
			if ref := info.Ref(); ref != settings.DefaultModel {
				settings.DefaultModel = ref
				changed = true
			}
		} else {
			settings.DefaultModel = ""
			changed = true
		}
	}
	if settings.FallbackModel != "" {
		if settings.FallbackModel == settings.DefaultModel {
			settings.FallbackModel = ""
			changed = true
		} else if _, info, ok := model.Resolve(settings.FallbackModel); ok {
			if ref := info.Ref(); ref != settings.FallbackModel {
				settings.FallbackModel = ref
				changed = true
			}
		} else {
			settings.FallbackModel = ""
			changed = true
		}
	}
	return changed
}

// Save 校验并保存设置，返回保存后的完整设置。
//
// 非法取值不会报错，而是回退到当前值或默认值，保证前端不会因为
// 一个坏字段就整体保存失败；涉及工作区与模型时才做真实校验。
func (s *SettingsService) Save(next Settings) (Settings, error) {
	current := s.Get()

	// 主题与模式只接受已知枚举，其余值保持原样。
	if next.Theme != "system" && next.Theme != "dark" && next.Theme != "light" {
		next.Theme = current.Theme
	}
	if next.Mode != "code" && next.Mode != "work" {
		next.Mode = current.Mode
	}
	// 审批档位交给策略包解析，未知值回退到分级。
	switch policy.ParseLevel(next.ApprovalLevel) {
	case policy.LevelStrict:
		next.ApprovalLevel = string(policy.LevelStrict)
	case policy.LevelReadonly:
		next.ApprovalLevel = string(policy.LevelReadonly)
	case policy.LevelFree:
		next.ApprovalLevel = string(policy.LevelFree)
	default:
		next.ApprovalLevel = string(policy.LevelGraded)
	}
	// 面板宽度与字号限制在可用范围内，超出时使用安全默认值。
	if next.PanelWidth < 320 {
		next.PanelWidth = 320
	}
	if next.PanelWidth > 760 {
		next.PanelWidth = 760
	}
	if next.FontSize < 11 || next.FontSize > 18 {
		next.FontSize = 15
	}
	if next.TerminalFontSize < 10 || next.TerminalFontSize > 20 {
		next.TerminalFontSize = 12
	}
	// 往返上限允许 0（不限制），其余限制在 1..1000；非法值回退默认。
	next.MaxIterations = clampMaxIterations(next.MaxIterations)
	// 默认模型允许为空；非空时必须是当前用户目录中的模型。
	// 失效模型不静默回退，而是清空，让用户重新选择。
	normalizeModelSettings(&next)

	if next.ActiveWorkspace != "" {
		// 校验活动工作区存在并刷新最近打开时间；策略状态不在这里同步——
		// 运行环境由每个会话在运行开始时从自己绑定的工作区解析。
		workspace, err := s.db.GetWorkspace(next.ActiveWorkspace)
		if err != nil {
			return current, fmt.Errorf("工作区不存在: %w", err)
		}
		workspace.LastOpenedAt = store.NowMillis()
		if err := s.db.UpsertWorkspace(workspace); err != nil {
			return current, err
		}
	}

	s.engine.SetLevel(policy.ParseLevel(next.ApprovalLevel))
	if err := s.db.SetSetting(settingsKey, next); err != nil {
		return current, err
	}
	s.bus.Publish(events.TypeSettingsUpdated, "", "", next)
	return next, nil
}
