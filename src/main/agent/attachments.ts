// 附件加载：文本/图片两类，越界与敏感名拦截，尺寸与总量限制。
// 被跳过的附件保留在结果里并带说明，让界面能解释为什么没有发送。
import { readFileSync, statSync } from "node:fs";
import { extname } from "node:path";
import { isSensitiveAttachment } from "../policy/attachment";
import { resolveExisting } from "../policy/resolve";
import type { Attachment } from "../model/types";

export const TEXT_LIMIT = 1024 * 1024; // 单文本 1MB
export const IMAGE_LIMIT = 5 * 1024 * 1024; // 单图 5MB
export const TOTAL_LIMIT = 12 * 1024 * 1024; // 总量 12MB
export const MAX_ATTACHMENTS = 10;

const TEXT_EXTS = new Set([
  "txt", "md", "markdown", "go", "js", "jsx", "ts", "tsx", "json", "yaml", "yml", "toml", "csv", "tsv",
  "log", "py", "rb", "rs", "java", "kt", "c", "h", "cc", "cpp", "hpp", "cs", "php", "swift",
  "sh", "bash", "zsh", "sql", "html", "htm", "css", "scss", "xml", "ini", "cfg", "conf",
]);

const IMAGE_MIME: Record<string, string> = {
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".webp": "image/webp",
};

export interface LoadResult {
  attachments: Attachment[];
  totalBytes: number;
}

/**
 * 解析并读取附件列表：
 * - 数量截断（超出忽略）
 * - 敏感文件名拦截（密钥/环境文件不发送给模型）
 * - 路径解析：先按附件目录根，回落工作区根（两处都做越界校验；附件目录由应用管理，不受工作区限制）
 */
export function loadAttachments(workspace: string, attachmentDir: string, paths: string[]): LoadResult {
  const list = paths.slice(0, MAX_ATTACHMENTS);
  const attachments: Attachment[] = [];
  let total = 0;

  for (const raw of list) {
    const name = raw.split("/").pop() ?? raw;
    if (isSensitiveAttachment(name)) {
      attachments.push({ name, mime: "", text: "", data: "", size: 0, skipped: "密钥或环境文件不会发送给模型" });
      continue;
    }
    let abs = "";
    try {
      abs = resolveExisting([attachmentDir, workspace].filter((r) => r), raw).abs;
    } catch {
      attachments.push({ name, mime: "", text: "", data: "", size: 0, skipped: "路径无法解析或超出可读范围" });
      continue;
    }
    let st;
    try {
      st = statSync(abs);
    } catch {
      attachments.push({ name, mime: "", text: "", data: "", size: 0, skipped: "文件不存在" });
      continue;
    }
    if (st.isDirectory()) {
      attachments.push({ name, mime: "", text: "", data: "", size: 0, skipped: "文件夹不能作为附件" });
      continue;
    }
    const ext = extname(abs).toLowerCase();
    const mime = IMAGE_MIME[ext];
    if (mime) {
      if (st.size > IMAGE_LIMIT || total + st.size > TOTAL_LIMIT) {
        attachments.push({ name, mime, text: "", data: "", size: st.size, skipped: "图片超过大小限制（单图 5MB / 总量 12MB）" });
        continue;
      }
      const data = readFileSync(abs).toString("base64");
      total += st.size;
      attachments.push({ name, mime, text: "", data, size: st.size, skipped: "" });
      continue;
    }
    if (!TEXT_EXTS.has(ext.slice(1))) {
      attachments.push({ name, mime: "", text: "", data: "", size: st.size, skipped: "暂不支持该附件类型（支持文本与 png/jpg/gif/webp 图片）" });
      continue;
    }
    if (st.size > TEXT_LIMIT || total + st.size > TOTAL_LIMIT) {
      attachments.push({ name, mime: "", text: "", data: "", size: st.size, skipped: "文件超过大小限制（单文件 1MB / 总量 12MB）" });
      continue;
    }
    const raw2 = readFileSync(abs);
    if (raw2.includes(0)) {
      attachments.push({ name, mime: "", text: "", data: "", size: st.size, skipped: "看起来是二进制文件，无法作为文本读取" });
      continue;
    }
    total += st.size;
    attachments.push({ name, mime: "text/plain", text: raw2.toString("utf8"), data: "", size: st.size, skipped: "" });
  }
  return { attachments, totalBytes: total };
}
