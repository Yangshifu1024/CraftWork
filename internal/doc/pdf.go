// 本文件负责 PDF（Portable Document Format，便携式文档格式）文本层的
// 提取。实现是面向文本内容的轻量扫描器：遍历文件中的流对象，按需解压
// 后解析字符串操作符；不渲染页面、不执行脚本，也不做 OCR（光学字符识别）。

package doc

import (
	"bytes"
	"compress/zlib"
	"encoding/hex"
	"fmt"
	"io"
	"os"
	"strings"
	"unicode/utf16"
)

// readPDF 提取 PDF 文本层。加密 PDF 与没有文本层的扫描件会返回说明。
//
// 先按文件里是否出现 /Encrypt 标记做加密判断，再扫描所有流对象。
// 提取结果为空时不报错，而是返回 Note 说明可能是扫描件，因为这类文件
// 本身结构合法，只是没有可提取的文本。
func readPDF(path string) (Document, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return Document{}, fmt.Errorf("读取 PDF: %w", err)
	}
	if len(data) > MaxFileBytes {
		return Document{}, fmt.Errorf("PDF 超过 50MB，暂不支持解析")
	}
	if bytes.Contains(data, []byte("/Encrypt")) {
		return Document{}, fmt.Errorf("PDF 已加密，暂不支持读取")
	}

	text := pdfText(data)
	text = strings.TrimSpace(text)
	if text == "" {
		return Document{
			Kind: "pdf",
			Note: "未检测到文本层，可能是扫描件；当前不支持 OCR。",
		}, nil
	}
	text, truncated := truncateText(text)
	return Document{Kind: "pdf", Text: text, Truncated: truncated}, nil
}

// pdfText 扫描 PDF 中的所有流对象并拼接其中的文本片段。
//
// 这里采用轻量扫描而不是完整解析对象图：先找到 stream 关键字，再向前
// 找最近的 << 判断是否带 /FlateDecode 过滤器（zlib 压缩），解压后交给
// pdfContentText 解析。它不处理交叉引用表、对象流等高级结构，但足以
// 覆盖常见导出工具生成的文本层。
func pdfText(data []byte) string {
	var builder strings.Builder
	index := 0
	for {
		start := bytes.Index(data[index:], []byte("stream"))
		if start < 0 {
			break
		}
		start += index
		// 流字典紧挨在 stream 之前，最后一个 << 到 stream 之间就是字典。
		dictStart := bytes.LastIndex(data[:start], []byte("<<"))
		dict := []byte{}
		if dictStart >= 0 {
			dict = data[dictStart:start]
		}
		contentStart := start + len("stream")
		// stream 关键字后允许一个 CR、一个 LF 或 CRLF，属于数据分隔符。
		if contentStart < len(data) && data[contentStart] == '\r' {
			contentStart++
		}
		if contentStart < len(data) && data[contentStart] == '\n' {
			contentStart++
		}
		end := bytes.Index(data[contentStart:], []byte("endstream"))
		if end < 0 {
			break
		}
		end += contentStart
		raw := data[contentStart:end]
		if bytes.Contains(dict, []byte("/FlateDecode")) {
			// 解压失败时保留原始数据，交给后续解析器尽量提取可读内容。
			if decoded, err := inflate(raw); err == nil {
				raw = decoded
			}
		}
		builder.WriteString(pdfContentText(raw))
		// 从 endstream 之后继续，避免把结束标记里的 stream 当成新流。
		index = end + len("endstream")
	}
	return builder.String()
}

// inflate 用 zlib 解压流数据，并限制解压后的最大长度。
func inflate(data []byte) ([]byte, error) {
	reader, err := zlib.NewReader(bytes.NewReader(data))
	if err != nil {
		return nil, err
	}
	defer reader.Close()
	return io.ReadAll(io.LimitReader(reader, MaxFileBytes))
}

// pdfContentText 从已解压的内容流中提取文本。
//
// 内容流由操作符与操作数组成：(...) 是字面量字符串，<...> 是十六进制
// 字符串，[...] 是数组（常用于字距调整），Td / TD / T* / ET 以及单引号、
// 双引号操作符表示换行。这里只收集字符串内容，忽略字体、颜色等排版指令。
func pdfContentText(content []byte) string {
	var builder strings.Builder
	lastWasText := false
	index := 0
	for index < len(content) {
		char := content[index]
		switch {
		case char == '(':
			text, next := readPDFLiteral(content, index)
			appendPDFText(&builder, text, &lastWasText)
			index = next
		case char == '<' && index+1 < len(content) && content[index+1] != '<':
			// 单个 < 才是十六进制字符串；<< 是字典开始，必须跳过。
			text, next := readPDFHex(content, index)
			appendPDFText(&builder, text, &lastWasText)
			index = next
		case char == '[':
			// 数组里通常混有字符串与字距调整数字，交给专门的解析。
			index = readPDFArray(content, index, &builder, &lastWasText)
		case isPDFLetter(char):
			// 读取一个完整操作符名称，再判断它是否代表换行。
			start := index
			for index < len(content) && isPDFLetter(content[index]) {
				index++
			}
			switch string(content[start:index]) {
			case "Td", "TD", "T*", "ET", "'", `"`:
				trimPDFNewline(&builder)
				builder.WriteByte('\n')
				lastWasText = false
			}
		default:
			index++
		}
	}
	return builder.String()
}

// readPDFLiteral 读取一个字面量字符串，返回内容与下一个读取位置。
//
// 反斜杠引入转义：常见控制字符有专门写法，\(、\)、\\ 表示字符本身，
// 三位以内的八进制数字表示一个字节；反斜杠后跟换行表示行续接，
// 换行本身不属于内容。
func readPDFLiteral(content []byte, start int) (string, int) {
	var builder strings.Builder
	index := start + 1
	for index < len(content) {
		char := content[index]
		if char == ')' {
			return builder.String(), index + 1
		}
		if char != '\\' {
			builder.WriteByte(char)
			index++
			continue
		}
		index++
		if index >= len(content) {
			break
		}
		escaped := content[index]
		switch escaped {
		case 'n':
			builder.WriteByte('\n')
		case 'r':
			builder.WriteByte('\r')
		case 't':
			builder.WriteByte('\t')
		case 'b':
			builder.WriteByte('\b')
		case 'f':
			builder.WriteByte('\f')
		case '(', ')', '\\':
			builder.WriteByte(escaped)
		case '\n':
			// 行续接
		case '\r':
			// CRLF 形式的行续接要把 LF 一起跳过。
			if index+1 < len(content) && content[index+1] == '\n' {
				index++
			}
		default:
			if escaped >= '0' && escaped <= '7' {
				// 八进制转义最多三位，表示一个字节。
				value := 0
				count := 0
				for count < 3 && index < len(content) && content[index] >= '0' && content[index] <= '7' {
					value = value*8 + int(content[index]-'0')
					index++
					count++
				}
				builder.WriteByte(byte(value))
				continue
			}
			builder.WriteByte(escaped)
		}
		index++
	}
	return builder.String(), index
}

// readPDFHex 读取一个十六进制字符串，返回解码后的文本与下一个读取位置。
//
// 尖括号之间的空白与奇数个半字节都会被忽略或补零，解码出的字节再按
// PDF 字符串编码规则转换成文本。
func readPDFHex(content []byte, start int) (string, int) {
	index := start + 1
	var digits strings.Builder
	for index < len(content) && content[index] != '>' {
		char := content[index]
		if (char >= '0' && char <= '9') || (char >= 'a' && char <= 'f') || (char >= 'A' && char <= 'F') {
			digits.WriteByte(char)
		}
		index++
	}
	if index < len(content) {
		index++
	}
	raw, err := hex.DecodeString(digits.String())
	if err != nil || len(raw) == 0 {
		return "", index
	}
	return decodePDFString(raw), index
}

// readPDFArray 读取一个数组，把其中的字符串追加到输出。
//
// 数组在 PDF 中常用于文字定位，元素可以是字符串，也可以是字距调整
// 数字；较大的负数字近似表示词间空格，因此这里会插入一个空格。
// 返回下一个读取位置。
func readPDFArray(content []byte, start int, builder *strings.Builder, lastWasText *bool) int {
	index := start + 1
	for index < len(content) && content[index] != ']' {
		switch {
		case content[index] == '(':
			text, next := readPDFLiteral(content, index)
			appendPDFText(builder, text, lastWasText)
			index = next
		case content[index] == '<' && index+1 < len(content) && content[index+1] != '<':
			text, next := readPDFHex(content, index)
			appendPDFText(builder, text, lastWasText)
			index = next
		case content[index] == '-' || (content[index] >= '0' && content[index] <= '9'):
			// 读取一个数字，只用于判断是否需要补空格，不作为文本输出。
			startNumber := index
			index++
			for index < len(content) && (content[index] == '.' || (content[index] >= '0' && content[index] <= '9')) {
				index++
			}
			number := strings.TrimSpace(string(content[startNumber:index]))
			// 用字符串比较做近似判断：负得足够多说明字距很大，视为词间空格。
			if len(number) > 1 && strings.HasPrefix(number, "-") && number <= "-100" {
				builder.WriteByte(' ')
				*lastWasText = false
			}
		default:
			index++
		}
	}
	if index < len(content) {
		index++
	}
	return index
}

// appendPDFText 追加一段文本，并在相邻片段之间补一个空格。
//
// 当上一段与当前段都是文本，且两侧都没有空白字符时补空格，
// 避免把本应分开的单词粘在一起。
func appendPDFText(builder *strings.Builder, text string, lastWasText *bool) {
	if text == "" {
		return
	}
	if *lastWasText && builder.Len() > 0 {
		previous := builder.String()[builder.Len()-1]
		if previous != ' ' && previous != '\n' && text[0] != ' ' {
			builder.WriteByte(' ')
		}
	}
	builder.WriteString(text)
	*lastWasText = true
}

// trimPDFNewline 去掉输出末尾多余的一个换行。
//
// 换行操作符会先调用它，再写入新换行，避免连续出现空行。
func trimPDFNewline(builder *strings.Builder) {
	value := builder.String()
	if strings.HasSuffix(value, "\n") {
		builder.Reset()
		builder.WriteString(value[:len(value)-1])
	}
}

// decodePDFString 按 PDF 字符串的约定把字节转换成文本。
//
// 带 FE FF 前缀的是 UTF-16 大端序，按 UTF-16 解码；带 EF BB BF 前缀的
// 是 UTF-8；其余按原始字节直接转字符串，通常是 PDFDocEncoding 或拉丁
// 字母编码，这里不做进一步转码。
func decodePDFString(raw []byte) string {
	if len(raw) >= 2 && raw[0] == 0xFE && raw[1] == 0xFF {
		units := make([]uint16, 0, (len(raw)-2)/2)
		for index := 2; index+1 < len(raw); index += 2 {
			units = append(units, uint16(raw[index])<<8|uint16(raw[index+1]))
		}
		return string(utf16.Decode(units))
	}
	if bytes.HasPrefix(raw, []byte{0xEF, 0xBB, 0xBF}) {
		return string(raw[3:])
	}
	return string(raw)
}

// isPDFLetter 判断字节是否属于操作符名称。
//
// PDF 操作符由字母组成，单引号与双引号也是独立的换行操作符，
// 因此一并视为操作符字符。
func isPDFLetter(char byte) bool {
	return (char >= 'a' && char <= 'z') || (char >= 'A' && char <= 'Z') || char == '\'' || char == '"'
}
