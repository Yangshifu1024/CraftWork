// 应用级服务：信息、外链、文件选择、剪贴板、附件存取、窗口控制。
// 外链是唯一出口：仅 http/https 且有 host，交由主进程确认后打开系统浏览器。
import { mkdirSync, readFileSync, writeFileSync, existsSync, statSync, unlinkSync } from "node:fs";
import { join } from "node:path";
import { shell, clipboard, BrowserWindow, dialog } from "electron";
import type { AppInfo, AttachmentContent } from "../../shared/types";
import { isSensitiveAttachment } from "../policy/attachment";
import { newId, IdPrefix } from "../../shared/ids";

const MAX_ATTACHMENT_BYTES = 12 * 1024 * 1024;
const IMAGE_MIME: Record<string, string> = {
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".webp": "image/webp",
  ".bmp": "image/bmp",
};

export class AppService {
  constructor(
    private readonly version: string,
    private readonly dataDir: string,
  ) {}

  get attachmentsDir(): string {
    return join(this.dataDir, "attachments");
  }

  info(): AppInfo {
    return {
      name: "CraftWork",
      version: this.version,
      platform: process.platform,
      arch: process.arch,
      dataDir: this.dataDir,
    };
  }

  /** 外链唯一出口：仅 http/https 且有 host。 */
  async openExternal(rawURL: string): Promise<void> {
    let url: URL;
    try {
      url = new URL(rawURL);
    } catch {
      throw new Error("无效的链接");
    }
    if ((url.protocol !== "http:" && url.protocol !== "https:") || !url.hostname) {
      throw new Error("只允许打开 http/https 链接");
    }
    await shell.openExternal(url.toString());
  }

  async pickFiles(): Promise<string[]> {
    const win = BrowserWindow.getFocusedWindow() ?? BrowserWindow.getAllWindows()[0];
    if (!win) return [];
    const result = await dialog.showOpenDialog(win, {
      properties: ["openFile", "multiSelections"],
    });
    return result.canceled ? [] : result.filePaths;
  }

  async pickFolder(): Promise<string> {
    const win = BrowserWindow.getFocusedWindow() ?? BrowserWindow.getAllWindows()[0];
    if (!win) return "";
    const result = await dialog.showOpenDialog(win, {
      properties: ["openDirectory", "createDirectory"],
    });
    return result.canceled ? "" : (result.filePaths[0] ?? "");
  }

  /** 保存粘贴的附件（base64）：大小/敏感名/名称清洗后 O_EXCL 写入附件目录。 */
  savePastedAttachment(name: string, base64: string): string {
    const buffer = Buffer.from(base64, "base64");
    if (buffer.length === 0) throw new Error("附件内容为空");
    if (buffer.length > MAX_ATTACHMENT_BYTES) throw new Error("附件超过 12MB 上限");
    if (isSensitiveAttachment(name)) throw new Error("密钥或环境文件不会保存为附件");
    const dir = this.attachmentsDir;
    mkdirSync(dir, { recursive: true, mode: 0o700 });
    const clean = sanitizeAttachmentName(name);
    const path = join(dir, `${newId(IdPrefix.Attachment)}_${clean}`);
    try {
      writeFileSync(path, buffer, { flag: "wx", mode: 0o600 });
    } catch {
      throw new Error("附件保存失败");
    }
    return path;
  }

  /** 读取附件（白名单图片 MIME；返回 base64，不给渲染进程文件路径）。 */
  readAttachment(path: string): AttachmentContent {
    const ext = path.slice(path.lastIndexOf(".")).toLowerCase();
    const mime = IMAGE_MIME[ext];
    if (!mime) throw new Error("暂不支持该附件类型");
    const st = statSync(path);
    if (st.isDirectory()) throw new Error("目标是文件夹");
    if (st.size > MAX_ATTACHMENT_BYTES) throw new Error("附件超过 12MB 上限");
    if (!existsSync(path)) throw new Error("附件不存在");
    return { base64: readFileSync(path).toString("base64"), mime };
  }

  clipboardWrite(text: string): boolean {
    try {
      clipboard.writeText(text);
      return true;
    } catch {
      return false;
    }
  }

  async clipboardRead(): Promise<string> {
    try {
      return await clipboard.readText();
    } catch {
      return "";
    }
  }

  showInFolder(path: string): void {
    if (!existsSync(path)) throw new Error("路径不存在");
    void shell.showItemInFolder(path);
  }

  minimiseWindow(): void {
    BrowserWindow.getFocusedWindow()?.minimize();
  }

  toggleMaximiseWindow(): void {
    const win = BrowserWindow.getFocusedWindow();
    if (!win) return;
    if (win.isMaximized()) win.unmaximize();
    else win.maximize();
  }

  closeWindow(): void {
    BrowserWindow.getFocusedWindow()?.close();
  }
}

/** 清洗附件文件名：保留安全字符；超长按字符截断；空回退 pasted-file。 */
function sanitizeAttachmentName(name: string): string {
  const base = (name.split("\\").pop() ?? name).split("/").pop() ?? "pasted-file";
  let cleaned = base.replace(/[^a-zA-Z0-9._\- ]/g, "_").replace(/^\.+/, "");
  if (!cleaned) cleaned = "pasted-file";
  const dot = cleaned.lastIndexOf(".");
  if (dot >= 0 && cleaned.length - dot - 1 > 16) {
    cleaned = cleaned.slice(0, dot);
  }
  if ([...cleaned].length > 96) {
    cleaned = [...cleaned].slice(0, 96).join("");
  }
  return cleaned;
}

// unlinkSync 供附件清理扩展使用（当前生命周期内附件随目录保留）
void unlinkSync;
