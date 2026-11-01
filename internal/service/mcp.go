package service

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"sync"
	"time"

	"craftwork/internal/id"
	"craftwork/internal/mcp"
	"craftwork/internal/store"
	"craftwork/internal/tools"
)

const mcpServersKey = "mcp.servers"

// MCPToolView 是 MCP 工具的展示结构。
type MCPToolView struct {
	Name        string `json:"name"`
	Description string `json:"description"`
}

// MCPServerView 是 MCP 服务器的展示结构。
type MCPServerView struct {
	ID        string        `json:"id"`
	Name      string        `json:"name"`
	Command   string        `json:"command"`
	Args      []string      `json:"args"`
	Enabled   bool          `json:"enabled"`
	Connected bool          `json:"connected"`
	Error     string        `json:"error,omitempty"`
	Tools     []MCPToolView `json:"tools"`
}

// MCPService 管理本地 MCP 服务器的配置、连接与工具注册。
type MCPService struct {
	db       *store.DB
	registry *tools.Registry

	mu         sync.Mutex
	configs    []mcp.ServerConfig
	clients    map[string]*mcp.Client
	serverTool map[string][]mcp.ToolInfo
	registered map[string][]string
	errors     map[string]string
}

// NewMCPService 创建 MCP 服务并读取已保存的服务器配置。
func NewMCPService(db *store.DB, registry *tools.Registry) *MCPService {
	service := &MCPService{
		db:         db,
		registry:   registry,
		clients:    map[string]*mcp.Client{},
		serverTool: map[string][]mcp.ToolInfo{},
		registered: map[string][]string{},
		errors:     map[string]string{},
	}
	var saved []mcp.ServerConfig
	if err := db.GetSetting(mcpServersKey, &saved); err == nil {
		service.configs = saved
	}
	return service
}

// Restore 在启动时连接启用的服务器。失败只记录在视图里，不阻塞窗口。
func (s *MCPService) Restore() {
	s.mu.Lock()
	configs := append([]mcp.ServerConfig(nil), s.configs...)
	s.mu.Unlock()
	for _, config := range configs {
		if !config.Enabled {
			continue
		}
		config := config
		go func() {
			if _, err := s.Connect(config.ID); err != nil {
				// 错误已写入视图，启动日志由调用方决定是否展示。
				_ = err
			}
		}()
	}
}

// List 返回所有 MCP 服务器与工具状态。
func (s *MCPService) List() []MCPServerView {
	s.mu.Lock()
	defer s.mu.Unlock()
	views := make([]MCPServerView, 0, len(s.configs))
	for _, config := range s.configs {
		views = append(views, s.viewLocked(config))
	}
	return views
}

// Add 添加并尝试连接一个 MCP 服务器。
func (s *MCPService) Add(name, commandLine string) (MCPServerView, error) {
	name = strings.TrimSpace(name)
	if name == "" {
		return MCPServerView{}, errors.New("服务器名称不能为空")
	}
	command, arguments, err := splitCommandLine(commandLine)
	if err != nil {
		return MCPServerView{}, err
	}
	config := mcp.ServerConfig{
		ID:      id.New("mcp"),
		Name:    name,
		Command: command,
		Args:    arguments,
		Enabled: true,
	}
	s.mu.Lock()
	s.configs = append(s.configs, config)
	err = s.persistLocked()
	s.mu.Unlock()
	if err != nil {
		return MCPServerView{}, err
	}
	view, _ := s.Connect(config.ID)
	return view, nil
}

// Remove 断开并删除一个服务器。
func (s *MCPService) Remove(serverID string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.disconnectLocked(serverID)
	filtered := s.configs[:0]
	for _, config := range s.configs {
		if config.ID != serverID {
			filtered = append(filtered, config)
		}
	}
	s.configs = filtered
	delete(s.errors, serverID)
	return s.persistLocked()
}

// Connect 启动服务器、握手并注册工具。
func (s *MCPService) Connect(serverID string) (MCPServerView, error) {
	s.mu.Lock()
	defer s.mu.Unlock()

	index := -1
	for i, config := range s.configs {
		if config.ID == serverID {
			index = i
			break
		}
	}
	if index < 0 {
		return MCPServerView{}, errors.New("未知的 MCP 服务器")
	}
	config := s.configs[index]
	s.disconnectLocked(serverID)

	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	client, err := mcp.Start(ctx, config)
	if err == nil {
		err = client.Initialize(ctx)
	}
	if err != nil {
		s.errors[serverID] = err.Error()
		s.configs[index].Enabled = false
		_ = s.persistLocked()
		return s.viewLocked(s.configs[index]), err
	}
	list, listErr := client.ListTools(ctx)
	if listErr != nil {
		_ = client.Close()
		s.errors[serverID] = listErr.Error()
		s.configs[index].Enabled = false
		_ = s.persistLocked()
		return s.viewLocked(s.configs[index]), listErr
	}

	s.clients[serverID] = client
	s.serverTool[serverID] = list
	names := make([]string, 0, len(list))
	for _, info := range list {
		tool := mcp.NewDynamicTool(client, config, info)
		s.registry.Register(tool)
		names = append(names, tool.Name())
	}
	s.registered[serverID] = names
	s.configs[index].Enabled = true
	delete(s.errors, serverID)
	if err := s.persistLocked(); err != nil {
		return s.viewLocked(s.configs[index]), err
	}
	return s.viewLocked(s.configs[index]), nil
}

// Disconnect 断开服务器并移除其工具。
func (s *MCPService) Disconnect(serverID string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.disconnectLocked(serverID)
	for index := range s.configs {
		if s.configs[index].ID == serverID {
			s.configs[index].Enabled = false
		}
	}
	return s.persistLocked()
}

func (s *MCPService) disconnectLocked(serverID string) {
	if client := s.clients[serverID]; client != nil {
		_ = client.Close()
		delete(s.clients, serverID)
	}
	for _, name := range s.registered[serverID] {
		s.registry.Unregister(name)
	}
	delete(s.registered, serverID)
	delete(s.serverTool, serverID)
	delete(s.errors, serverID)
}

func (s *MCPService) viewLocked(config mcp.ServerConfig) MCPServerView {
	view := MCPServerView{
		ID:        config.ID,
		Name:      config.Name,
		Command:   config.Command,
		Args:      append([]string(nil), config.Args...),
		Enabled:   config.Enabled,
		Connected: s.clients[config.ID] != nil,
		Error:     s.errors[config.ID],
		Tools:     []MCPToolView{},
	}
	for _, info := range s.serverTool[config.ID] {
		view.Tools = append(view.Tools, MCPToolView{Name: info.Name, Description: info.Description})
	}
	return view
}

func (s *MCPService) persistLocked() error {
	return s.db.SetSetting(mcpServersKey, s.configs)
}

// splitCommandLine 按空白拆分命令，支持单引号、双引号与反斜杠转义。
func splitCommandLine(line string) (string, []string, error) {
	var fields []string
	var current strings.Builder
	quote := rune(0)
	escaped := false
	flush := func() {
		if current.Len() > 0 {
			fields = append(fields, current.String())
			current.Reset()
		}
	}
	for _, char := range line {
		if escaped {
			current.WriteRune(char)
			escaped = false
			continue
		}
		switch {
		case char == '\\':
			escaped = true
		case quote != 0:
			if char == quote {
				quote = 0
			} else {
				current.WriteRune(char)
			}
		case char == '\'' || char == '"':
			quote = char
		case char == ' ' || char == '\t' || char == '\n':
			flush()
		default:
			current.WriteRune(char)
		}
	}
	if quote != 0 {
		return "", nil, fmt.Errorf("命令中的引号没有闭合")
	}
	flush()
	if len(fields) == 0 {
		return "", nil, errors.New("命令不能为空")
	}
	return fields[0], fields[1:], nil
}
