// 审计 / 用量 / 备份 读取与恢复（恢复是唯一读备份回写的路径，且恢复动作本身落审计）。
import { existsSync, readFileSync, statSync } from "node:fs";
import { mkdirSync } from "node:fs";
import { dirname } from "node:path";
import type { AuditEntry, Backup, UsageSummary } from "../../shared/types";
import { sha256Hex, writeFileAtomic } from "../core/platform";
import type { DB } from "../store/db";
import { addAudit, getBackup, listAudit, listBackups, usageSummary } from "../store/records";

export class AuditService {
  constructor(private readonly db: DB) {}

  list(limit: number): AuditEntry[] {
    return listAudit(this.db, limit);
  }

  usage(): UsageSummary {
    return usageSummary(this.db);
  }

  backups(limit: number): Backup[] {
    return listBackups(this.db, limit);
  }

  /** 恢复备份：读备份文件 → 保留原权限 → 原子写回 → 落审计。 */
  restore(backupId: string): void {
    const backup = getBackup(this.db, backupId);
    if (!existsSync(backup.backupPath)) throw new Error("备份文件不存在或已被清理");
    const data = readFileSync(backup.backupPath);
    let mode = 0o644;
    try {
      mode = statSync(backup.path).mode & 0o777;
    } catch {
      // 目标不存在用默认权限
    }
    mkdirSync(dirname(backup.path), { recursive: true });
    writeFileAtomic(backup.path, data, mode);
    if (sha256Hex(data) !== backup.hash) {
      // 写入成功但哈希不一致：如实告知（不回滚，备份仍在）
      addAudit(this.db, { tool: "backup_restore", risk: "R1", decision: "allow", scope: "user", detail: backup.path });
      throw new Error("恢复完成，但备份校验值不一致，请检查目标文件");
    }
    addAudit(this.db, { tool: "backup_restore", risk: "R1", decision: "allow", scope: "user", detail: backup.path });
  }
}
