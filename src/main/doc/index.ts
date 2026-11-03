// 文档读写：Word（docx）/ Excel（xlsx）/ PDF 的文本级处理。
// 限制：docx 只读正文；xlsx 不计算公式（保留公式原文）；pdf 只读文本层、不支持 OCR。
// 编辑语义：XML 转义后的字节级替换（恰好一次）；xlsx 用正则定点替换保留原结构与样式。
import { readFileSync, renameSync, chmodSync, unlinkSync, writeFileSync, openSync, writeSync, closeSync } from "node:fs";
import { dirname, extname } from "node:path";
import { inflateSync } from "node:zlib";
import JSZip from "jszip";

export const MAX_FILE_BYTES = 50 * 1024 * 1024;
export const MAX_TEXT_BYTES = 2 * 1024 * 1024;

export interface DocResult {
  kind: string;
  text: string;
  note: string;
  truncated: boolean;
}

export interface DocEdit {
  kind: "text" | "cell";
  old?: string;
  new?: string;
  sheet?: string;
  cell?: string;
  value?: string;
}

export class ErrReadOnly extends Error {
  constructor() {
    super("该格式不支持编辑");
    this.name = "ErrReadOnly";
  }
}

const EXT_KIND: Record<string, string> = {
  ".docx": "docx",
  ".xlsx": "xlsx",
  ".pdf": "pdf",
};

export function docKind(path: string): string {
  return EXT_KIND[extname(path).toLowerCase()] ?? "";
}

export function docSupported(path: string): boolean {
  return docKind(path) !== "";
}

export async function docReadFile(path: string): Promise<DocResult> {
  const kind = docKind(path);
  if (!kind) throw new Error("不支持的文档格式");
  const raw = readFileSync(path);
  if (raw.length > MAX_FILE_BYTES) throw new Error("文档过大（超过 50MB 上限）");
  if (kind === "docx") return readDocx(raw);
  if (kind === "xlsx") return readXlsx(raw);
  return readPdf(raw);
}

export async function docReplace(path: string, edit: DocEdit): Promise<void> {
  const kind = docKind(path);
  if (kind === "docx") return replaceDocx(path, edit);
  if (kind === "xlsx") return replaceXlsx(path, edit);
  throw new ErrReadOnly();
}

// ---------- 通用：重写 zip（复制其余条目，替换目标条目） ----------

async function rewriteZip(
  raw: Buffer,
  path: string,
  replace: (name: string) => Promise<string | null>,
): Promise<void> {
  const zip = await JSZip.loadAsync(raw);
  const out = new JSZip();
  const names = Object.keys(zip.files);
  for (const name of names) {
    const entry = zip.files[name]!;
    if (entry.dir) {
      out.folder(name);
      continue;
    }
    let content: string | null = null;
    try {
      content = await replace(name);
    } catch {
      content = null;
    }
    const data = content ?? (await entry.async("nodebuffer"));
    out.file(name, data, {
      compression: entry.options.compression === "DEFLATE" ? "DEFLATE" : "STORE",
      date: entry.date,
      unixPermissions: entry.unixPermissions ?? undefined,
    });
  }
  const buf = await out.generateAsync({ type: "nodebuffer", compression: "DEFLATE" });
  atomicWrite(path, buf);
}

function atomicWrite(path: string, data: Buffer): void {
  const tmp = `${dirname(path)}/.craftwork-doc-${process.pid}-${Date.now()}.tmp`;
  const fd = openSync(tmp, "wx", 0o644);
  try {
    writeSync(fd, data);
  } finally {
    closeSync(fd);
  }
  try {
    chmodSync(tmp, 0o644);
  } catch {
    // 忽略
  }
  try {
    renameSync(tmp, path);
  } catch (err) {
    try {
      unlinkSync(tmp);
    } catch {
      // 忽略
    }
    throw err;
  }
}

// ---------- XML 工具 ----------

export function xmlEscape(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

function xmlUnescape(text: string): string {
  return text
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&#(\d+);/g, (_, d) => String.fromCodePoint(Number(d)))
    .replace(/&#x([0-9a-fA-F]+);/g, (_, h) => String.fromCodePoint(parseInt(h, 16)))
    .replace(/&amp;/g, "&");
}

// ---------- docx ----------

/**
 * 读取 docx 正文：word/document.xml 流式扫描。
 * <w:p> 段落成行；表格 <w:tbl>/<w:tr>/<w:tc>（单元格内换行连接，单元格间 tab 连接）；
 * <w:t> 文本、<w:tab/>→\t、<w:br/>→\n。页眉页脚批注不在范围。
 */
async function readDocx(raw: Buffer): Promise<DocResult> {
  const zip = await JSZip.loadAsync(raw);
  const entry = zip.files["word/document.xml"];
  if (!entry) return { kind: "docx", text: "", note: "文档缺少正文内容。", truncated: false };
  const xml = await entry.async("string");
  const lines: string[] = [];
  let i = 0;
  const n = xml.length;

  const findTagEnd = (from: number): number => xml.indexOf(">", from);


  while (i < n) {
    const lt = xml.indexOf("<", i);
    if (lt < 0) break;
    if (xml.startsWith("<w:p ", lt) || xml.startsWith("<w:p>", lt)) {
      // 解析一个段落
      let j = lt;
      const lineParts: string[] = [];
      // 段落深度处理嵌套（表格里也有 w:p）——简单策略：找配对的 </w:p>，逐层跳过内部 <w:p...
      // 实际 OOXML 中 w:p 不嵌套 w:p（w:p 内是 runs），直接找最近的 </w:p> 即可
      let depth = 0;
      for (;;) {
        const nextOpen = xml.indexOf("<", j + 1);
        const nextClose = xml.indexOf("</w:p>", j + 1);
        if (nextClose < 0) {
          j = n;
          break;
        }
        if (nextOpen >= 0 && nextOpen < nextClose) {
          if (xml.startsWith("<w:p", nextOpen)) depth++;
          j = findTagEnd(nextOpen);
          if (j < 0) {
            j = n;
            break;
          }
        } else {
          if (depth === 0) {
            // 段落体范围 [lt, nextClose)
            const body = xml.slice(lt, nextClose);
            lineParts.push(extractParagraphText(body));
            j = nextClose + 5;
            break;
          }
          depth--;
          j = nextClose + 5;
        }
      }
      lines.push(lineParts.join(""));
      i = j;
      continue;
    }
    if (xml.startsWith("<w:tbl", lt)) {
      // 解析表格：找配对的 </w:tbl>
      const tableEnd = findBalanced(xml, lt, "<w:tbl", "</w:tbl>");
      if (tableEnd < 0) break;
      const body = xml.slice(lt, tableEnd);
      lines.push(...extractTable(body));
      i = tableEnd;
      continue;
    }
    i = findTagEnd(lt) + 1;
    if (i <= lt) break;
  }

  let text = lines.join("\n");
  let truncated = false;
  if (Buffer.byteLength(text, "utf8") > MAX_TEXT_BYTES) {
    text = `${Buffer.from(text, "utf8").subarray(0, MAX_TEXT_BYTES).toString("utf8")}\n…（文本已截断）`;
    truncated = true;
  }
  return { kind: "docx", text, note: "", truncated };
}

/** 提取一个 <w:p>...</w:p> 体内的可见文本。 */
function extractParagraphText(body: string): string {
  let out = "";
  let i = 0;
  for (;;) {
    const open = body.indexOf("<", i);
    if (open < 0) break;
    if (body.startsWith("<w:t", open)) {
      const close = body.indexOf("</w:t>", open);
      if (close < 0) break;
      const tagEnd = body.indexOf(">", open);
      const isOpen = body[tagEnd - 1] !== "/";
      if (isOpen) {
        out += xmlUnescape(body.slice(tagEnd + 1, close));
        i = close + 6;
      } else {
        i = tagEnd + 1;
      }
      continue;
    }
    if (body.startsWith("<w:tab", open)) {
      out += "\t";
    } else if (body.startsWith("<w:br", open)) {
      out += "\n";
    }
    const end = body.indexOf(">", open);
    if (end < 0) break;
    i = end + 1;
  }
  return out;
}

/** 找配对的闭合标签（处理同名嵌套），返回闭合结束位置。 */
function findBalanced(xml: string, from: number, openTag: string, closeTag: string): number {
  let depth = 0;
  let i = from;
  for (;;) {
    const open = xml.indexOf(openTag, i);
    const close = xml.indexOf(closeTag, i);
    if (close < 0) return -1;
    const openIsReal = open >= 0 && open < close && xml[open + openTag.length] !== "/";
    if (openIsReal) {
      depth++;
      i = open + openTag.length;
    } else {
      depth--;
      i = close + closeTag.length;
      if (depth === 0) return i;
    }
  }
}

/** 表格提取：行内单元格多段落 \n 连接，单元格间 \t 连接。 */
function extractTable(body: string): string[] {
  const lines: string[] = [];
  let i = 0;
  for (;;) {
    const trOpen = body.indexOf("<w:tr", i);
    if (trOpen < 0) break;
    const trEnd = findBalanced(body, trOpen, "<w:tr", "</w:tr>");
    if (trEnd < 0) break;
    const trBody = body.slice(trOpen, trEnd);
    const cells: string[] = [];
    let j = 0;
    for (;;) {
      const tcOpen = trBody.indexOf("<w:tc", j);
      if (tcOpen < 0) break;
      const tcEnd = findBalanced(trBody, tcOpen, "<w:tc", "</w:tc>");
      if (tcEnd < 0) break;
      const tcBody = trBody.slice(tcOpen, tcEnd);
      const paragraphs: string[] = [];
      let k = 0;
      for (;;) {
        const pOpen = tcBody.indexOf("<w:p", k);
        if (pOpen < 0) break;
        const pEnd = tcBody.indexOf("</w:p>", pOpen);
        if (pEnd < 0) break;
        paragraphs.push(extractParagraphText(tcBody.slice(pOpen, pEnd)));
        k = pEnd + 6;
      }
      cells.push(paragraphs.join("\n"));
      j = tcEnd;
    }
    lines.push(cells.join("\t"));
    i = trEnd;
  }
  return lines;
}

async function replaceDocx(path: string, edit: DocEdit): Promise<void> {
  if (edit.kind !== "text" || !edit.old) throw new Error("docx 只支持文本替换");
  const raw = readFileSync(path);
  const zip = await JSZip.loadAsync(raw);
  const entry = zip.files["word/document.xml"];
  if (!entry) throw new Error("文档缺少正文内容");
  const xml = await entry.async("string");
  const oldEscaped = xmlEscape(edit.old);
  const newEscaped = xmlEscape(edit.new ?? "");
  const count = xml.split(oldEscaped).length - 1;
  if (count === 0) {
    throw new Error("没有找到要替换的文本；文本可能被拆分到多个格式片段中，请调整原文");
  }
  if (count > 1) throw new Error(`找到 ${count} 处相同文本，请提供更长的唯一片段`);
  const updated = xml.replace(oldEscaped, newEscaped);
  await rewriteZip(raw, path, async (name) => (name === "word/document.xml" ? updated : null));
}

// ---------- xlsx ----------

interface XlsxSheet {
  name: string;
  path: string;
}

function colToNumber(col: string): number {
  let n = 0;
  for (const ch of col) {
    n = n * 26 + (ch.toUpperCase().charCodeAt(0) - 64);
  }
  return n;
}

function numberToCol(n: number): string {
  let s = "";
  while (n > 0) {
    const rem = (n - 1) % 26;
    s = String.fromCharCode(65 + rem) + s;
    n = Math.floor((n - 1) / 26);
  }
  return s;
}

async function loadXlsx(raw: Buffer): Promise<{
  zip: JSZip;
  workbookXml: string;
  relsXml: string;
  shared: string[];
  sheets: XlsxSheet[];
}> {
  const zip = await JSZip.loadAsync(raw);
  const workbookXml = (await zip.files["xl/workbook.xml"]?.async("string")) ?? "";
  const relsXml = (await zip.files["xl/_rels/workbook.xml.rels"]?.async("string")) ?? "";
  const sharedXml = (await zip.files["xl/sharedStrings.xml"]?.async("string")) ?? "";

  const shared: string[] = [];
  if (sharedXml) {
    const re = /<si>([\s\S]*?)<\/si>/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(sharedXml)) !== null) {
      const si = m[1] ?? "";
      const texts: string[] = [];
      const tre = /<t[^>]*>([\s\S]*?)<\/t>/g;
      let tm: RegExpExecArray | null;
      while ((tm = tre.exec(si)) !== null) texts.push(xmlUnescape(tm[1] ?? ""));
      shared.push(texts.join(""));
    }
  }

  // sheet 名 → r:id → Target 路径
  const relMap = new Map<string, string>();
  const relRe = /<Relationship[^>]*Id="([^"]+)"[^>]*Target="([^"]+)"[^>]*\/?>/g;
  let rm: RegExpExecArray | null;
  while ((rm = relRe.exec(relsXml)) !== null) {
    let target = rm[2] ?? "";
    if (target.startsWith("/")) target = target.slice(1);
    else if (!target.startsWith("xl/")) target = `xl/${target}`;
    relMap.set(rm[1] ?? "", target);
  }
  const sheets: XlsxSheet[] = [];
  const sheetRe = /<sheet[^>]*name="([^"]+)"[^>]*r:id="([^"]+)"[^>]*\/?>/g;
  let sm: RegExpExecArray | null;
  while ((sm = sheetRe.exec(workbookXml)) !== null) {
    const target = relMap.get(sm[2] ?? "");
    if (target) sheets.push({ name: xmlUnescape(sm[1] ?? ""), path: target });
  }
  return { zip, workbookXml, relsXml, shared, sheets };
}

async function readXlsx(raw: Buffer): Promise<DocResult> {
  const { zip, shared, sheets } = await loadXlsx(raw);
  if (sheets.length === 0) return { kind: "xlsx", text: "", note: "工作簿没有工作表。", truncated: false };
  const parts: string[] = [];
  for (const sheet of sheets) {
    const sheetXml = (await zip.files[sheet.path]?.async("string")) ?? "";
    parts.push(`工作表：${sheet.name}`);
    const rowRe = /<row[^>]*r="(\d+)"[^>]*>([\s\S]*?)<\/row>/g;
    interface Cell {
      col: number;
      text: string;
    }
    const rows: Array<{ num: number; cells: Cell[] }> = [];
    let rm: RegExpExecArray | null;
    while ((rm = rowRe.exec(sheetXml)) !== null) {
      const rowNum = Number(rm[1]);
      const body = rm[2] ?? "";
      const cells: Cell[] = [];
      const cellRe = /<c([^>]*)>([\s\S]*?)<\/c>|<c([^>]*)\/>/g;
      let cm: RegExpExecArray | null;
      while ((cm = cellRe.exec(body)) !== null) {
        const attrs = cm[1] ?? cm[3] ?? "";
        const inner = cm[2] ?? "";
        const refMatch = /r="([A-Z]+)(\d+)"/.exec(attrs);
        const col = refMatch ? colToNumber(refMatch[1] ?? "A") : cells.length + 1;
        const typeMatch = /t="([^"]+)"/.exec(attrs);
        const type = typeMatch?.[1] ?? "";
        let text = "";
        const formulaMatch = /<f[^>]*>([\s\S]*?)<\/f>/.exec(inner);
        if (formulaMatch) {
          text = `=${xmlUnescape(formulaMatch[1] ?? "")}`;
        } else if (type === "s") {
          const v = /<v[^>]*>([\s\S]*?)<\/v>/.exec(inner);
          const idx = Number(v?.[1] ?? -1);
          text = idx >= 0 && idx < shared.length ? (shared[idx] ?? "") : "";
        } else if (type === "inlineStr") {
          const texts: string[] = [];
          const tre = /<t[^>]*>([\s\S]*?)<\/t>/g;
          let tm: RegExpExecArray | null;
          while ((tm = tre.exec(inner)) !== null) texts.push(xmlUnescape(tm[1] ?? ""));
          text = texts.join("");
        } else if (type === "b") {
          const v = /<v[^>]*>([\s\S]*?)<\/v>/.exec(inner);
          text = (v?.[1] ?? "0") === "1" ? "TRUE" : "FALSE";
        } else {
          const v = /<v[^>]*>([\s\S]*?)<\/v>/.exec(inner);
          if (v) text = xmlUnescape(v[1] ?? "");
        }
        cells.push({ col, text });
      }
      cells.sort((a, b) => a.col - b.col);
      rows.push({ num: rowNum, cells });
    }
    rows.sort((a, b) => a.num - b.num);
    for (const row of rows) {
      const line = row.cells.map((c) => `${numberToCol(c.col)}${row.num}: ${c.text}`).join("\t");
      if (line) parts.push(line);
    }
  }
  let text = parts.join("\n");
  let truncated = false;
  if (Buffer.byteLength(text, "utf8") > MAX_TEXT_BYTES) {
    text = `${Buffer.from(text, "utf8").subarray(0, MAX_TEXT_BYTES).toString("utf8")}\n…（文本已截断）`;
    truncated = true;
  }
  return { kind: "xlsx", text, note: "", truncated };
}

/** buildCellXML：=开头→公式（不含等号）；数字→<v>；空串→自闭合；其余→inlineStr 保留空格。 */
function buildCellXML(value: string): string {
  if (value.startsWith("=")) {
    return `<f>${xmlEscape(value.slice(1))}</f>`;
  }
  if (value !== "" && Number.isFinite(Number(value)) && /^-?\d+(\.\d+)?([eE][+-]?\d+)?$/.test(value.trim())) {
    return `<v>${value}</v>`;
  }
  if (value === "") return "";
  return `<is><t xml:space="preserve">${xmlEscape(value)}</t></is>`;
}

async function replaceXlsx(path: string, edit: DocEdit): Promise<void> {
  if (edit.kind !== "cell" || !edit.cell) throw new Error("xlsx 只支持单元格更新");
  const raw = readFileSync(path);
  const { zip, sheets } = await loadXlsx(raw);
  if (sheets.length === 0) throw new Error("工作簿没有工作表");

  // 解析目标 sheet：名称或从 1 序号
  let sheet = sheets[0]!;
  if (edit.sheet) {
    const asNum = Number(edit.sheet);
    if (Number.isInteger(asNum) && asNum >= 1 && asNum <= sheets.length) {
      sheet = sheets[asNum - 1]!;
    } else {
      const found = sheets.find((s) => s.name === edit.sheet);
      if (!found) throw new Error(`找不到工作表：${edit.sheet}`);
      sheet = found;
    }
  }

  const ref = edit.cell.toUpperCase();
  if (!/^[A-Z]+[1-9][0-9]*$/.test(ref)) throw new Error(`无效的单元格引用：${edit.cell}`);
  const refMatch = /^([A-Z]+)(\d+)$/.exec(ref)!;
  const colNum = colToNumber(refMatch[1] ?? "A");
  const rowNum = Number(refMatch[2]);

  let sheetXml = (await zip.files[sheet.path]?.async("string")) ?? "";
  const cellXML = buildCellXML(edit.value ?? "");
  const cellRegex = new RegExp(`<c r="${ref}"[^>]*(?:/>|>[\\s\\S]*?</c>)`);

  const buildCell = (): string => {
    if (!cellXML) return `<c r="${ref}"/>`;
    // inlineStr 需要类型标记；其余（公式/数值）不带类型
    if (cellXML.startsWith("<is>")) return `<c r="${ref}" t="inlineStr">${cellXML}</c>`;
    return `<c r="${ref}">${cellXML}</c>`;
  };

  if (cellRegex.test(sheetXml)) {
    // 原位替换（新值不继承原单元格样式，行为与读取端 inlineStr 语义一致）
    sheetXml = sheetXml.replace(cellRegex, buildCell());
  } else {
    // 行内按列序插入 / 新建行按行号插入 / 追加末尾
    const rowRegex = new RegExp(`<row r="${rowNum}"([^>]*)>([\\s\\S]*?)</row>`);
    const rowMatch = rowRegex.exec(sheetXml);
    if (rowMatch) {
      const rowBody = rowMatch[2] ?? "";
      const cellRefRegex = /<c r="([A-Z]+)\d+"/g;
      let lastCol = 0;
      let m: RegExpExecArray | null;
      let insertPos = (rowMatch.index ?? 0) + (rowMatch[0]?.length ?? 0);
      while ((m = cellRefRegex.exec(rowBody)) !== null) {
        const c = colToNumber(m[1] ?? "A");
        if (c > colNum && cellXML) {
          insertPos = (rowMatch.index ?? 0) + m.index;
          break;
        }
        lastCol = c;
      }
      if (cellXML) {
        sheetXml =
          sheetXml.slice(0, insertPos) + buildCell() + sheetXml.slice(insertPos);
      } else {
        // 空值且原单元格不存在：无需操作
      }
      void lastCol;
    } else {
      // 新建行：插入到正确位置（按行号）或 sheetData 末尾
      const newRow = `<row r="${rowNum}">${cellXML ? buildCell() : ""}</row>`;
      const sheetDataOpen = sheetXml.indexOf("<sheetData");
      if (sheetDataOpen < 0) throw new Error("工作表缺少 sheetData");
      const insertAt = findSheetDataInsertPos(sheetXml, sheetDataOpen, rowNum);
      sheetXml = sheetXml.slice(0, insertAt) + newRow + sheetXml.slice(insertAt);
    }
  }

  await rewriteZip(raw, path, async (name) => (name === sheet.path ? sheetXml : null));
}

/** 找 sheetData 内按行号排序的插入位置。 */
function findSheetDataInsertPos(xml: string, sheetDataOpen: number, rowNum: number): number {
  const bodyStart = xml.indexOf(">", sheetDataOpen) + 1;
  const rowRegex = /<row[^>]*r="(\d+)"/g;
  rowRegex.lastIndex = bodyStart;
  let m: RegExpExecArray | null;
  while ((m = rowRegex.exec(xml)) !== null) {
    if (Number(m[1]) > rowNum) return m.index;
  }
  const close = xml.indexOf("</sheetData>", bodyStart);
  return close >= 0 ? close : xml.length;
}

// ---------- pdf ----------

/**
 * 轻量 PDF 文本层扫描：不渲染、不执行脚本、无 OCR。
 * /Encrypt → 报错；FlateDecode 流 zlib 解压；内容流解析字面量/十六进制字符串与定位操作符。
 */
function readPdf(raw: Buffer): DocResult {
  const head = raw.subarray(0, Math.min(raw.length, 4096)).toString("latin1");
  if (!head.startsWith("%PDF")) {
    return { kind: "pdf", text: "", note: "不是有效的 PDF 文件。", truncated: false };
  }
  if (raw.includes("/Encrypt")) {
    throw new Error("PDF 已加密，无法读取");
  }

  const chunks: string[] = [];
  let i = 0;
  const latin = raw.toString("latin1");
  for (;;) {
    const streamStart = latin.indexOf("stream", i);
    if (streamStart < 0) break;
    // 向前找最近的 << 字典判 FlateDecode
    const dictStart = latin.lastIndexOf("<<", streamStart);
    const dict = dictStart >= 0 ? latin.slice(dictStart, streamStart) : "";
    const isFlate = dict.includes("/FlateDecode");
    let dataStart = streamStart + "stream".length;
    if (latin[dataStart] === "\r") dataStart++;
    if (latin[dataStart] === "\n") dataStart++;
    const streamEnd = latin.indexOf("endstream", dataStart);
    if (streamEnd < 0) break;
    if (isFlate) {
      const data = raw.subarray(dataStart, streamEnd);
      try {
        const inflated = inflateSync(data).toString("latin1");
        chunks.push(inflated);
      } catch {
        // 解压失败保留原始数据（可能是未压缩内容流）
        chunks.push(data.toString("latin1"));
      }
    } else if (!dict.includes("/Image") && !dict.includes("/XObject")) {
      chunks.push(raw.subarray(dataStart, streamEnd).toString("latin1"));
    }
    i = streamEnd + "endstream".length;
  }

  const text = extractPdfText(chunks.join("\n"));
  if (!text.trim()) {
    return {
      kind: "pdf",
      text: "",
      note: "未检测到文本层，可能是扫描件；当前不支持 OCR。",
      truncated: false,
    };
  }
  let out = text;
  let truncated = false;
  if (Buffer.byteLength(out, "utf8") > MAX_TEXT_BYTES) {
    out = `${Buffer.from(out, "utf8").subarray(0, MAX_TEXT_BYTES).toString("utf8")}\n…（文本已截断）`;
    truncated = true;
  }
  return { kind: "pdf", text: out, note: "", truncated };
}

/** 解析 PDF 内容流中的文本操作。 */
function extractPdfText(content: string): string {
  let out = "";
  let i = 0;
  const n = content.length;
  let pendingSpace = false;

  const appendText = (s: string): void => {
    if (pendingSpace && out.length > 0 && !/\s$/.test(out)) out += " ";
    pendingSpace = false;
    out += s;
  };

  while (i < n) {
    const ch = content[i];
    if (ch === "(") {
      // 字面量字符串（支持嵌套括号与转义）
      let depth = 1;
      let j = i + 1;
      let literal = "";
      while (j < n && depth > 0) {
        const c = content[j]!;
        if (c === "\\") {
          const next = content[j + 1];
          j += 2;
          switch (next) {
            case "n":
              literal += "\n";
              break;
            case "r":
              literal += "\r";
              break;
            case "t":
              literal += "\t";
              break;
            case "b":
              literal += "\b";
              break;
            case "f":
              literal += "\f";
              break;
            case "(":
              literal += "(";
              break;
            case ")":
              literal += ")";
              break;
            case "\\": literal += "\\"; break;
            default: {
              if (next && /[0-7]/.test(next)) {
                // 八进制 ≤3 位
                let oct = next;
                while (oct.length < 3 && /[0-7]/.test(content[j] ?? "")) {
                  oct += content[j]!;
                  j++;
                }
                literal += String.fromCharCode(parseInt(oct, 8));
              } else if (next === "\n") {
                // 行续接
              } else if (next) {
                literal += next;
              }
            }
          }
          continue;
        }
        if (c === "(") depth++;
        else if (c === ")") {
          depth--;
          if (depth === 0) break;
        }
        literal += c;
        j++;
      }
      appendText(decodePdfString(Buffer.from(literal, "latin1")));
      i = j + 1;
      continue;
    }
    if (ch === "<" && content[i + 1] !== "<") {
      const close = content.indexOf(">", i);
      if (close < 0) break;
      const hex = content.slice(i + 1, close).replace(/[^0-9a-fA-F]/g, "");
      const buf = Buffer.from(hex.length % 2 === 0 ? hex : `${hex}0`, "hex");
      appendText(decodePdfString(buf));
      i = close + 1;
      continue;
    }
    if (ch === "-" && /[0-9]/.test(content[i + 1] ?? "") && out.length > 0) {
      // 数组里的负数间距（TJ）：≤-100 近视为词间空格
      const m = /^-([0-9]+)/.exec(content.slice(i));
      if (m && Number(m[1]) >= 100) pendingSpace = true;
      i += (m?.[0]?.length ?? 1);
      continue;
    }
    // 操作符
    const two = content.slice(i, i + 2);
    if (two === "Td" || two === "TD" || two === "T*" || two === "ET" || ch === "'" || ch === '"') {
      out += "\n";
      i += two === "Td" || two === "TD" || two === "T*" || two === "ET" ? 2 : 1;
      continue;
    }
    if (ch === "T" && content[i + 1] === "J") {
      pendingSpace = false;
      i += 2;
      continue;
    }
    i++;
  }
  return out
    .split("\n")
    .map((line) => line.replace(/\s+/g, " ").trim())
    .filter((line) => line.length > 0)
    .join("\n");
}

/** 解码 PDF 字符串字节：FE FF → UTF-16BE；EF BB BF → UTF-8；其余 latin1。 */
function decodePdfString(buf: Buffer): string {
  if (buf.length >= 2 && buf[0] === 0xfe && buf[1] === 0xff) {
    return buf.subarray(2).toString("utf16le").split("").reverse().join("") === ""
      ? ""
      : swapUTF16BE(buf.subarray(2));
  }
  if (buf.length >= 3 && buf[0] === 0xef && buf[1] === 0xbb && buf[2] === 0xbf) {
    return buf.subarray(3).toString("utf8");
  }
  return buf.toString("latin1");
}

function swapUTF16BE(buf: Buffer): string {
  const swapped = Buffer.alloc(buf.length);
  for (let i = 0; i + 1 < buf.length; i += 2) {
    swapped[i] = buf[i + 1] ?? 0;
    swapped[i + 1] = buf[i] ?? 0;
  }
  return swapped.toString("utf16le");
}

// 供外部复用（预览等）
export { writeFileSync };
