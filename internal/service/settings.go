// 本文件是设置相关的前端绑定：读写应用设置，并在保存时校验取值、
// 同步审批档位到策略引擎、切换活动工作区。设置保存在 SQLite 的
// 键值表中，读取失败时回退到默认设置。

package service

import (
	"fmt"

	"craftwork/internal/events"
	"craftwork/internal/model"
	"craftwork/internal/policy"
	"craftwork/internal/store"
	"craftwork/internal/tools"
)

// settingsKey 是应用整体设置在键值表中的键名。
const settingsKey = "app"

// Settings 是前端可读写的应用设置。
type Settings struct {
	// Theme 是界面主题：dark 或 light。
	Theme string `json:"theme"`
	// Mode 是默认对话模式：code 或 work。
	Mode string `json:"mode"`
	// PanelWidth 是右侧面板宽度（像素）。
	PanelWidth int `json:"panelWidth"`
	// ApprovalLevel 是审批档位：graded、strict 或 readonly。
	ApprovalLevel string `json:"approvalLevel"`
	// DefaultModel 是默认模型标识。
	DefaultModel string `json:"defaultModel"`
	// FallbackModel 是主模型失败时使用的备用模型，空表示关闭。
	FallbackModel string `json:"fallbackModel"`
	// ActiveWorkspace 是当前活动工作区标识。
	ActiveWorkspace string `json:"activeWorkspace"`
	// Language 是界面语言。
	Language string `json:"language"`
	// FontSize 是正文与编辑器字号。
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
}

// DefaultSettings 返回首次启动的默认设置。
func DefaultSettings() Settings {
	_, modelInfo := model.DefaultModel()
	return Settings{
		Theme:            "dark",
		Mode:             "code",
		PanelWidth:       440,
		ApprovalLevel:    string(policy.LevelGraded),
		DefaultModel:     modelInfo.ID,
		Language:         "zh-CN",
		FontSize:         13,
		CodeTheme:        "dark",
		TerminalFontSize: 12,
		AutoUpdate:       true,
		Notifications:    true,
		UpdateChannel:    "stable",
		Telemetry:        false,
	}
}

// SettingsService 读写设置，并把审批档位与工作区同步到策略引擎。
type SettingsService struct {
	// db 是设置的存储入口。
	db *store.DB
	// bus 推送设置变更事件。
	bus *events.Bus
	// engine 是策略引擎，保存设置时同步审批档位与工作区。
	engine *policy.Engine
	// registry 是工具注册表，切换工作区时同步根目录。
	registry *tools.Registry
}

// NewSettingsService 创建设置服务。
func NewSettingsService(db *store.DB, bus *events.Bus, engine *policy.Engine, registry *tools.Registry) *SettingsService {
	return &SettingsService{db: db, bus: bus, engine: engine, registry: registry}
}

// Get 返回当前设置。
func (s *SettingsService) Get() Settings {
	settings := DefaultSettings()
	_ = s.db.GetSetting(settingsKey, &settings)
	return settings
}

// Save 校验并保存设置，返回保存后的完整设置。
//
// 非法取值不会报错，而是回退到当前值或默认值，保证前端不会因为
// 一个坏字段就整体保存失败；涉及工作区与模型时才做真实校验。
func (s *SettingsService) Save(next Settings) (Settings, error) {
	current := s.Get()

	// 主题与模式只接受已知枚举，其余值保持原样。
	if next.Theme != "dark" && next.Theme != "light" {
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
		next.FontSize = 13
	}
	if next.TerminalFontSize < 10 || next.TerminalFontSize > 20 {
		next.TerminalFontSize = 12
	}
	if _, _, ok := model.FindModel(next.DefaultModel); !ok {
		_, modelInfo := model.DefaultModel()
		next.DefaultModel = modelInfo.ID
	}
	// 失败转移只接受真实提供商；空字符串表示关闭。
	if next.FallbackModel != "" {
		provider, _, ok := model.FindModel(next.FallbackModel)
		if !ok || provider.Kind == "mock" || next.FallbackModel == next.DefaultModel {
			next.FallbackModel = ""
		}
	}

	if next.ActiveWorkspace != "" {
		// 切换活动工作区时要同时更新路径、信任状态与工具注册表，
		// 否则策略判断会继续使用旧工作区。
		workspace, err := s.db.GetWorkspace(next.ActiveWorkspace)
		if err != nil {
			return current, fmt.Errorf("工作区不存在: %w", err)
		}
		s.engine.SetWorkspace(workspace.Path)
		s.engine.SetRestricted(workspace.Trust != "trusted")
		s.registry.SetWorkspace(workspace.Path)
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
