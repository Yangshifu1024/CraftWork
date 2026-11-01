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

func pdfText(data []byte) string {
	var builder strings.Builder
	index := 0
	for {
		start := bytes.Index(data[index:], []byte("stream"))
		if start < 0 {
			break
		}
		start += index
		dictStart := bytes.LastIndex(data[:start], []byte("<<"))
		dict := []byte{}
		if dictStart >= 0 {
			dict = data[dictStart:start]
		}
		contentStart := start + len("stream")
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
			if decoded, err := inflate(raw); err == nil {
				raw = decoded
			}
		}
		builder.WriteString(pdfContentText(raw))
		index = end + len("endstream")
	}
	return builder.String()
}

func inflate(data []byte) ([]byte, error) {
	reader, err := zlib.NewReader(bytes.NewReader(data))
	if err != nil {
		return nil, err
	}
	defer reader.Close()
	return io.ReadAll(io.LimitReader(reader, MaxFileBytes))
}

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
			text, next := readPDFHex(content, index)
			appendPDFText(&builder, text, &lastWasText)
			index = next
		case char == '[':
			index = readPDFArray(content, index, &builder, &lastWasText)
		case isPDFLetter(char):
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
			if index+1 < len(content) && content[index+1] == '\n' {
				index++
			}
		default:
			if escaped >= '0' && escaped <= '7' {
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
			startNumber := index
			index++
			for index < len(content) && (content[index] == '.' || (content[index] >= '0' && content[index] <= '9')) {
				index++
			}
			number := strings.TrimSpace(string(content[startNumber:index]))
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

func trimPDFNewline(builder *strings.Builder) {
	value := builder.String()
	if strings.HasSuffix(value, "\n") {
		builder.Reset()
		builder.WriteString(value[:len(value)-1])
	}
}

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

func isPDFLetter(char byte) bool {
	return (char >= 'a' && char <= 'z') || (char >= 'A' && char <= 'Z') || char == '\'' || char == '"'
}
