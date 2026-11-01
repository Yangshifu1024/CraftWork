import { describe, expect, it } from "vitest";
import { collectPastedFiles, isSupportedImage, pastedFileName, readFileAsBase64 } from "./paste";

/** 构造测试用文件。 */
function file(name: string, content = "hello", type = "text/plain"): File {
  return new File([content], name, { type });
}

/** 构造一个只实现粘贴所需字段的 DataTransferItem。 */
function item(source: File | null): DataTransferItem {
  return { kind: "file", getAsFile: () => source } as unknown as DataTransferItem;
}

describe("粘贴附件工具", () => {
  it("优先从 items 取文件并跳过无效项", () => {
    const files = collectPastedFiles({
      items: [item(file("a.txt")), item(null), item(file("b.txt"))],
      files: [file("fallback.txt")],
    });
    expect(files.map((entry) => entry.name)).toEqual(["a.txt", "b.txt"]);
  });

  it("items 为空时回退到 files", () => {
    const files = collectPastedFiles({
      items: [],
      files: [file("a.txt"), file("b.txt")],
    });
    expect(files.map((entry) => entry.name)).toEqual(["a.txt", "b.txt"]);
  });

  it("纯文本粘贴没有文件时返回空数组", () => {
    expect(collectPastedFiles(null)).toEqual([]);
    expect(collectPastedFiles({})).toEqual([]);
    expect(collectPastedFiles({ items: [item(null)] })).toEqual([]);
  });

  it("仅在受支持的图片扩展名上返回 true", () => {
    expect(isSupportedImage(file("pic.PNG"))).toBe(true);
    expect(isSupportedImage(file("photo.jpeg"))).toBe(true);
    expect(isSupportedImage(file("vector.svg"))).toBe(false);
    expect(isSupportedImage(file("README"))).toBe(false);
  });

  it("保留浏览器给出的文件名，空名字按类型补默认名", () => {
    expect(pastedFileName(file("note.txt"))).toBe("note.txt");
    expect(pastedFileName(file("", "img", "image/png"))).toMatch(/^pasted-\d{8}-\d{6}\.png$/);
    expect(pastedFileName(file("", "raw", ""))).toMatch(/^pasted-\d{8}-\d{6}\.bin$/);
  });

  it("把文件读成不含 data URL 前缀的 base64", async () => {
    await expect(readFileAsBase64(file("a.txt", "hello"))).resolves.toBe("aGVsbG8=");
  });
});
