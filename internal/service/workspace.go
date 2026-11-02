// 本文件是工作区相关的前端绑定：工作区增删切换、信任状态、文件树、
// 文件读写、行级差异与 Git 状态。所有路径都经 policy.Resolve 校验，
// 保存文件前会备份并在检测到外部修改时拒绝覆盖。

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
	// ID 是工作区标识。
	ID string `json:"id"`
	// Name 是工作区展示名称，默认取目录名。
	Name string `json:"name"`
	// Path 是工作区根目录的绝对路径。
	Path string `json:"path"`
	// Trust 是 trusted 或 restricted。
	Trust string `json:"trust"`
	// Purpose 是工作区用途：project（用户添加的项目目录）或
	// conversation（应用内置的临时会话目录）。
	Purpose string `json:"purpose"`
	// CreatedAt 是添加时间（毫秒时间戳）。
	CreatedAt int64 `json:"createdAt"`
	// LastOpenedAt 是最后一次设为活动工作区的时间。
	LastOpenedAt int64 `json:"lastOpenedAt"`
}

// FileNode 是文件树中的一个节点（单层，按需加载）。
type FileNode struct {
	// Name 是文件或目录名。
	Name string `json:"name"`
	// Path 是相对工作区的路径。
	Path string `json:"path"`
	// Type 是 dir 或 file。
	Type string `json:"type"`
	// Size 是字节数。
	Size int64 `json:"size"`
	// Modified 是最后修改时间（毫秒时间戳）。
	Modified int64 `json:"modified"`
}

// FileContent 是读取文件的结果。
type FileContent struct {
	// Path 是相对工作区的路径。
	Path string `json:"path"`
	// Content 是文件文本内容；二进制文件不带内容。
	Content string `json:"content"`
	// Size 是文件实际字节数。
	Size int64 `json:"size"`
	// Binary 表示这是二进制文件，无法在编辑器中查看。
	Binary bool `json:"binary"`
	// Language 是编辑器高亮使用的语言标识。
	Language string `json:"language"`
	// Lines 是内容行数。
	Lines int `json:"lines"`
	// Truncated 表示内容超过上限，只返回了前一部分。
	Truncated bool `json:"truncated"`
	// Hash 是原始文件内容的 SHA-256，用于保存时检测外部修改。
	Hash string `json:"hash"`
	// Modified 是最后修改时间（毫秒时间戳）。
	Modified int64 `json:"modified"`
}

// WorkspaceService 管理工作区、文件树、文件读写与差异计算。
//
// 它不持有任何运行策略状态：工作区路径与信任由每次 Agent 运行开始时
// 从会话绑定的工作区解析（agent.RunEnv），这里只维护工作区记录本身。
type WorkspaceService struct {
	// db 是工作区记录与审计的存储入口。
	db *store.DB
	// bus 推送工作区变更与文件变动事件。
	bus *events.Bus
	// backupDir 是文件备份目录。
	backupDir string
}

// NewWorkspaceService 创建工作区服务。
func NewWorkspaceService(db *store.DB, bus *events.Bus, backupDir string) *WorkspaceService {
	return &WorkspaceService{db: db, bus: bus, backupDir: backupDir}
}

// conversationWorkspaceName 是内置临时会话目录的展示名称。
const conversationWorkspaceName = "临时会话"

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
//
// 输入路径会先转成绝对路径并解析符号链接，确保之后的所有边界判断
// 都基于真实的目录位置，而不是可能被替换的链接。
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
	// 内置临时会话目录由应用管理，不允许作为项目工作区重复添加。
	existing, err := s.db.ListWorkspaces()
	if err != nil {
		return WorkspaceView{}, err
	}
	for _, workspace := range existing {
		if workspace.Path == abs && isConversation(&workspace) {
			return WorkspaceView{}, errors.New("该目录是内置的临时会话目录，无需添加为工作区")
		}
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

// EnsureConversationWorkspace 确保内置的临时会话目录存在并返回其工作区记录。
//
// 临时会话是绑定该目录的普通会话：Agent 始终有一个真实的工作目录，
// 路径策略、审批与终端机制全部按现有方式工作。目录固定在数据目录下的
// workspace/default（默认 ~/.craftwork/workspace/default），随
// CRAFTWORK_HOME 一起隔离；被用户删除后下次启动会自动重建。
// 幂等：记录已存在时直接沿用，不刷新最近打开时间。
// 它只在启动期由 app 装配调用，特意实现为包级函数而不是服务方法，
// 避免被 Wails 绑定成前端 API。
func EnsureConversationWorkspace(db *store.DB, dataDir string) (*store.Workspace, error) {
	dir := filepath.Join(dataDir, "workspace", "default")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, fmt.Errorf("创建临时会话目录: %w", err)
	}
	abs, err := filepath.EvalSymlinks(dir)
	if err != nil {
		return nil, fmt.Errorf("解析临时会话目录: %w", err)
	}
	workspaces, err := db.ListWorkspaces()
	if err != nil {
		return nil, err
	}
	for _, workspace := range workspaces {
		if workspace.Path != abs {
			continue
		}
		if workspace.Purpose == store.WorkspacePurposeConversation {
			return &workspace, nil
		}
		// 同路径的历史记录（例如手动添加过该目录）：纠正为内置用途，
		// 并固定为受信任，避免临时会话落到受限策略上。
		workspace.Name = conversationWorkspaceName
		workspace.Trust = "trusted"
		workspace.Purpose = store.WorkspacePurposeConversation
		if err := db.UpsertWorkspace(&workspace); err != nil {
			return nil, err
		}
		if err := db.UpdateWorkspacePurpose(workspace.ID, store.WorkspacePurposeConversation); err != nil {
			return nil, err
		}
		return &workspace, nil
	}
	now := store.NowMillis()
	workspace := &store.Workspace{
		ID:           id.New("ws"),
		Name:         conversationWorkspaceName,
		Path:         abs,
		Trust:        "trusted",
		Purpose:      store.WorkspacePurposeConversation,
		CreatedAt:    now,
		LastOpenedAt: now,
	}
	if err := db.UpsertWorkspace(workspace); err != nil {
		return nil, err
	}
	return workspace, nil
}

// isConversation 判断工作区是否为内置的临时会话目录。
func isConversation(workspace *store.Workspace) bool {
	return workspace.Purpose == store.WorkspacePurposeConversation
}

// SetTrust 修改工作区信任状态。
func (s *WorkspaceService) SetTrust(workspaceID, trust string) error {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return err
	}
	// 内置临时目录固定为受信任：它是应用自管的 scratch 目录，
	// 审批体验与受信任工作区保持一致，不允许改成受限。
	if isConversation(workspace) {
		return errors.New("内置临时目录的信任状态不可修改")
	}
	if trust != "trusted" {
		trust = "restricted"
	}
	workspace.Trust = trust
	if err := s.db.UpsertWorkspace(workspace); err != nil {
		return err
	}
	s.bus.Publish(events.TypeWorkspaceUpdated, "", "", map[string]any{
		"workspaceId": workspace.ID,
		"trust":       trust,
	})
	return nil
}

// Remove 移除工作区记录，不删除磁盘文件。
//
// 如果移除的是当前活动工作区，同时清空设置里的指针，避免界面继续
// 指向已不存在的工作区；运行中的会话不受影响——它们的环境在运行
// 开始时已经解析成值。
func (s *WorkspaceService) Remove(workspaceID string) error {
	// 内置临时目录是临时会话的运行根，移除会让所有临时会话失效，直接拒绝。
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return err
	}
	if isConversation(workspace) {
		return errors.New("内置临时目录不能移除")
	}
	settings := DefaultSettings()
	_ = s.db.GetSetting(settingsKey, &settings)
	if settings.ActiveWorkspace == workspaceID {
		settings.ActiveWorkspace = ""
		if err := s.db.SetSetting(settingsKey, settings); err != nil {
			return err
		}
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
	// 目录路径同样必须落在工作区内。
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
//
// 目录、超过 10 MB 的文件会被拒绝；二进制文件只返回元数据，
// 文本超过 2 MB 时截断并标记 Truncated。
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

// LocateFile 把正文引用里可能不完整的相对路径解析成工作区内的真实文件。
//
// 模型输出引用时常写短路径（如 policy.go 指代 internal/policy/policy.go），
// 直接读取会失败。解析顺序：路径原样可用时直接返回；否则遍历工作区查找
// 「文件名完全一致」或「路径以后缀方式匹配」的文件，唯一命中才返回，
// 零个或多个命中都返回错误，宁可打不开也不打开错误的文件。
// .git、node_modules 等依赖与产物目录不参与匹配。
func (s *WorkspaceService) LocateFile(workspaceID, rel string) (string, error) {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return "", err
	}
	clean := strings.TrimPrefix(strings.TrimPrefix(strings.TrimSpace(rel), "./"), "/")
	if clean == "" {
		return "", errors.New("路径为空")
	}
	// 原样路径已存在（且是文件）时无需查找。
	if abs, err := policy.Resolve(workspace.Path, clean); err == nil {
		if info, statErr := os.Stat(abs); statErr == nil && !info.IsDir() {
			return clean, nil
		}
	}

	var byName, bySuffix []string
	root := workspace.Path
	err = filepath.WalkDir(root, func(path string, entry os.DirEntry, walkErr error) error {
		if walkErr != nil {
			return nil // 单个目录不可读不中断整体查找。
		}
		if entry.IsDir() {
			name := entry.Name()
			if path != root && (name == ".git" || name == "node_modules" || name == "dist" || name == "bin" || name == ".task") {
				return filepath.SkipDir
			}
			return nil
		}
		relPath, err := filepath.Rel(root, path)
		if err != nil {
			return nil
		}
		relPath = filepath.ToSlash(relPath)
		if entry.Name() == filepath.Base(clean) {
			byName = append(byName, relPath)
		}
		if strings.HasSuffix(relPath, "/"+clean) || relPath == clean {
			bySuffix = append(bySuffix, relPath)
		}
		return nil
	})
	if err != nil {
		return "", fmt.Errorf("查找文件: %w", err)
	}
	// 后缀匹配更精确，优先；两类都要求唯一命中。
	for _, candidates := range [][]string{bySuffix, byName} {
		if len(candidates) == 1 {
			return candidates[0], nil
		}
	}
	if len(bySuffix) > 1 || len(byName) > 1 {
		return "", fmt.Errorf("路径 %s 在工作区内匹配到多个文件，请使用更完整的路径", rel)
	}
	return "", fmt.Errorf("工作区内找不到 %s", rel)
}

// WriteFile 保存编辑器内容。写入前备份，并在外部修改时拒绝覆盖。
//
// expectedHash 是前端打开文件时拿到的哈希；非空且与磁盘内容不一致，
// 说明文件已被其他程序修改，此时返回错误让用户重新加载，避免覆盖。
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
	// 原子替换：先写同目录临时文件，再重命名覆盖目标，避免留下半截文件。
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
		// porcelain 每行格式为“XY 路径”，重命名还会出现“旧 -> 新”。
		if len(line) < 4 {
			continue
		}
		code := strings.TrimSpace(line[:2])
		path := strings.TrimSpace(line[3:])
		// 重命名只看新路径，旧路径对界面没有意义。
		if index := strings.Index(path, " -> "); index >= 0 {
			path = path[index+4:]
		}
		if path != "" {
			result[path] = code
		}
	}
	return result, nil
}

// activate 把工作区设为活动状态。
//
// 只更新数据库中的活动工作区设置与目录访问时间；运行环境不在这里
// 同步——每个会话在运行开始时从自己绑定的工作区解析。
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
	s.bus.Publish(events.TypeWorkspaceUpdated, "", "", map[string]any{
		"workspaceId": workspace.ID,
		"active":      true,
	})
	return nil
}

// backup 把文件当前内容保存到备份目录，并写入一条备份记录。
// 未配置备份目录时静默跳过。
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

// toWorkspaceView 把存储层的工作区转换成前端视图。
func toWorkspaceView(workspace store.Workspace) WorkspaceView {
	return WorkspaceView{
		ID:           workspace.ID,
		Name:         workspace.Name,
		Path:         workspace.Path,
		Trust:        workspace.Trust,
		Purpose:      workspace.Purpose,
		CreatedAt:    workspace.CreatedAt,
		LastOpenedAt: workspace.LastOpenedAt,
	}
}

// languageOf 根据扩展名推断编辑器高亮语言，未知类型返回 text。
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
