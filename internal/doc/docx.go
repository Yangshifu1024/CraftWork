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
				inText = true
			case "tab":
				if inText {
					paragraph.WriteString("\t")
				}
			case "br":
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

// ReplaceDOCXText 在 word/document.xml 中替换唯一一段文本。
//
// 只支持完整位于同一个 <w:t> 节点内的文本；如果被拆分成多个 run，
// 会返回可读错误，提示改用更短的片段。写入保留其余 ZIP 部件不变。
func ReplaceDOCXText(path, oldText, newText string) error {
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
		return fmt.Errorf("找到 %d 处相同文本，请提供更长的唯一片段", count)
	}
	updated := bytes.Replace(data, []byte(oldEscaped), []byte(newEscaped), 1)

	info, err := os.Stat(path)
	if err != nil {
		return err
	}
	return rewriteZip(path, "word/document.xml", updated, info.Mode().Perm())
}

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
		if file.FileInfo().IsDir() {
			continue
		}
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
	if err := os.Chmod(tempPath, mode); err != nil {
		return err
	}
	return os.Rename(tempPath, path)
}

func escapeXMLText(text string) string {
	var buffer bytes.Buffer
	_ = xml.EscapeText(&buffer, []byte(text))
	return buffer.String()
}

func filepathDir(path string) string {
	if index := strings.LastIndexAny(path, `/\`); index >= 0 {
		return path[:index]
	}
	return "."
}
