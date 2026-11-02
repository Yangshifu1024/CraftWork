// 本文件负责会话、工作区与设置的读写：创建与查询会话、维护会话状态与归档、
// 管理工作区记录，以及把任意值以 JSON 形式存入 settings 表。
//
// 会话是软删除：deleted_at 非空即视为不存在，查询时统一过滤。

package store

import (
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/yangshifu1024/craftwork/internal/id"
)

// MarshalPayload 把事件载荷编码为 JSON 文本。
func MarshalPayload(v any) (string, error) {
	data, err := json.Marshal(v)
	if err != nil {
		return "", fmt.Errorf("编码事件载荷: %w", err)
	}
	return string(data), nil
}

// UnmarshalPayload 把数据库中的 JSON 文本还原为前端可消费的对象。
func UnmarshalPayload(raw string) (any, error) {
	var value any
	if err := json.Unmarshal([]byte(raw), &value); err != nil {
		return nil, fmt.Errorf("解析事件载荷: %w", err)
	}
	return value, nil
}

// nowMillis 返回当前毫秒时间戳，包内所有时间字段统一使用这个口径。
func nowMillis() int64 {
	return time.Now().UnixMilli()
}

// CreateThread 写入新会话。
func (db *DB) CreateThread(thread *Thread) error {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()
	_, err := db.sqlDB.Exec(
		`INSERT INTO threads (id, mode, title, title_auto, model_id, workspace_id, status, archived, tokens, created_at, updated_at)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		thread.ID, thread.Mode, thread.Title, boolToInt(thread.TitleAuto), thread.ModelID, thread.WorkspaceID,
		thread.Status, boolToInt(thread.Archived), thread.Tokens, thread.CreatedAt, thread.UpdatedAt,
	)
	if err != nil {
		return fmt.Errorf("创建会话: %w", err)
	}
	return nil
}

// GetThread 读取单个会话。
func (db *DB) GetThread(id string) (*Thread, error) {
	row := db.sqlDB.QueryRow(
		`SELECT id, mode, title, title_auto, model_id, workspace_id, status, archived, tokens, created_at, updated_at
		 FROM threads WHERE id = ? AND deleted_at IS NULL`, id)
	thread, err := scanThread(row)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, fmt.Errorf("读取会话: %w", err)
	}
	return thread, nil
}

// ListThreads 按模式列出会话，默认不包含归档。
func (db *DB) ListThreads(mode string, includeArchived bool) ([]Thread, error) {
	query := `SELECT id, mode, title, title_auto, model_id, workspace_id, status, archived, tokens, created_at, updated_at
	          FROM threads WHERE mode = ? AND deleted_at IS NULL`
	if !includeArchived {
		query += ` AND archived = 0`
	}
	query += ` ORDER BY updated_at DESC`
	rows, err := db.sqlDB.Query(query, mode)
	if err != nil {
		return nil, fmt.Errorf("列出会话: %w", err)
	}
	defer rows.Close()

	var result []Thread
	for rows.Next() {
		thread, err := scanThread(rows)
		if err != nil {
			return nil, fmt.Errorf("扫描会话: %w", err)
		}
		result = append(result, *thread)
	}
	return result, rows.Err()
}

// SearchThreads 按标题或消息内容搜索会话。
//
// 内容搜索直接匹配事件的 JSON 载荷，属于简单包含查询；限制 100 条避免
// 在大量历史消息上做全表扫描时把界面拖慢。
func (db *DB) SearchThreads(mode, query string) ([]Thread, error) {
	like := "%" + strings.TrimSpace(query) + "%"
	rows, err := db.sqlDB.Query(
		`SELECT DISTINCT t.id, t.mode, t.title, t.title_auto, t.model_id, t.workspace_id, t.status, t.archived, t.tokens, t.created_at, t.updated_at
		 FROM threads t
		 LEFT JOIN events e ON e.thread_id = t.id
		 WHERE t.mode = ? AND t.deleted_at IS NULL AND (t.title LIKE ? OR e.payload LIKE ?)
		 ORDER BY t.updated_at DESC LIMIT 100`, mode, like, like)
	if err != nil {
		return nil, fmt.Errorf("搜索会话: %w", err)
	}
	defer rows.Close()

	var result []Thread
	for rows.Next() {
		thread, err := scanThread(rows)
		if err != nil {
			return nil, fmt.Errorf("扫描会话: %w", err)
		}
		result = append(result, *thread)
	}
	return result, rows.Err()
}

// RenameThread 修改会话标题，并把标题标记为用户手动命名。
func (db *DB) RenameThread(id, title string) error {
	return db.updateThread(id,
		`UPDATE threads SET title = ?, title_auto = 0, updated_at = ? WHERE id = ?`,
		title, nowMillis(), id)
}

// AutoTitleThread 在标题仍允许自动命名时写入首轮短标题。
//
// 条件里同时检查 title_auto 与当前标题，避免用户手动改名、或另一条消息
// 已经完成自动命名后再次覆盖。返回是否实际更新了标题。
func (db *DB) AutoTitleThread(id, title, previousTitle string) (bool, error) {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()
	result, err := db.sqlDB.Exec(
		`UPDATE threads SET title = ?, title_auto = 0, updated_at = ?
		 WHERE id = ? AND title_auto = 1 AND title = ?`,
		title, nowMillis(), id, previousTitle)
	if err != nil {
		return false, fmt.Errorf("自动命名会话: %w", err)
	}
	affected, _ := result.RowsAffected()
	return affected > 0, nil
}

// SetThreadStatus 更新会话运行状态。
func (db *DB) SetThreadStatus(id, status string) error {
	return db.updateThread(id, `UPDATE threads SET status = ?, updated_at = ? WHERE id = ?`, status, nowMillis(), id)
}

// SetThreadModel 更新会话当前模型。
func (db *DB) SetThreadModel(id, modelID string) error {
	return db.updateThread(id, `UPDATE threads SET model_id = ?, updated_at = ? WHERE id = ?`, modelID, nowMillis(), id)
}

// SetThreadArchived 归档或取消归档。
func (db *DB) SetThreadArchived(id string, archived bool) error {
	return db.updateThread(id, `UPDATE threads SET archived = ?, updated_at = ? WHERE id = ?`, boolToInt(archived), nowMillis(), id)
}

// TouchThread 更新会话的最近活动时间。
func (db *DB) TouchThread(id string) error {
	return db.updateThread(id, `UPDATE threads SET updated_at = ? WHERE id = ?`, nowMillis(), id)
}

// AddThreadTokens 累加会话 token 用量。
func (db *DB) AddThreadTokens(id string, tokens int64) error {
	if tokens == 0 {
		return nil
	}
	return db.updateThread(id, `UPDATE threads SET tokens = tokens + ?, updated_at = ? WHERE id = ?`, tokens, nowMillis(), id)
}

// DeleteThread 彻底删除会话及其事件、审批与运行记录。
//
// 这是物理删除，不可恢复；调用方需要自行确认用户意图。四条删除语句放在
// 同一个事务里，避免留下孤立的事件或审批行。
func (db *DB) DeleteThread(id string) error {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()

	tx, err := db.sqlDB.Begin()
	if err != nil {
		return fmt.Errorf("开始删除会话事务: %w", err)
	}
	defer tx.Rollback()

	for _, statement := range []string{
		`DELETE FROM events WHERE thread_id = ?`,
		`DELETE FROM approvals WHERE thread_id = ?`,
		`DELETE FROM runs WHERE thread_id = ?`,
		`DELETE FROM threads WHERE id = ?`,
	} {
		if _, err := tx.Exec(statement, id); err != nil {
			return fmt.Errorf("删除会话数据: %w", err)
		}
	}
	if err := tx.Commit(); err != nil {
		return fmt.Errorf("提交删除会话: %w", err)
	}
	return nil
}

// UpsertWorkspace 按路径新增或更新工作区。已存在时保留原 id 与用途标记。
//
// 路径是唯一键，这样用户重复打开同一目录不会产生多条记录；用途由首次
// 创建决定（内置临时目录不会被后续写入改回项目），这里不更新 purpose。
func (db *DB) UpsertWorkspace(workspace *Workspace) error {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()

	if workspace.Purpose == "" {
		workspace.Purpose = WorkspacePurposeProject
	}
	var existing string
	err := db.sqlDB.QueryRow(`SELECT id FROM workspaces WHERE path = ?`, workspace.Path).Scan(&existing)
	switch {
	case err == nil:
		// 已存在：沿用原 id 并刷新名称、信任级别与最近打开时间。
		workspace.ID = existing
		_, err = db.sqlDB.Exec(
			`UPDATE workspaces SET name = ?, trust = ?, last_opened_at = ? WHERE id = ?`,
			workspace.Name, workspace.Trust, workspace.LastOpenedAt, workspace.ID)
	case errors.Is(err, sql.ErrNoRows):
		_, err = db.sqlDB.Exec(
			`INSERT INTO workspaces (id, name, path, trust, purpose, created_at, last_opened_at) VALUES (?, ?, ?, ?, ?, ?, ?)`,
			workspace.ID, workspace.Name, workspace.Path, workspace.Trust, workspace.Purpose, workspace.CreatedAt, workspace.LastOpenedAt)
	default:
		return fmt.Errorf("查询工作区: %w", err)
	}
	if err != nil {
		return fmt.Errorf("保存工作区: %w", err)
	}
	return nil
}

// ListWorkspaces 按最近打开时间列出工作区，并装配各自的源文件夹列表。
func (db *DB) ListWorkspaces() ([]Workspace, error) {
	rows, err := db.sqlDB.Query(
		`SELECT id, name, path, trust, purpose, created_at, last_opened_at FROM workspaces ORDER BY last_opened_at DESC`)
	if err != nil {
		return nil, fmt.Errorf("列出工作区: %w", err)
	}
	defer rows.Close()

	var result []Workspace
	for rows.Next() {
		var workspace Workspace
		if err := rows.Scan(&workspace.ID, &workspace.Name, &workspace.Path, &workspace.Trust, &workspace.Purpose,
			&workspace.CreatedAt, &workspace.LastOpenedAt); err != nil {
			return nil, fmt.Errorf("扫描工作区: %w", err)
		}
		result = append(result, workspace)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	// 工作区数量有限，逐个装配文件夹即可，不为少量行引入 join 复杂度。
	for index := range result {
		folders, err := db.ListWorkspaceFolders(result[index].ID)
		if err != nil {
			return nil, err
		}
		result[index].Folders = folders
	}
	return result, nil
}

// GetWorkspace 读取单个工作区，并装配其源文件夹列表。
func (db *DB) GetWorkspace(id string) (*Workspace, error) {
	var workspace Workspace
	err := db.sqlDB.QueryRow(
		`SELECT id, name, path, trust, purpose, created_at, last_opened_at FROM workspaces WHERE id = ?`, id).
		Scan(&workspace.ID, &workspace.Name, &workspace.Path, &workspace.Trust, &workspace.Purpose,
			&workspace.CreatedAt, &workspace.LastOpenedAt)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, fmt.Errorf("读取工作区: %w", err)
	}
	folders, err := db.ListWorkspaceFolders(id)
	if err != nil {
		return nil, err
	}
	workspace.Folders = folders
	return &workspace, nil
}

// ListWorkspaceFolders 按挂载顺序列出工作区的全部源文件夹。
//
// 兼容性兜底：保证返回列表的第一条是 workspaces.path 指向的主要目录。
// 正常数据里 position = 0 的记录就是它；v6 迁移前的存量行或异常数据缺失
// 时，用 workspaces.path 补一条，保证「第一条是主要目录」的约定恒成立。
func (db *DB) ListWorkspaceFolders(workspaceID string) ([]WorkspaceFolder, error) {
	rows, err := db.sqlDB.Query(
		`SELECT id, workspace_id, path, position, created_at FROM workspace_folders WHERE workspace_id = ? ORDER BY position`, workspaceID)
	if err != nil {
		return nil, fmt.Errorf("列出工作区文件夹: %w", err)
	}
	defer rows.Close()

	var result []WorkspaceFolder
	for rows.Next() {
		var folder WorkspaceFolder
		if err := rows.Scan(&folder.ID, &folder.WorkspaceID, &folder.Path, &folder.Position, &folder.CreatedAt); err != nil {
			return nil, fmt.Errorf("扫描工作区文件夹: %w", err)
		}
		result = append(result, folder)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}

	var primaryPath string
	err = db.sqlDB.QueryRow(`SELECT path FROM workspaces WHERE id = ?`, workspaceID).Scan(&primaryPath)
	if errors.Is(err, sql.ErrNoRows) {
		// 工作区本身已删除：原样返回空列表，由调用方处理。
		return result, nil
	}
	if err != nil {
		return nil, fmt.Errorf("读取工作区主目录: %w", err)
	}
	if len(result) == 0 || result[0].Path != primaryPath {
		result = append([]WorkspaceFolder{{
			ID:          id.New("wfo"),
			WorkspaceID: workspaceID,
			Path:        primaryPath,
			Position:    0,
		}}, result...)
	}
	return result, nil
}

// ReplaceWorkspaceFolders 在事务内整体替换工作区的源文件夹列表。
//
// paths 按给定顺序重写，第一条即主要目录（position = 0），同时同步主目录
// 到 workspaces.path 保持两者一致。目录数据由服务层先行校验（存在、是
// 目录、去重），这里只负责持久化。
func (db *DB) ReplaceWorkspaceFolders(workspaceID string, paths []string) error {
	if len(paths) == 0 {
		return errors.New("工作区至少需要一个源文件夹")
	}
	db.writeMu.Lock()
	defer db.writeMu.Unlock()

	tx, err := db.sqlDB.Begin()
	if err != nil {
		return fmt.Errorf("开始替换文件夹事务: %w", err)
	}
	defer tx.Rollback()

	if _, err := tx.Exec(`DELETE FROM workspace_folders WHERE workspace_id = ?`, workspaceID); err != nil {
		return fmt.Errorf("清空工作区文件夹: %w", err)
	}
	now := nowMillis()
	for index, path := range paths {
		if _, err := tx.Exec(
			`INSERT INTO workspace_folders (id, workspace_id, path, position, created_at) VALUES (?, ?, ?, ?, ?)`,
			id.New("wfo"), workspaceID, path, index, now); err != nil {
			return fmt.Errorf("写入工作区文件夹: %w", err)
		}
	}
	if _, err := tx.Exec(`UPDATE workspaces SET path = ? WHERE id = ?`, paths[0], workspaceID); err != nil {
		return fmt.Errorf("同步主目录: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return fmt.Errorf("提交替换文件夹: %w", err)
	}
	return nil
}

// BackfillThreadWorkspaces 把没有绑定工作区的会话归入指定工作区。
//
// 历史版本在无活动工作区时也会创建 workspace_id 为空的会话，这类会话
// 运行时所有路径工具都会失败；引入内置临时目录后统一回填，让它们可以
// 继续使用。重复执行没有副作用。
func (db *DB) BackfillThreadWorkspaces(workspaceID string) error {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()

	if _, err := db.sqlDB.Exec(
		`UPDATE threads SET workspace_id = ? WHERE workspace_id = ''`, workspaceID); err != nil {
		return fmt.Errorf("回填会话工作区: %w", err)
	}
	return nil
}

// DeleteWorkspace 删除工作区记录，不删除磁盘文件。
func (db *DB) DeleteWorkspace(id string) error {
	return db.updateThread(id, `DELETE FROM workspaces WHERE id = ?`, id)
}

// UpdateWorkspacePurpose 修改工作区用途标记。
//
// 仅用于把历史数据里与内置目录同路径的记录纠正为 conversation，
// 正常流程中用途由创建时的 upsert 决定，不再变化。
func (db *DB) UpdateWorkspacePurpose(id, purpose string) error {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()

	if _, err := db.sqlDB.Exec(`UPDATE workspaces SET purpose = ? WHERE id = ?`, purpose, id); err != nil {
		return fmt.Errorf("更新工作区用途: %w", err)
	}
	return nil
}

// GetSetting 读取 JSON 设置。不存在时返回 ErrNotFound。
//
// 设置值在数据库里是一段 JSON 文本，这里直接反序列化到调用方给定的目标。
func (db *DB) GetSetting(key string, target any) error {
	var raw string
	err := db.sqlDB.QueryRow(`SELECT value FROM settings WHERE key = ?`, key).Scan(&raw)
	if errors.Is(err, sql.ErrNoRows) {
		return ErrNotFound
	}
	if err != nil {
		return fmt.Errorf("读取设置 %s: %w", key, err)
	}
	if err := json.Unmarshal([]byte(raw), target); err != nil {
		return fmt.Errorf("解析设置 %s: %w", key, err)
	}
	return nil
}

// SetSetting 写入 JSON 设置。
func (db *DB) SetSetting(key string, value any) error {
	raw, err := json.Marshal(value)
	if err != nil {
		return fmt.Errorf("编码设置 %s: %w", key, err)
	}
	db.writeMu.Lock()
	defer db.writeMu.Unlock()
	if _, err := db.sqlDB.Exec(
		`INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value`,
		key, string(raw)); err != nil {
		return fmt.Errorf("写入设置 %s: %w", key, err)
	}
	return nil
}

// DeleteSetting 删除设置项。
func (db *DB) DeleteSetting(key string) error {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()
	if _, err := db.sqlDB.Exec(`DELETE FROM settings WHERE key = ?`, key); err != nil {
		return fmt.Errorf("删除设置 %s: %w", key, err)
	}
	return nil
}

// updateThread 执行一条以会话 ID 为参数的更新语句，并统一处理“记录不存在”。
//
// 名称沿用了会话场景，但包内的其他单行写操作也复用它，因此错误信息写成“更新记录”。
func (db *DB) updateThread(id, statement string, args ...any) error {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()
	result, err := db.sqlDB.Exec(statement, args...)
	if err != nil {
		return fmt.Errorf("更新记录: %w", err)
	}
	// 没有命中任何行说明目标记录不存在，返回统一的 ErrNotFound 供上层区分。
	affected, _ := result.RowsAffected()
	if affected == 0 {
		return ErrNotFound
	}
	return nil
}

// rowScanner 抽象出 *sql.Row 与 *sql.Rows 的共同能力，让扫描逻辑只写一份。
type rowScanner interface {
	// Scan 把当前行的列值依次写入调用方提供的目标变量。
	Scan(dest ...any) error
}

// scanThread 把一行查询结果扫描成会话对象，并把 SQLite 的整数转换成布尔值。
func scanThread(row rowScanner) (*Thread, error) {
	var thread Thread
	var archived int
	var titleAuto int
	if err := row.Scan(&thread.ID, &thread.Mode, &thread.Title, &titleAuto, &thread.ModelID, &thread.WorkspaceID,
		&thread.Status, &archived, &thread.Tokens, &thread.CreatedAt, &thread.UpdatedAt); err != nil {
		return nil, err
	}
	thread.Archived = archived != 0
	thread.TitleAuto = titleAuto != 0
	return &thread, nil
}

// boolToInt 把布尔值转换成 SQLite 使用的 0/1 整数。
func boolToInt(value bool) int {
	if value {
		return 1
	}
	return 0
}
