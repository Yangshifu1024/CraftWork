// 粘贴附件收集（纯逻辑，无 React/DOM 依赖，便于将来 vitest 直测）：
// 从剪贴板文件列表选取可上传文件并生成存储名。总量限制对齐 Agent 侧附件上限（12MB）。
// FileReader 转 base64 是浏览器 API，单独导出供组件调用。

/** 粘贴附件总量上限（字节），对齐 craft-agent 附件加载总量。 */
export const MAX_PASTE_TOTAL_BYTES = 12 * 1024 * 1024;

const EXT_BY_MIME: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
  "image/svg+xml": "svg",
};

/** 图片 MIME → 扩展名（无匹配返回空串）。 */
export function mimeToExt(mime: string): string {
  return EXT_BY_MIME[mime] ?? "";
}

/** 是否可内联预览的图片类型。 */
export function isImageName(name: string): boolean {
  return /\.(png|jpe?g|gif|webp|svg)$/i.test(name);
}

/** 按总量上限选取文件：超出后丢弃后续（整批语义由调用方提示）。 */
export function selectPastedFiles(files: ArrayLike<File>, maxTotal = MAX_PASTE_TOTAL_BYTES): File[] {
  const out: File[] = [];
  let total = 0;
  for (const file of Array.from(files)) {
    if (file.size <= 0 || total + file.size > maxTotal) continue;
    total += file.size;
    out.push(file);
  }
  return out;
}

/** 粘贴内容的存储名：剪贴板截图常为无名 "image.png"，按 MIME 推导可读名。 */
export function pastedFileName(file: File): string {
  const name = file.name.trim();
  if (name && name.toLowerCase() !== "image.png") return name;
  const ext = mimeToExt(file.type);
  if (ext) return `粘贴图像.${ext}`;
  return `粘贴文件-${Date.now()}`;
}

/** File → 纯 base64（去 data URL 前缀）。 */
export function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = String(reader.result ?? "");
      const comma = result.indexOf(",");
      resolve(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.onerror = () => reject(reader.error ?? new Error("读取文件失败"));
    reader.readAsDataURL(file);
  });
}
