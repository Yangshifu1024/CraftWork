// 本文件负责 Word（.docx）的文本读取与有限写入。docx 本质是一个 ZIP
//（压缩包），正文位于 word/document.xml；读取时按段落与表格提取文本，
// 写入时只替换单个 <w:t> 节点内的内容，其余 ZIP 部件原样保留。

package doc

import (
	"archive/zip"
	"bytes"
	"encoding/xml"
	"fmt"
	"io"
	"os"
	"strings"
)

// readDOCX 读取 word/document.xml，按段落与表格输出纯文本。
//
// 只读取正文部件，页眉、页脚、批注与脚注都不在本次范围内；
// 结果超过上限时截断并标记 Truncated。
func readDOCX(path string) (Document, error) {
	reader, err := zip.OpenReader(path)
	if err != nil {
		return Document{}, fmt.Errorf("打开 Word 文档: %w", err)
	}
	defer reader.Close()

	data, err := readZipEntry(reader, "word/document.xml")
	if err != nil {
		return Document{}, err
	}
	text, err := docxText(data)
	if err != nil {
		return Document{}, err
	}
	text, truncated := truncateText(text)
	return Document{Kind: "docx", Text: text, Truncated: truncated}, nil
}

// docxText 解析 word/document.xml，按文档顺序输出纯文本。
//
// 段落之间用换行分隔；表格按行输出，同一行的单元格用制表符连接，
// 单元格内的多个段落用换行连接。这里只关心文本节点，样式、批注与
// 其它非正文内容都会被忽略。
func docxText(data []byte) (string, error) {
	decoder := xml.NewDecoder(bytes.NewReader(data))
	var lines []string
	var paragraph strings.Builder
	var cell strings.Builder
	var row []string
	inText := false
	inCell := false
	inTable := false

	for {
		token, err := decoder.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			return "", fmt.Errorf("解析 Word 文档: %w", err)
		}
		switch typed := token.(type) {
		case xml.StartElement:
			switch typed.Name.Local {
			case "tbl":
				inTable = true
			case "tr":
				row = nil
			case "tc":
				inCell = true
				cell.Reset()
			case "p":
				paragraph.Reset()
			case "t":
				// 只有 <w:t> 内的字符数据才是可见文本。
				inText = true
			case "tab":
				// 文档内的制表符是一个独立的空元素。
				if inText {
					paragraph.WriteString("\t")
				}
			case "br":
				// 手动换行同样以空元素表示。
				paragraph.WriteString("\n")
			}
		case xml.CharData:
			if inText {
				paragraph.Write(typed)
			}
		case xml.EndElement:
			switch typed.Name.Local {
			case "t":
				inText = false
			case "p":
				// 段落结束：表格内累积到单元格，正文直接作为一行。
				text := paragraph.String()
				if inCell {
					if cell.Len() > 0 {
						cell.WriteString("\n")
					}
					cell.WriteString(text)
				} else {
					lines = append(lines, text)
				}
			case "tc":
				row = append(row, strings.TrimSpace(cell.String()))
				inCell = false
			case "tr":
				// 表格行结束：制表符分隔的单元格组成一行文本。
				if inTable && len(row) > 0 {
					lines = append(lines, strings.Join(row, "\t"))
				}
				row = nil
			case "tbl":
				inTable = false
			}
		}
	}
	return strings.Join(lines, "\n"), nil
}

// replaceDOCXText 在 word/document.xml 中替换唯一一段文本。
//
// 只支持完整位于同一个 <w:t> 节点内的文本；如果被拆分成多个 run，
// 会返回可读错误，提示改用更短的片段。写入保留其余 ZIP 部件不变。
//
// 匹配前先对原文与新文本做 XML 转义，因为文档里保存的是转义后的形式；
// 同时要求原文在文档中恰好出现一次，避免误改其它位置。
func replaceDOCXText(path, oldText, newText string) error {
	if oldText == "" {
		return fmt.Errorf("old_text 不能为空")
	}
	reader, err := zip.OpenReader(path)
	if err != nil {
		return fmt.Errorf("打开 Word 文档: %w", err)
	}
	defer reader.Close()

	data, err := readZipEntry(reader, "word/document.xml")
	if err != nil {
		return err
	}
	oldEscaped := escapeXMLText(oldText)
	newEscaped := escapeXMLText(newText)
	count := bytes.Count(data, []byte(oldEscaped))
	if count == 0 {
		return fmt.Errorf("没有找到要替换的文本；文本可能被拆分到多个格式片段中，请换一段更短的唯一文本")
	}
	if count > 1 {
		// 出现多次时无法判断用户想改哪一处，直接要求提供更长的唯一片段。
		return fmt.Errorf("找到 %d 处相同文本，请提供更长的唯一片段", count)
	}
	updated := bytes.Replace(data, []byte(oldEscaped), []byte(newEscaped), 1)

	info, err := os.Stat(path)
	if err != nil {
		return err
	}
	return rewriteZip(path, "word/document.xml", updated, info.Mode().Perm())
}

// readZipEntry 按完整名称读取 ZIP 中的一个条目。
//
// 只读取不超过 MaxFileBytes 的内容，防止压缩包解压后过大；
// 找不到条目时返回带名称的错误，便于定位缺失的部件。
func readZipEntry(reader *zip.ReadCloser, name string) ([]byte, error) {
	for _, file := range reader.File {
		if file.Name != name {
			continue
		}
		handle, err := file.Open()
		if err != nil {
			return nil, fmt.Errorf("读取 %s: %w", name, err)
		}
		defer handle.Close()
		data, err := io.ReadAll(io.LimitReader(handle, MaxFileBytes))
		if err != nil {
			return nil, fmt.Errorf("读取 %s: %w", name, err)
		}
		return data, nil
	}
	return nil, fmt.Errorf("文档缺少 %s", name)
}

// rewriteZip 复制原 ZIP 并替换其中一个条目。
//
// ZIP（压缩包）不能原地修改单个条目，因此这里创建一个新压缩包：
// 逐条复制原有部件，遇到目标条目时写入新内容，其余内容与压缩方式、
// 修改时间、权限都保持不变。新包先写到同目录临时文件，成功后再
// 原子替换原文件，避免中途失败损坏原文档。
func rewriteZip(path, entryName string, content []byte, mode os.FileMode) error {
	reader, err := zip.OpenReader(path)
	if err != nil {
		return err
	}
	defer reader.Close()

	temp, err := os.CreateTemp(filepathDir(path), ".craftwork-doc-*.tmp")
	if err != nil {
		return fmt.Errorf("创建临时文件: %w", err)
	}
	tempPath := temp.Name()
	defer os.Remove(tempPath)

	writer := zip.NewWriter(temp)
	for _, file := range reader.File {
		// 目录条目不需要复制，新包会根据条目名称自动还原目录结构。
		if file.FileInfo().IsDir() {
			continue
		}
		// 复用原有压缩方式与时间戳，尽量保持文档指纹稳定。
		header := &zip.FileHeader{Name: file.Name, Method: file.Method, Modified: file.Modified}
		header.SetMode(file.Mode())
		destination, err := writer.CreateHeader(header)
		if err != nil {
			temp.Close()
			writer.Close()
			return err
		}
		if file.Name == entryName {
			if _, err := destination.Write(content); err != nil {
				temp.Close()
				writer.Close()
				return err
			}
			continue
		}
		source, err := file.Open()
		if err != nil {
			temp.Close()
			writer.Close()
			return err
		}
		if _, err := io.Copy(destination, source); err != nil {
			source.Close()
			temp.Close()
			writer.Close()
			return err
		}
		source.Close()
	}
	if err := writer.Close(); err != nil {
		temp.Close()
		return err
	}
	if err := temp.Close(); err != nil {
		return err
	}
	// 临时文件默认权限较严，替换前恢复原文档权限。
	if err := os.Chmod(tempPath, mode); err != nil {
		return err
	}
	return os.Rename(tempPath, path)
}

// escapeXMLText 按 XML 规则转义文本中的特殊字符。
//
// 写入 bytes.Buffer 不会失败，因此这里可以忽略返回的错误。
func escapeXMLText(text string) string {
	var buffer bytes.Buffer
	_ = xml.EscapeText(&buffer, []byte(text))
	return buffer.String()
}

// filepathDir 返回路径的目录部分。
//
// 同时识别 / 与 \ 两种分隔符；没有分隔符时返回当前目录。
func filepathDir(path string) string {
	if index := strings.LastIndexAny(path, `/\`); index >= 0 {
		return path[:index]
	}
	return "."
}
