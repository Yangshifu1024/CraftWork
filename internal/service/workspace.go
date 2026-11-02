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

	"github.com/yangshifu1024/craftwork/internal/editor"
	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/id"
	"github.com/yangshifu1024/craftwork/internal/platform"
	"github.com/yangshifu1024/craftwork/internal/policy"
	"github.com/yangshifu1024/craftwork/internal/store"
	"github.com/yangshifu1024/craftwork/internal/tools"

	"github.com/wailsapp/wails/v3/pkg/application"
)

// WorkspaceView 是工作区的展示结构。
type WorkspaceView struct {
	// ID 是工作区标识。
	ID string `json:"id"`
	// Name 是工作区展示名称，默认取目录名。
	Name string `json:"name"`
	// Path 是主要目录的绝对路径。
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
	// Folders 是工作区挂载的全部源文件夹，按挂载顺序排列，第一条是主要目录。
	Folders []WorkspaceFolderView `json:"folders,omitempty"`
}

// WorkspaceFolderView 是工作区挂载的一个源文件夹。
//
// 前端文件树按文件夹分段渲染，读写接口以 FolderID 限定目标根
// （空值表示主要目录）。
type WorkspaceFolderView struct {
	// ID 是文件夹记录标识，读写接口用它限定目标根。
	ID string `json:"id"`
	// Path 是目录的绝对路径。
	Path string `json:"path"`
	// Primary 表示这是主要目录（position = 0），即会话的工作目录。
	Primary bool `json:"primary"`
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
	// Path 是相对所属源文件夹的路径。
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
	// WorkspaceID 是读取该文件的工作区标识；保存时显式传回，保存目标
	// 不依赖前端「当前激活工作区」的环境状态。
	WorkspaceID string `json:"workspaceId"`
	// FolderID 是该文件所属源文件夹的标识；保存时显式传回以限定目标根，
	// 空值表示主要目录。
	FolderID string `json:"folderId,omitempty"`
}

// LocatedFile 是 LocateFile 的解析结果：正文引用的短路径在多个源文件夹
// 中唯一命中后，返回所属文件夹与相对路径。
type LocatedFile struct {
	// FolderID 是命中文件所属的源文件夹标识；空值表示主要目录。
	FolderID string `json:"folderId,omitempty"`
	// Path 是相对该源文件夹的路径。
	Path string `json:"path"`
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
	abs, err := normalizeFolder(path)
	if err != nil {
		return WorkspaceView{}, err
	}
	if err := s.rejectConversationPath(abs); err != nil {
		return WorkspaceView{}, err
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

// AddProject 创建一个多文件夹项目并设为当前活动工作区。
//
// folders 是用户在「创建项目」对话框选择的源文件夹，第一个是主要目录
// （会话工作目录），其余是附加目录（同样纳入 Agent 可访问范围）。name
// 为空时用主要目录的目录名推导；与现有项目重名时自动追加序号。trust
// 由前端在「信任此文件夹」确认通过后传 "trusted"，否则受限。
func (s *WorkspaceService) AddProject(name string, folders []string, trust string) (WorkspaceView, error) {
	if len(folders) == 0 {
		return WorkspaceView{}, errors.New("请至少选择一个源文件夹")
	}
	// 逐个规范化：绝对路径 + 符号链接解析 + 必须是目录 + 去重 + 拒绝内置目录。
	normalized := make([]string, 0, len(folders))
	seen := map[string]bool{}
	for _, folder := range folders {
		abs, err := normalizeFolder(folder)
		if err != nil {
			return WorkspaceView{}, err
		}
		if seen[abs] {
			return WorkspaceView{}, fmt.Errorf("源文件夹重复：%s", abs)
		}
		seen[abs] = true
		if err := s.rejectConversationPath(abs); err != nil {
			return WorkspaceView{}, err
		}
		normalized = append(normalized, abs)
	}
	// 任一文件夹已是其他工作区的源文件夹时拒绝，避免两套授权范围交叉。
	existing, err := s.db.ListWorkspaces()
	if err != nil {
		return WorkspaceView{}, err
	}
	for _, workspace := range existing {
		for _, folder := range workspace.Folders {
			if seen[folder.Path] {
				return WorkspaceView{}, fmt.Errorf("目录已被工作区「%s」使用", workspace.Name)
			}
		}
	}

	if trust != "trusted" {
		trust = "restricted"
	}
	// 名称推导：空名称用主要目录的目录名；重名自动追加序号。
	if strings.TrimSpace(name) == "" {
		name = filepath.Base(normalized[0])
	}
	name = uniqueWorkspaceName(name, existing)

	now := store.NowMillis()
	workspace := &store.Workspace{
		ID:           id.New("ws"),
		Name:         name,
		Path:         normalized[0],
		Trust:        trust,
		Purpose:      store.WorkspacePurposeProject,
		CreatedAt:    now,
		LastOpenedAt: now,
	}
	if err := s.db.UpsertWorkspace(workspace); err != nil {
		return WorkspaceView{}, err
	}
	if err := s.db.ReplaceWorkspaceFolders(workspace.ID, normalized); err != nil {
		return WorkspaceView{}, err
	}
	if err := s.activate(workspace); err != nil {
		return WorkspaceView{}, err
	}
	// 重新读取以装配文件夹列表，返回值与 List 接口口径一致。
	saved, err := s.db.GetWorkspace(workspace.ID)
	if err != nil {
		return WorkspaceView{}, err
	}
	return toWorkspaceView(*saved), nil
}

// normalizeFolder 把用户输入的目录路径规范化：转绝对路径、解析符号链接、
// 确认存在且是目录。
func normalizeFolder(path string) (string, error) {
	path = strings.TrimSpace(path)
	if path == "" {
		return "", errors.New("路径不能为空")
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", fmt.Errorf("解析路径: %w", err)
	}
	abs, err = filepath.EvalSymlinks(abs)
	if err != nil {
		return "", fmt.Errorf("解析路径: %w", err)
	}
	info, err := os.Stat(abs)
	if err != nil {
		return "", fmt.Errorf("读取路径: %w", err)
	}
	if !info.IsDir() {
		return "", errors.New("请选择文件夹")
	}
	return abs, nil
}

// rejectConversationPath 拒绝把内置临时会话目录添加为项目源文件夹：
// 它由应用自管，不允许混进用户项目的授权范围。
func (s *WorkspaceService) rejectConversationPath(abs string) error {
	existing, err := s.db.ListWorkspaces()
	if err != nil {
		return err
	}
	for _, workspace := range existing {
		if workspace.Path == abs && isConversation(&workspace) {
			return errors.New("该目录是内置的临时会话目录，无需添加为工作区")
		}
	}
	return nil
}

// uniqueWorkspaceName 在现有项目名中去重：重名时追加 " (2)"、"(3)" 序号。
func uniqueWorkspaceName(name string, existing []store.Workspace) string {
	taken := map[string]bool{}
	for _, workspace := range existing {
		taken[workspace.Name] = true
	}
	if !taken[name] {
		return name
	}
	for index := 2; ; index++ {
		candidate := fmt.Sprintf("%s (%d)", name, index)
		if !taken[candidate] {
			return candidate
		}
	}
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

// folderRoot 返回 folderID 指定的源文件夹路径；folderID 为空表示主要目录。
// folderID 无法匹配时返回错误，避免把路径解析到错误的根。
func folderRoot(workspace *store.Workspace, folderID string) (string, error) {
	if folderID == "" {
		return workspace.Path, nil
	}
	for _, folder := range workspace.Folders {
		if folder.ID == folderID {
			return folder.Path, nil
		}
	}
	return "", fmt.Errorf("源文件夹不存在或已被移除")
}

// ListDir 读取某个源文件夹下目录的一层内容。
//
// folderID 为空表示主要目录；文件树按文件夹分段渲染，每段显式传入
// 所属文件夹，避免多根之间的同名相对路径歧义。
func (s *WorkspaceService) ListDir(workspaceID, folderID, rel string) ([]FileNode, error) {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return nil, err
	}
	root, err := folderRoot(workspace, folderID)
	if err != nil {
		return nil, err
	}
	// 目录路径同样必须落在该源文件夹内。
	abs, err := policy.Resolve(root, rel)
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
			Path:     policy.Relative(root, filepath.Join(abs, name)),
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

// ReadFile 读取工作区文件。folderID 为空表示主要目录。
//
// 目录、超过 10 MB 的文件会被拒绝；二进制文件只返回元数据，
// 文本超过 2 MB 时截断并标记 Truncated。
func (s *WorkspaceService) ReadFile(workspaceID, folderID, rel string) (FileContent, error) {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return FileContent{}, err
	}
	root, err := folderRoot(workspace, folderID)
	if err != nil {
		return FileContent{}, err
	}
	abs, err := policy.Resolve(root, rel)
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

	display := policy.Relative(root, abs)
	result := FileContent{
		Path:     display,
		Size:     info.Size(),
		Language: languageOf(abs),
		Hash:     platform.Sha256Hex(data),
		Modified: info.ModTime().UnixMilli(),
		// 内容自带工作区与源文件夹身份：前端保存时显式传回，不再读取
		// 「当前激活工作区」环境，避免切换工作区后把内容写进错误的目录。
		WorkspaceID: workspace.ID,
		FolderID:    folderID,
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

// LocateFile 把正文引用里可能不完整的相对路径解析成源文件夹内的真实文件。
//
// 模型输出引用时常写短路径（如 policy.go 指代 internal/policy/policy.go），
// 直接读取会失败。解析顺序：路径原样可用时直接返回；否则遍历全部源文件夹
// 查找「文件名完全一致」或「路径以后缀方式匹配」的文件，唯一命中才返回，
// 零个或多个命中都返回错误，宁可打不开也不打开错误的文件。
// .git、node_modules 等依赖与产物目录不参与匹配。
func (s *WorkspaceService) LocateFile(workspaceID, rel string) (LocatedFile, error) {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return LocatedFile{}, err
	}
	clean := strings.TrimPrefix(strings.TrimPrefix(strings.TrimSpace(rel), "./"), "/")
	if clean == "" {
		return LocatedFile{}, errors.New("路径为空")
	}
	folders := workspaceFolders(workspace)
	// 原样路径已存在（且是文件）时无需查找；按根顺序取第一个命中。
	for _, folder := range folders {
		if abs, err := policy.Resolve(folder.Path, clean); err == nil {
			if info, statErr := os.Stat(abs); statErr == nil && !info.IsDir() {
				return LocatedFile{FolderID: folder.ID, Path: clean}, nil
			}
		}
	}

	// 逐个文件夹查找，收集（文件夹, 相对路径）候选；唯一命中才返回。
	type candidate struct {
		folderID string
		relPath  string
	}
	var byName, bySuffix []candidate
	for _, folder := range folders {
		root := folder.Path
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
				byName = append(byName, candidate{folder.ID, relPath})
			}
			if strings.HasSuffix(relPath, "/"+clean) || relPath == clean {
				bySuffix = append(bySuffix, candidate{folder.ID, relPath})
			}
			return nil
		})
		if err != nil {
			return LocatedFile{}, fmt.Errorf("查找文件: %w", err)
		}
	}
	// 后缀匹配更精确，优先；两类都要求唯一命中。
	for _, candidates := range [][]candidate{bySuffix, byName} {
		if len(candidates) == 1 {
			return LocatedFile{FolderID: candidates[0].folderID, Path: candidates[0].relPath}, nil
		}
	}
	if len(bySuffix) > 1 || len(byName) > 1 {
		return LocatedFile{}, fmt.Errorf("路径 %s 在工作区内匹配到多个文件，请使用更完整的路径", rel)
	}
	return LocatedFile{}, fmt.Errorf("工作区内找不到 %s", rel)
}

// WriteFile 保存编辑器内容。写入前备份，并在外部修改时拒绝覆盖。
//
// folderID 限定目标源文件夹（空值表示主要目录）；expectedHash 是前端
// 打开文件时拿到的哈希，非空且与磁盘内容不一致时说明文件已被其他程序
// 修改，此时返回错误让用户重新加载，避免覆盖。
func (s *WorkspaceService) WriteFile(workspaceID, folderID, rel, content, expectedHash string) error {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return err
	}
	root, err := folderRoot(workspace, folderID)
	if err != nil {
		return err
	}
	abs, err := policy.Resolve(root, rel)
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
		Detail:   policy.Relative(root, abs),
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

// workspaceFolders 返回工作区的全部源文件夹；存储记录缺少文件夹列表时
// 退化为仅主要目录，保证历史数据可用。
func workspaceFolders(workspace *store.Workspace) []store.WorkspaceFolder {
	if len(workspace.Folders) > 0 {
		return workspace.Folders
	}
	return []store.WorkspaceFolder{{
		ID:          "",
		WorkspaceID: workspace.ID,
		Path:        workspace.Path,
		Position:    0,
	}}
}

// toWorkspaceView 把存储层的工作区转换成前端视图。
func toWorkspaceView(workspace store.Workspace) WorkspaceView {
	folders := make([]WorkspaceFolderView, 0, len(workspace.Folders))
	for _, folder := range workspace.Folders {
		folders = append(folders, WorkspaceFolderView{
			ID:      folder.ID,
			Path:    folder.Path,
			Primary: folder.Position == 0,
		})
	}
	return WorkspaceView{
		ID:           workspace.ID,
		Name:         workspace.Name,
		Path:         workspace.Path,
		Trust:        workspace.Trust,
		Purpose:      workspace.Purpose,
		CreatedAt:    workspace.CreatedAt,
		LastOpenedAt: workspace.LastOpenedAt,
		Folders:      folders,
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
