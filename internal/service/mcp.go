// 本文件是 MCP 服务器管理的前端绑定：保存服务器配置、启动与断开连接、
// 把服务器暴露的工具注册到工具注册表。配置保存在 SQLite 设置表中，
// 启动时按启用状态自动恢复连接，单个服务器失败不影响应用启动。

package service

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/yangshifu1024/craftwork/internal/id"
	"github.com/yangshifu1024/craftwork/internal/mcp"
	"github.com/yangshifu1024/craftwork/internal/store"
	"github.com/yangshifu1024/craftwork/internal/tools"
)

// mcpServersKey 是 MCP 服务器配置在设置表中的键名。
const mcpServersKey = "mcp.servers"

// MCPToolView 是 MCP 工具的展示结构。
type MCPToolView struct {
	// Name 是工具在服务器侧的名称。
	Name string `json:"name"`
	// Description 是工具用途说明。
	Description string `json:"description"`
}

// MCPServerView 是 MCP 服务器的展示结构。
type MCPServerView struct {
	// ID 是服务器标识。
	ID string `json:"id"`
	// Name 是服务器名称。
	Name string `json:"name"`
	// Command 是启动命令。
	Command string `json:"command"`
	// Args 是启动参数。
	Args []string `json:"args"`
	// Enabled 表示该服务器是否启用。
	Enabled bool `json:"enabled"`
	// Connected 表示当前是否已连接。
	Connected bool `json:"connected"`
	// Error 是最近一次连接失败的原因。
	Error string `json:"error,omitempty"`
	// Tools 是服务器暴露的工具列表。
	Tools []MCPToolView `json:"tools"`
}

// MCPService 管理本地 MCP 服务器的配置、连接与工具注册。
type MCPService struct {
	// db 是服务器配置的存储入口。
	db *store.DB
	// registry 是工具注册表，动态工具注册到这里。
	registry *tools.Registry

	// mu 保护缓存中的所有字段。
	mu sync.Mutex
	// configs 是已保存的服务器配置。
	configs []mcp.ServerConfig
	// clients 按服务器标识保存活动连接。
	clients map[string]*mcp.Client
	// serverTool 缓存每个服务器返回的工具信息。
	serverTool map[string][]mcp.ToolInfo
	// registered 记录每个服务器注册到工具注册表的工具名，断开时按名移除。
	registered map[string][]string
	// errors 保存每个服务器最近一次失败的原因。
	errors map[string]string
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
	// 复制一份配置再解锁，避免连接过程中长时间持锁。
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
//
// 连接失败或工具列表读取失败时，把服务器标记为停用并保存错误信息，
// 下次启动不会反复尝试连接同一台坏掉的服务器。成功时替换旧连接，
// 并把每个工具的适配器注册到工具注册表。
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

// disconnectLocked 关闭连接并从注册表移除该服务器的全部工具。
// 调用方必须已经持有 s.mu。
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

// viewLocked 组装服务器展示结构。调用方必须已经持有 s.mu。
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

// persistLocked 把当前服务器配置写回设置表。调用方必须已经持有 s.mu。
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
