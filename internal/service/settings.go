package service

import (
	"fmt"

	"craftwork/internal/events"
	"craftwork/internal/model"
	"craftwork/internal/policy"
	"craftwork/internal/store"
	"craftwork/internal/tools"
)

const settingsKey = "app"

// Settings 是前端可读写的应用设置。
type Settings struct {
	Theme            string `json:"theme"`
	Mode             string `json:"mode"`
	PanelWidth       int    `json:"panelWidth"`
	ApprovalLevel    string `json:"approvalLevel"`
	DefaultModel     string `json:"defaultModel"`
	FallbackModel    string `json:"fallbackModel"`
	ActiveWorkspace  string `json:"activeWorkspace"`
	Language         string `json:"language"`
	FontSize         int    `json:"fontSize"`
	CodeTheme        string `json:"codeTheme"`
	TerminalFontSize int    `json:"terminalFontSize"`
	AutoUpdate       bool   `json:"autoUpdate"`
	UpdateChannel    string `json:"updateChannel"`
	Telemetry        bool   `json:"telemetry"`
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
		UpdateChannel:    "stable",
		Telemetry:        false,
	}
}

// SettingsService 读写设置，并把审批档位与工作区同步到策略引擎。
type SettingsService struct {
	db       *store.DB
	bus      *events.Bus
	engine   *policy.Engine
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
func (s *SettingsService) Save(next Settings) (Settings, error) {
	current := s.Get()

	if next.Theme != "dark" && next.Theme != "light" {
		next.Theme = current.Theme
	}
	if next.Mode != "code" && next.Mode != "work" {
		next.Mode = current.Mode
	}
	switch policy.ParseLevel(next.ApprovalLevel) {
	case policy.LevelStrict:
		next.ApprovalLevel = string(policy.LevelStrict)
	case policy.LevelReadonly:
		next.ApprovalLevel = string(policy.LevelReadonly)
	default:
		next.ApprovalLevel = string(policy.LevelGraded)
	}
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
