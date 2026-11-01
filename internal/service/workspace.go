package service

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"

	"craftwork/internal/editor"
	"craftwork/internal/events"
	"craftwork/internal/id"
	"craftwork/internal/platform"
	"craftwork/internal/policy"
	"craftwork/internal/store"
	"craftwork/internal/tools"

	"github.com/wailsapp/wails/v3/pkg/application"
)

// WorkspaceView 是工作区的展示结构。
type WorkspaceView struct {
	ID           string `json:"id"`
	Name         string `json:"name"`
	Path         string `json:"path"`
	Trust        string `json:"trust"`
	CreatedAt    int64  `json:"createdAt"`
	LastOpenedAt int64  `json:"lastOpenedAt"`
}

// FileNode 是文件树中的一个节点（单层，按需加载）。
type FileNode struct {
	Name     string `json:"name"`
	Path     string `json:"path"`
	Type     string `json:"type"` // dir / file
	Size     int64  `json:"size"`
	Modified int64  `json:"modified"`
}

// FileContent 是读取文件的结果。
type FileContent struct {
	Path      string `json:"path"`
	Content   string `json:"content"`
	Size      int64  `json:"size"`
	Binary    bool   `json:"binary"`
	Language  string `json:"language"`
	Lines     int    `json:"lines"`
	Truncated bool   `json:"truncated"`
	Hash      string `json:"hash"`
	Modified  int64  `json:"modified"`
}

// WorkspaceService 管理工作区、文件树、文件读写与差异计算。
type WorkspaceService struct {
	db        *store.DB
	bus       *events.Bus
	engine    *policy.Engine
	registry  *tools.Registry
	backupDir string
}

// NewWorkspaceService 创建工作区服务。
func NewWorkspaceService(db *store.DB, bus *events.Bus, engine *policy.Engine,
	registry *tools.Registry, backupDir string) *WorkspaceService {
	return &WorkspaceService{db: db, bus: bus, engine: engine, registry: registry, backupDir: backupDir}
}

// List 列出所有工作区。
func (s *WorkspaceService) List() ([]WorkspaceView, error) {
	workspaces, err := s.db.ListWorkspaces()
	if err != nil {
		return nil, err
	}
	result := make([]WorkspaceView, 0, len(workspaces))
	for _, workspace := range workspaces {
		result = append(result, toWorkspaceView(workspace))
	}
	return result, nil
}

// PickFolder 打开系统文件夹选择对话框。
func (s *WorkspaceService) PickFolder() (string, error) {
	current := application.Get()
	if current == nil {
		return "", errors.New("应用尚未初始化")
	}
	return current.Dialog.OpenFile().
		SetTitle("选择工作区").
		CanChooseDirectories(true).
		CanChooseFiles(false).
		CanCreateDirectories(true).
		PromptForSingleSelection()
}

// Add 添加工作区并设为当前活动工作区。
func (s *WorkspaceService) Add(path, trust string) (WorkspaceView, error) {
	path = strings.TrimSpace(path)
	if path == "" {
		return WorkspaceView{}, errors.New("路径不能为空")
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return WorkspaceView{}, fmt.Errorf("解析路径: %w", err)
	}
	abs, err = filepath.EvalSymlinks(abs)
	if err != nil {
		return WorkspaceView{}, fmt.Errorf("解析路径: %w", err)
	}
	info, err := os.Stat(abs)
	if err != nil {
		return WorkspaceView{}, fmt.Errorf("读取路径: %w", err)
	}
	if !info.IsDir() {
		return WorkspaceView{}, errors.New("请选择文件夹")
	}
	if trust != "trusted" {
		trust = "restricted"
	}
	now := store.NowMillis()
	workspace := &store.Workspace{
		ID:           id.New("ws"),
		Name:         filepath.Base(abs),
		Path:         abs,
		Trust:        trust,
		CreatedAt:    now,
		LastOpenedAt: now,
	}
	if err := s.db.UpsertWorkspace(workspace); err != nil {
		return WorkspaceView{}, err
	}
	if err := s.activate(workspace); err != nil {
		return WorkspaceView{}, err
	}
	return toWorkspaceView(*workspace), nil
}

// SetActive 切换当前活动工作区。
func (s *WorkspaceService) SetActive(workspaceID string) (WorkspaceView, error) {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return WorkspaceView{}, err
	}
	if err := s.activate(workspace); err != nil {
		return WorkspaceView{}, err
	}
	return toWorkspaceView(*workspace), nil
}

// Active 返回当前活动工作区。
func (s *WorkspaceService) Active() (*WorkspaceView, error) {
	settings := DefaultSettings()
	_ = s.db.GetSetting(settingsKey, &settings)
	if settings.ActiveWorkspace == "" {
		return nil, store.ErrNotFound
	}
	workspace, err := s.db.GetWorkspace(settings.ActiveWorkspace)
	if err != nil {
		return nil, err
	}
	view := toWorkspaceView(*workspace)
	return &view, nil
}

// SetTrust 修改工作区信任状态。
func (s *WorkspaceService) SetTrust(workspaceID, trust string) error {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return err
	}
	if trust != "trusted" {
		trust = "restricted"
	}
	workspace.Trust = trust
	if err := s.db.UpsertWorkspace(workspace); err != nil {
		return err
	}
	if s.engine.Workspace() == workspace.Path {
		s.engine.SetRestricted(trust != "trusted")
	}
	s.bus.Publish(events.TypeWorkspaceUpdated, "", "", map[string]any{
		"workspaceId": workspace.ID,
		"trust":       trust,
	})
	return nil
}

// Remove 移除工作区记录，不删除磁盘文件。
func (s *WorkspaceService) Remove(workspaceID string) error {
	settings := DefaultSettings()
	_ = s.db.GetSetting(settingsKey, &settings)
	if settings.ActiveWorkspace == workspaceID {
		settings.ActiveWorkspace = ""
		if err := s.db.SetSetting(settingsKey, settings); err != nil {
			return err
		}
		s.engine.SetWorkspace("")
		s.registry.SetWorkspace("")
	}
	if err := s.db.DeleteWorkspace(workspaceID); err != nil {
		return err
	}
	s.bus.Publish(events.TypeWorkspaceUpdated, "", "", map[string]any{
		"workspaceId": workspaceID,
		"removed":     true,
	})
	return nil
}

// ListDir 读取目录的一层内容。
func (s *WorkspaceService) ListDir(workspaceID, rel string) ([]FileNode, error) {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return nil, err
	}
	abs, err := policy.Resolve(workspace.Path, rel)
	if err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(abs)
	if err != nil {
		return nil, fmt.Errorf("读取目录: %w", err)
	}

	nodes := make([]FileNode, 0, len(entries))
	for _, entry := range entries {
		name := entry.Name()
		if strings.HasPrefix(name, ".") {
			continue
		}
		if entry.IsDir() && tools.IsIgnoredDir(name) {
			continue
		}
		info, err := entry.Info()
		if err != nil {
			continue
		}
		nodeType := "file"
		if entry.IsDir() {
			nodeType = "dir"
		}
		nodes = append(nodes, FileNode{
			Name:     name,
			Path:     policy.Relative(workspace.Path, filepath.Join(abs, name)),
			Type:     nodeType,
			Size:     info.Size(),
			Modified: info.ModTime().UnixMilli(),
		})
	}
	sort.Slice(nodes, func(i, j int) bool {
		if nodes[i].Type != nodes[j].Type {
			return nodes[i].Type == "dir"
		}
		return nodes[i].Name < nodes[j].Name
	})
	return nodes, nil
}

// ReadFile 读取工作区文件。
func (s *WorkspaceService) ReadFile(workspaceID, rel string) (FileContent, error) {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return FileContent{}, err
	}
	abs, err := policy.Resolve(workspace.Path, rel)
	if err != nil {
		return FileContent{}, err
	}
	info, err := os.Stat(abs)
	if err != nil {
		return FileContent{}, fmt.Errorf("读取文件: %w", err)
	}
	if info.IsDir() {
		return FileContent{}, errors.New("这是一个目录")
	}
	if info.Size() > 10*1024*1024 {
		return FileContent{}, fmt.Errorf("文件过大（%d MB），暂不支持在编辑器中打开", info.Size()/1024/1024)
	}
	data, err := os.ReadFile(abs)
	if err != nil {
		return FileContent{}, fmt.Errorf("读取文件: %w", err)
	}

	display := policy.Relative(workspace.Path, abs)
	result := FileContent{
		Path:     display,
		Size:     info.Size(),
		Language: languageOf(abs),
		Hash:     platform.Sha256Hex(data),
		Modified: info.ModTime().UnixMilli(),
	}
	if platform.IsBinary(data) {
		result.Binary = true
		return result, nil
	}
	if len(data) > 2*1024*1024 {
		data = data[:2*1024*1024]
		result.Truncated = true
	}
	result.Content = string(data)
	result.Lines = strings.Count(result.Content, "\n") + 1
	return result, nil
}

// WriteFile 保存编辑器内容。写入前备份，并在外部修改时拒绝覆盖。
func (s *WorkspaceService) WriteFile(workspaceID, rel, content, expectedHash string) error {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return err
	}
	abs, err := policy.Resolve(workspace.Path, rel)
	if err != nil {
		return err
	}

	mode := os.FileMode(0o644)
	if existing, readErr := os.ReadFile(abs); readErr == nil {
		if expectedHash != "" && platform.Sha256Hex(existing) != expectedHash {
			return errors.New("文件已被外部修改，请重新加载后再保存")
		}
		if info, statErr := os.Stat(abs); statErr == nil {
			mode = info.Mode().Perm()
		}
		if err := s.backup(abs, existing); err != nil {
			return err
		}
	}
	if err := os.MkdirAll(filepath.Dir(abs), 0o755); err != nil {
		return fmt.Errorf("创建目录: %w", err)
	}
	if err := platform.WriteFileAtomic(abs, []byte(content), mode); err != nil {
		return err
	}
	_ = s.db.AddAudit(&store.AuditEntry{
		Tool:     "editor_write",
		Risk:     policy.R1.String(),
		Decision: "allow",
		Scope:    "user",
		Detail:   policy.Relative(workspace.Path, abs),
	})
	return nil
}

// ComputeDiff 计算两段文本的行级差异。
func (s *WorkspaceService) ComputeDiff(oldText, newText string) editor.Result {
	return editor.Diff(oldText, newText)
}

// GitStatuses 返回工作区的 Git 文件状态，非 Git 仓库返回空表。
func (s *WorkspaceService) GitStatuses(workspaceID string) (map[string]string, error) {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return nil, err
	}
	cmd := exec.Command("git", "status", "--porcelain")
	cmd.Dir = workspace.Path
	output, err := cmd.Output()
	result := map[string]string{}
	if err != nil {
		return result, nil
	}
	for _, line := range strings.Split(string(output), "\n") {
		if len(line) < 4 {
			continue
		}
		code := strings.TrimSpace(line[:2])
		path := strings.TrimSpace(line[3:])
		if index := strings.Index(path, " -> "); index >= 0 {
			path = path[index+4:]
		}
		if path != "" {
			result[path] = code
		}
	}
	return result, nil
}

func (s *WorkspaceService) activate(workspace *store.Workspace) error {
	workspace.LastOpenedAt = store.NowMillis()
	if err := s.db.UpsertWorkspace(workspace); err != nil {
		return err
	}
	settings := DefaultSettings()
	_ = s.db.GetSetting(settingsKey, &settings)
	settings.ActiveWorkspace = workspace.ID
	if err := s.db.SetSetting(settingsKey, settings); err != nil {
		return err
	}
	s.engine.SetWorkspace(workspace.Path)
	s.engine.SetRestricted(workspace.Trust != "trusted")
	s.registry.SetWorkspace(workspace.Path)
	s.bus.Publish(events.TypeWorkspaceUpdated, "", "", map[string]any{
		"workspaceId": workspace.ID,
		"active":      true,
	})
	return nil
}

func (s *WorkspaceService) backup(absPath string, data []byte) error {
	if s.backupDir == "" {
		return nil
	}
	if err := os.MkdirAll(s.backupDir, 0o700); err != nil {
		return fmt.Errorf("创建备份目录: %w", err)
	}
	backupPath := filepath.Join(s.backupDir, platform.BackupName(absPath))
	if err := os.WriteFile(backupPath, data, 0o600); err != nil {
		return fmt.Errorf("写入备份: %w", err)
	}
	hash := platform.Sha256Hex(data)
	return s.db.AddBackup(&store.Backup{
		ID:         id.New("bk"),
		Path:       absPath,
		BackupPath: backupPath,
		Hash:       hash,
		Size:       int64(len(data)),
	})
}

func toWorkspaceView(workspace store.Workspace) WorkspaceView {
	return WorkspaceView{
		ID:           workspace.ID,
		Name:         workspace.Name,
		Path:         workspace.Path,
		Trust:        workspace.Trust,
		CreatedAt:    workspace.CreatedAt,
		LastOpenedAt: workspace.LastOpenedAt,
	}
}

func languageOf(path string) string {
	switch strings.ToLower(filepath.Ext(path)) {
	case ".go":
		return "go"
	case ".ts", ".tsx":
		return "typescript"
	case ".js", ".jsx", ".mjs", ".cjs":
		return "javascript"
	case ".json":
		return "json"
	case ".md", ".markdown":
		return "markdown"
	case ".css", ".scss", ".less":
		return "css"
	case ".html", ".htm":
		return "html"
	case ".py":
		return "python"
	case ".rs":
		return "rust"
	case ".java", ".kt":
		return "java"
	case ".yaml", ".yml":
		return "yaml"
	case ".toml":
		return "toml"
	case ".sh", ".bash", ".zsh":
		return "shell"
	case ".sql":
		return "sql"
	default:
		return "text"
	}
}
