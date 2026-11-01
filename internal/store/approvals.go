// 本文件负责审批行的持久化：创建等待中的审批、记录用户决定、查询单条或线程内的
// 待审批，以及在线程取消时批量关闭未决审批。
//
// 审批行的 decision 为空表示仍在等待；一旦写入决定就不再允许修改。

package store

import (
	"database/sql"
	"errors"
	"fmt"
)

// CreateApproval 写入一条等待中的审批。
func (db *DB) CreateApproval(approval *Approval) error {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()
	if _, err := db.sqlDB.Exec(
		`INSERT INTO approvals (id, thread_id, run_id, tool_call_id, tool, risk, requested_at)
		 VALUES (?, ?, ?, ?, ?, ?, ?)`,
		approval.ID, approval.ThreadID, approval.RunID, approval.ToolCallID,
		approval.Tool, approval.Risk, approval.RequestedAt); err != nil {
		return fmt.Errorf("创建审批: %w", err)
	}
	return nil
}

// ResolveApproval 记录审批决定。只有等待中的审批可以被决定。
//
// WHERE 条件里的 decision IS NULL 是并发保护：重复提交同一个审批时，
// 第二次更新不会命中任何行，从而返回 ErrNotFound。
func (db *DB) ResolveApproval(id, decision, scope string) error {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()
	result, err := db.sqlDB.Exec(
		`UPDATE approvals SET decision = ?, scope = ?, decided_at = ?
		 WHERE id = ? AND decision IS NULL`, decision, scope, nowMillis(), id)
	if err != nil {
		return fmt.Errorf("记录审批决定: %w", err)
	}
	affected, _ := result.RowsAffected()
	if affected == 0 {
		return ErrNotFound
	}
	return nil
}

// GetApproval 读取单条审批。
//
// 可空列用 COALESCE 转成空字符串或 0，调用方无需处理 sql.NullString。
func (db *DB) GetApproval(id string) (*Approval, error) {
	var approval Approval
	err := db.sqlDB.QueryRow(
		`SELECT id, thread_id, run_id, tool_call_id, tool, risk,
		        COALESCE(decision, ''), COALESCE(scope, ''), requested_at, COALESCE(decided_at, 0)
		 FROM approvals WHERE id = ?`, id).
		Scan(&approval.ID, &approval.ThreadID, &approval.RunID, &approval.ToolCallID,
			&approval.Tool, &approval.Risk, &approval.Decision, &approval.Scope,
			&approval.RequestedAt, &approval.DecidedAt)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, fmt.Errorf("读取审批: %w", err)
	}
	return &approval, nil
}

// PendingApprovals 返回线程内等待中的审批。
func (db *DB) PendingApprovals(threadID string) ([]Approval, error) {
	rows, err := db.sqlDB.Query(
		`SELECT id, thread_id, run_id, tool_call_id, tool, risk,
		        COALESCE(decision, ''), COALESCE(scope, ''), requested_at, COALESCE(decided_at, 0)
		 FROM approvals WHERE thread_id = ? AND decision IS NULL ORDER BY requested_at`, threadID)
	if err != nil {
		return nil, fmt.Errorf("读取待审批: %w", err)
	}
	defer rows.Close()

	var result []Approval
	for rows.Next() {
		var approval Approval
		if err := rows.Scan(&approval.ID, &approval.ThreadID, &approval.RunID, &approval.ToolCallID,
			&approval.Tool, &approval.Risk, &approval.Decision, &approval.Scope,
			&approval.RequestedAt, &approval.DecidedAt); err != nil {
			return nil, fmt.Errorf("扫描审批: %w", err)
		}
		result = append(result, approval)
	}
	return result, rows.Err()
}

// CancelThreadApprovals 取消线程内所有等待中的审批。
func (db *DB) CancelThreadApprovals(threadID string) error {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()
	if _, err := db.sqlDB.Exec(
		`UPDATE approvals SET decision = 'canceled', decided_at = ?
		 WHERE thread_id = ? AND decision IS NULL`, nowMillis(), threadID); err != nil {
		return fmt.Errorf("取消线程审批: %w", err)
	}
	return nil
}
