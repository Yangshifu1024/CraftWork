// 本文件负责审计记录、用量统计与备份记录的写入与查询：追加审计、按时间
// 倒序读取审计、按天/按模型聚合用量，以及备份记录的写入与读取。

package store

import (
	"database/sql"
	"errors"
	"fmt"
	"time"
)

// AddAudit 追加一条审计记录。
//
// 记录时间由数据库写入前补齐（未提供时取当前时间），主键由数据库自增生成，
// 最后回填到入参上。审计记录只追加、不修改，保证可追溯。
func (db *DB) AddAudit(entry *AuditEntry) error {
	if entry.TS == 0 {
		entry.TS = nowMillis()
	}
	// 写操作串行化，避免并发写入同一数据库时互相干扰。
	db.writeMu.Lock()
	defer db.writeMu.Unlock()
	result, err := db.sqlDB.Exec(
		`INSERT INTO audit_log (ts, thread_id, run_id, tool, risk, decision, scope, detail)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
		entry.TS, entry.ThreadID, entry.RunID, entry.Tool, entry.Risk,
		entry.Decision, entry.Scope, entry.Detail)
	if err != nil {
		return fmt.Errorf("写入审计: %w", err)
	}
	entry.ID, _ = result.LastInsertId()
	return nil
}

// ListAudit 按时间倒序返回审计记录。
//
// limit 是最多返回条数；小于等于 0 或超过 500 时统一按 200 处理，防止一次
// 拉取过多数据。可空字段用 COALESCE 转成空字符串，扫描时不必处理 NULL。
func (db *DB) ListAudit(limit int) ([]AuditEntry, error) {
	if limit <= 0 || limit > 500 {
		limit = 200
	}
	rows, err := db.sqlDB.Query(
		`SELECT id, ts, COALESCE(thread_id, ''), COALESCE(run_id, ''), tool, risk,
		        decision, COALESCE(scope, ''), COALESCE(detail, '')
		 FROM audit_log ORDER BY ts DESC LIMIT ?`, limit)
	if err != nil {
		return nil, fmt.Errorf("读取审计: %w", err)
	}
	defer rows.Close()

	var result []AuditEntry
	for rows.Next() {
		var entry AuditEntry
		if err := rows.Scan(&entry.ID, &entry.TS, &entry.ThreadID, &entry.RunID, &entry.Tool,
			&entry.Risk, &entry.Decision, &entry.Scope, &entry.Detail); err != nil {
			return nil, fmt.Errorf("扫描审计: %w", err)
		}
		result = append(result, entry)
	}
	return result, rows.Err()
}

// AddUsage 记录一次模型调用的用量。记录时间缺失时取当前时间。
func (db *DB) AddUsage(row *UsageRow) error {
	if row.TS == 0 {
		row.TS = nowMillis()
	}
	db.writeMu.Lock()
	defer db.writeMu.Unlock()
	if _, err := db.sqlDB.Exec(
		`INSERT INTO usage (ts, thread_id, model_id, input_tokens, output_tokens, cache_tokens, reasoning_tokens, cost_micros)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
		row.TS, row.ThreadID, row.ModelID, row.InputTokens, row.OutputTokens,
		row.CacheTokens, row.ReasoningTokens, row.CostMicros); err != nil {
		return fmt.Errorf("写入用量: %w", err)
	}
	return nil
}

// UsageSummary 聚合今天、近 7 天、近 30 天的用量。
//
// 时间窗口都从本地时区的今天零点开始计算：今天是从零到现在，近 7 天含今天
// 共 7 个自然日，近 30 天含今天共 30 个自然日；按模型与按天的明细同样取近
// 30 天，其中按天明细只返回最近 14 天。
func (db *DB) UsageSummary() (*UsageSummary, error) {
	now := time.Now()
	startOfDay := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location()).UnixMilli()
	// day 是一天的毫秒数，用于回推窗口起点。
	const day = int64(24 * time.Hour / time.Millisecond)

	summary := &UsageSummary{}
	var err error
	if summary.Today, err = db.usageTotals(startOfDay); err != nil {
		return nil, err
	}
	if summary.Week, err = db.usageTotals(startOfDay - 6*day); err != nil {
		return nil, err
	}
	if summary.Month, err = db.usageTotals(startOfDay - 29*day); err != nil {
		return nil, err
	}
	if summary.ByModel, err = db.usageByModel(startOfDay - 29*day); err != nil {
		return nil, err
	}
	if summary.ByDay, err = db.usageByDay(startOfDay - 13*day); err != nil {
		return nil, err
	}
	return summary, nil
}

// usageTotals 聚合指定时间之后的用量合计。COALESCE 保证没有记录时返回 0
// 而不是 NULL。
func (db *DB) usageTotals(since int64) (UsageTotals, error) {
	var totals UsageTotals
	err := db.sqlDB.QueryRow(
		`SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
		        COALESCE(SUM(cache_tokens), 0), COALESCE(SUM(cost_micros), 0), COUNT(*)
		 FROM usage WHERE ts >= ?`, since).
		Scan(&totals.InputTokens, &totals.OutputTokens, &totals.CacheTokens, &totals.CostMicros, &totals.Runs)
	if err != nil {
		return UsageTotals{}, fmt.Errorf("聚合用量: %w", err)
	}
	return totals, nil
}

// usageByModel 按模型聚合指定时间之后的用量，结果按输入加输出的 token 总量
// 从高到低排列，方便界面直接展示用量最多的模型。
func (db *DB) usageByModel(since int64) ([]ModelUsage, error) {
	rows, err := db.sqlDB.Query(
		`SELECT model_id, COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
		        COALESCE(SUM(cost_micros), 0), COUNT(*)
		 FROM usage WHERE ts >= ? GROUP BY model_id
		 ORDER BY SUM(input_tokens + output_tokens) DESC`, since)
	if err != nil {
		return nil, fmt.Errorf("按模型聚合用量: %w", err)
	}
	defer rows.Close()

	var result []ModelUsage
	for rows.Next() {
		var item ModelUsage
		if err := rows.Scan(&item.ModelID, &item.InputTokens, &item.OutputTokens,
			&item.CostMicros, &item.Runs); err != nil {
			return nil, fmt.Errorf("扫描模型用量: %w", err)
		}
		result = append(result, item)
	}
	return result, rows.Err()
}

// usageByDay 按本地自然日聚合指定时间之后的用量。毫秒时间戳先除以 1000 转成
// 秒，再交给 SQLite 的日期函数按本地时区分组，结果按日期升序。
func (db *DB) usageByDay(since int64) ([]DailyUsage, error) {
	rows, err := db.sqlDB.Query(
		`SELECT date(ts / 1000, 'unixepoch', 'localtime') AS day,
		        COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
		        COALESCE(SUM(cost_micros), 0), COUNT(*)
		 FROM usage WHERE ts >= ? GROUP BY day ORDER BY day`, since)
	if err != nil {
		return nil, fmt.Errorf("按天聚合用量: %w", err)
	}
	defer rows.Close()

	var result []DailyUsage
	for rows.Next() {
		var item DailyUsage
		if err := rows.Scan(&item.Day, &item.InputTokens, &item.OutputTokens,
			&item.CostMicros, &item.Runs); err != nil {
			return nil, fmt.Errorf("扫描每日用量: %w", err)
		}
		result = append(result, item)
	}
	return result, rows.Err()
}

// AddBackup 记录一次写入前备份。
func (db *DB) AddBackup(backup *Backup) error {
	if backup.CreatedAt == 0 {
		backup.CreatedAt = nowMillis()
	}
	db.writeMu.Lock()
	defer db.writeMu.Unlock()
	if _, err := db.sqlDB.Exec(
		`INSERT INTO backups (id, path, backup_path, hash, size, created_at)
		 VALUES (?, ?, ?, ?, ?, ?)`,
		backup.ID, backup.Path, backup.BackupPath, backup.Hash, backup.Size, backup.CreatedAt); err != nil {
		return fmt.Errorf("写入备份记录: %w", err)
	}
	return nil
}

// GetBackup 读取单条备份记录。
func (db *DB) GetBackup(id string) (*Backup, error) {
	var backup Backup
	err := db.sqlDB.QueryRow(
		`SELECT id, path, backup_path, hash, size, created_at FROM backups WHERE id = ?`, id).
		Scan(&backup.ID, &backup.Path, &backup.BackupPath, &backup.Hash, &backup.Size, &backup.CreatedAt)
	if err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return nil, ErrNotFound
		}
		return nil, fmt.Errorf("读取备份记录: %w", err)
	}
	return &backup, nil
}

// ListBackups 按时间倒序返回备份记录。
//
// limit 小于等于 0 或超过 200 时按 50 处理，避免一次返回过多记录。
func (db *DB) ListBackups(limit int) ([]Backup, error) {
	if limit <= 0 || limit > 200 {
		limit = 50
	}
	rows, err := db.sqlDB.Query(
		`SELECT id, path, backup_path, hash, size, created_at
		 FROM backups ORDER BY created_at DESC LIMIT ?`, limit)
	if err != nil {
		return nil, fmt.Errorf("读取备份: %w", err)
	}
	defer rows.Close()

	var result []Backup
	for rows.Next() {
		var backup Backup
		if err := rows.Scan(&backup.ID, &backup.Path, &backup.BackupPath, &backup.Hash,
			&backup.Size, &backup.CreatedAt); err != nil {
			return nil, fmt.Errorf("扫描备份: %w", err)
		}
		result = append(result, backup)
	}
	return result, rows.Err()
}
