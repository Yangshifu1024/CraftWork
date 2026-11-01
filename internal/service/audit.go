package service

import (
	"fmt"
	"os"
	"path/filepath"

	"craftwork/internal/platform"
	"craftwork/internal/policy"
	"craftwork/internal/store"
)

// AuditService 读取审计、用量与备份，并提供备份恢复。
type AuditService struct {
	db *store.DB
}

// NewAuditService 创建审计服务。
func NewAuditService(db *store.DB) *AuditService {
	return &AuditService{db: db}
}

// List 返回最近的审计记录。
func (s *AuditService) List(limit int) ([]store.AuditEntry, error) {
	return s.db.ListAudit(limit)
}

// Usage 返回用量聚合。
func (s *AuditService) Usage() (*store.UsageSummary, error) {
	return s.db.UsageSummary()
}

// Backups 返回最近的备份记录。
func (s *AuditService) Backups(limit int) ([]store.Backup, error) {
	return s.db.ListBackups(limit)
}

// Restore 用备份覆盖原文件。调用前前端必须让用户确认。
func (s *AuditService) Restore(backupID string) error {
	backup, err := s.db.GetBackup(backupID)
	if err != nil {
		return err
	}
	data, err := os.ReadFile(backup.BackupPath)
	if err != nil {
		return fmt.Errorf("读取备份文件: %w", err)
	}
	mode := os.FileMode(0o644)
	if info, statErr := os.Stat(backup.Path); statErr == nil {
		mode = info.Mode().Perm()
	}
	if err := os.MkdirAll(filepath.Dir(backup.Path), 0o755); err != nil {
		return fmt.Errorf("创建目标目录: %w", err)
	}
	if err := platform.WriteFileAtomic(backup.Path, data, mode); err != nil {
		return err
	}
	return s.db.AddAudit(&store.AuditEntry{
		Tool:     "backup_restore",
		Risk:     policy.R1.String(),
		Decision: "allow",
		Scope:    "user",
		Detail:   backup.Path,
	})
}
