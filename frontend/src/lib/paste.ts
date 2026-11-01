/**
 * 粘贴附件的浏览器侧工具。
 *
 * 输入框捕获 paste 事件后，这里负责从剪贴板取出文件、补齐文件名，并把
 * 内容转成 base64 交给 Go 落盘。WebView 不会把剪贴板文件暴露成本地路径，
 * 所以前端只能传字节，最终路径由 Go 在附件目录中生成。
 */

/** 与 Go 侧 maxPastedAttachmentBytes 保持一致，超过后不发送到后端。 */
export const MAX_PASTED_ATTACHMENT_BYTES = 12 * 1024 * 1024;

/** Go 侧按扩展名识别的图片类型；其余图片会在发送时被跳过。 */
const supportedImageExtensions = new Set([".png", ".jpg", ".jpeg", ".gif", ".webp"]);

/** 图片 MIME 到扩展名的映射，只在剪贴板没有文件名时用于生成默认名。 */
const imageExtensionByMIME: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
};

/**
 * 从剪贴板数据中取出文件。
 *
 * `items` 优先，因为多数 WebView 用它表示粘贴的文件并保持顺序；只有 items
 * 为空时才回退到 `files`，避免同一个文件被加入两次。纯文本粘贴没有文件，
 * 返回空数组，调用方应继续走默认的文本粘贴行为。
 */
export function collectPastedFiles(
  source: { files?: ArrayLike<File> | null; items?: ArrayLike<DataTransferItem> | null } | null | undefined,
): File[] {
  if (!source) {
    return [];
  }
  const files: File[] = [];
  const items = source.items;
  if (items) {
    for (let index = 0; index < items.length; index += 1) {
      const item = items[index];
      if (item?.kind !== "file") {
        continue;
      }
      const file = item.getAsFile();
      if (file) {
        files.push(file);
      }
    }
  }
  if (files.length === 0 && source.files) {
    for (let index = 0; index < source.files.length; index += 1) {
      files.push(source.files[index]);
    }
  }
  return files;
}

/** 判断文件扩展名是否属于 Go 侧支持的图片类型。 */
export function isSupportedImage(file: File): boolean {
  return supportedImageExtensions.has(fileExtension(file.name));
}

/**
 * 生成可读的附件文件名。
 *
 * 粘贴截图时浏览器经常给出空名字，这里按 MIME 补一个带时间戳的默认名；
 * 无法识别类型时使用 .bin，发送阶段会按不支持的附件类型提示用户。
 */
export function pastedFileName(file: File): string {
  const name = file.name?.trim();
  if (name) {
    return name;
  }
  const extension = imageExtensionByMIME[file.type] ?? "bin";
  return `pasted-${timestamp()}.${extension}`;
}

/** 把文件读成 base64；返回的内容不含 data URL 前缀，可直接交给 Go 解码。 */
export function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("读取文件失败"));
    reader.onload = () => {
      const result = typeof reader.result === "string" ? reader.result : "";
      const comma = result.indexOf(",");
      resolve(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.readAsDataURL(file);
  });
}

/** 取小写扩展名；没有扩展名或只有开头的点时返回空字符串。 */
function fileExtension(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot).toLowerCase() : "";
}

/** 生成本地时间戳，例如 20261101-203045，用于截图类默认文件名。 */
function timestamp(): string {
  const now = new Date();
  const pad = (value: number) => String(value).padStart(2, "0");
  return [
    now.getFullYear(),
    pad(now.getMonth() + 1),
    pad(now.getDate()),
    "-",
    pad(now.getHours()),
    pad(now.getMinutes()),
    pad(now.getSeconds()),
  ].join("");
}
