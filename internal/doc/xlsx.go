package doc

import (
	"archive/zip"
	"bytes"
	"encoding/xml"
	"fmt"
	"io"
	"os"
	"path"
	"regexp"
	"sort"
	"strconv"
	"strings"
)

type xlsxSheet struct {
	name string
	file string
}

var cellRefPattern = regexp.MustCompile(`^[A-Z]+[1-9][0-9]*$`)

// readXLSX 读取工作簿中的所有工作表，输出“单元格引用: 值”的文本。
func readXLSX(filePath string) (Document, error) {
	reader, err := zip.OpenReader(filePath)
	if err != nil {
		return Document{}, fmt.Errorf("打开 Excel 文档: %w", err)
	}
	defer reader.Close()

	sheets, err := xlsxSheets(reader)
	if err != nil {
		return Document{}, err
	}
	shared, err := xlsxSharedStrings(reader)
	if err != nil {
		return Document{}, err
	}

	var builder strings.Builder
	for _, sheet := range sheets {
		data, err := readZipEntry(reader, sheet.file)
		if err != nil {
			return Document{}, err
		}
		rows, err := xlsxRows(data, shared)
		if err != nil {
			return Document{}, fmt.Errorf("解析工作表 %s: %w", sheet.name, err)
		}
		builder.WriteString("工作表：" + sheet.name + "\n")
		rowNumbers := make([]int, 0, len(rows))
		for number := range rows {
			rowNumbers = append(rowNumbers, number)
		}
		sort.Ints(rowNumbers)
		for _, number := range rowNumbers {
			cells := rows[number]
			refs := make([]string, 0, len(cells))
			for ref := range cells {
				refs = append(refs, ref)
			}
			sort.Slice(refs, func(i, j int) bool {
				return columnIndex(refs[i]) < columnIndex(refs[j])
			})
			parts := make([]string, 0, len(refs))
			for _, ref := range refs {
				parts = append(parts, ref+": "+cells[ref])
			}
			builder.WriteString(strings.Join(parts, "\t") + "\n")
		}
		builder.WriteString("\n")
	}
	text, truncated := truncateText(strings.TrimRight(builder.String(), "\n"))
	return Document{Kind: "xlsx", Text: text, Truncated: truncated}, nil
}

func xlsxSheets(reader *zip.ReadCloser) ([]xlsxSheet, error) {
	workbook, err := readZipEntry(reader, "xl/workbook.xml")
	if err != nil {
		return nil, err
	}
	rels, err := readZipEntry(reader, "xl/_rels/workbook.xml.rels")
	if err != nil {
		return nil, err
	}
	targets, err := xlsxRelationships(rels)
	if err != nil {
		return nil, err
	}

	decoder := xml.NewDecoder(bytes.NewReader(workbook))
	var sheets []xlsxSheet
	for {
		token, err := decoder.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, fmt.Errorf("解析 workbook.xml: %w", err)
		}
		start, ok := token.(xml.StartElement)
		if !ok || start.Name.Local != "sheet" {
			continue
		}
		name := ""
		relationship := ""
		for _, attr := range start.Attr {
			switch attr.Name.Local {
			case "name":
				name = attr.Value
			case "id":
				if attr.Name.Space != "" {
					relationship = attr.Value
				}
			}
		}
		file := targets[relationship]
		if file == "" {
			continue
		}
		sheets = append(sheets, xlsxSheet{name: name, file: file})
	}
	if len(sheets) == 0 {
		return nil, fmt.Errorf("工作簿中没有工作表")
	}
	return sheets, nil
}

func xlsxRelationships(data []byte) (map[string]string, error) {
	decoder := xml.NewDecoder(bytes.NewReader(data))
	result := map[string]string{}
	for {
		token, err := decoder.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, fmt.Errorf("解析 workbook 关系: %w", err)
		}
		start, ok := token.(xml.StartElement)
		if !ok || start.Name.Local != "Relationship" {
			continue
		}
		id, target := "", ""
		for _, attr := range start.Attr {
			switch attr.Name.Local {
			case "Id":
				id = attr.Value
			case "Target":
				target = attr.Value
			}
		}
		if id != "" && target != "" {
			result[id] = normalizeXLSXTarget(target)
		}
	}
	return result, nil
}

func normalizeXLSXTarget(target string) string {
	target = strings.TrimPrefix(target, "/")
	if strings.HasPrefix(target, "xl/") {
		return target
	}
	cleaned := path.Clean("xl/" + target)
	return strings.TrimPrefix(cleaned, "../")
}

func xlsxSharedStrings(reader *zip.ReadCloser) ([]string, error) {
	data, err := readZipEntry(reader, "xl/sharedStrings.xml")
	if err != nil {
		if strings.Contains(err.Error(), "文档缺少") {
			return nil, nil
		}
		return nil, err
	}
	decoder := xml.NewDecoder(bytes.NewReader(data))
	var result []string
	var builder strings.Builder
	inText := false
	inItem := false
	for {
		token, err := decoder.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, fmt.Errorf("解析 sharedStrings.xml: %w", err)
		}
		switch typed := token.(type) {
		case xml.StartElement:
			switch typed.Name.Local {
			case "si":
				inItem = true
				builder.Reset()
			case "t":
				if inItem {
					inText = true
				}
			}
		case xml.CharData:
			if inText {
				builder.Write(typed)
			}
		case xml.EndElement:
			switch typed.Name.Local {
			case "t":
				inText = false
			case "si":
				if inItem {
					result = append(result, builder.String())
				}
				inItem = false
			}
		}
	}
	return result, nil
}

func xlsxRows(data []byte, shared []string) (map[int]map[string]string, error) {
	decoder := xml.NewDecoder(bytes.NewReader(data))
	rows := map[int]map[string]string{}
	rowNumber := 0
	inCell := false
	inValue := false
	inFormula := false
	inInlineText := false
	cellType := ""
	var ref, value, formula, inline strings.Builder

	flush := func() {
		if !inCell || ref.Len() == 0 {
			return
		}
		text := ""
		switch {
		case formula.Len() > 0:
			text = "=" + formula.String()
		case cellType == "s":
			index, err := strconv.Atoi(strings.TrimSpace(value.String()))
			if err == nil && index >= 0 && index < len(shared) {
				text = shared[index]
			}
		case cellType == "inlineStr":
			text = inline.String()
		case cellType == "b":
			if strings.TrimSpace(value.String()) == "1" {
				text = "TRUE"
			} else {
				text = "FALSE"
			}
		default:
			text = value.String()
		}
		if text != "" {
			if rows[rowNumber] == nil {
				rows[rowNumber] = map[string]string{}
			}
			rows[rowNumber][strings.ToUpper(ref.String())] = text
		}
		ref.Reset()
		value.Reset()
		formula.Reset()
		inline.Reset()
		cellType = ""
		inCell = false
	}

	for {
		token, err := decoder.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, err
		}
		switch typed := token.(type) {
		case xml.StartElement:
			switch typed.Name.Local {
			case "row":
				rowNumber = attrInt(typed, "r")
			case "c":
				inCell = true
				ref.Reset()
				value.Reset()
				formula.Reset()
				inline.Reset()
				ref.WriteString(attrValue(typed, "r"))
				cellType = attrValue(typed, "t")
			case "v":
				if inCell {
					inValue = true
				}
			case "f":
				if inCell {
					inFormula = true
				}
			case "is":
				// inlineStr 容器
			case "t":
				if inCell && cellType == "inlineStr" {
					inInlineText = true
				}
			}
		case xml.CharData:
			switch {
			case inValue:
				value.Write(typed)
			case inFormula:
				formula.Write(typed)
			case inInlineText:
				inline.Write(typed)
			}
		case xml.EndElement:
			switch typed.Name.Local {
			case "v":
				inValue = false
			case "f":
				inFormula = false
			case "t":
				inInlineText = false
			case "c":
				flush()
			}
		}
	}
	return rows, nil
}

// SetXLSXCell 修改一个单元格的值或公式，保留其余 ZIP 部件与样式。
func SetXLSXCell(filePath, sheetName, ref, value string) error {
	ref = strings.ToUpper(strings.TrimSpace(ref))
	if !cellRefPattern.MatchString(ref) {
		return fmt.Errorf("无效的单元格引用：%s", ref)
	}
	reader, err := zip.OpenReader(filePath)
	if err != nil {
		return fmt.Errorf("打开 Excel 文档: %w", err)
	}
	defer reader.Close()

	sheets, err := xlsxSheets(reader)
	if err != nil {
		return err
	}
	target := ""
	if sheetName == "" {
		target = sheets[0].file
	} else if index, err := strconv.Atoi(sheetName); err == nil && index >= 1 && index <= len(sheets) {
		target = sheets[index-1].file
	} else {
		for _, sheet := range sheets {
			if sheet.name == sheetName {
				target = sheet.file
				break
			}
		}
	}
	if target == "" {
		return fmt.Errorf("找不到工作表：%s", sheetName)
	}

	data, err := readZipEntry(reader, target)
	if err != nil {
		return err
	}
	updated, err := setCellXML(string(data), ref, value)
	if err != nil {
		return err
	}
	info, err := os.Stat(filePath)
	if err != nil {
		return err
	}
	return rewriteZip(filePath, target, []byte(updated), info.Mode().Perm())
}

var (
	cellPattern  = regexp.MustCompile(`(?s)<c r="([A-Z]+[0-9]+)"[^>]*(?:/>|>.*?</c>)`)
	rowPattern   = regexp.MustCompile(`(?s)<row r="([0-9]+)"[^>]*>.*?</row>`)
	stylePattern = regexp.MustCompile(`s="([0-9]+)"`)
)

func setCellXML(sheetXML, ref, value string) (string, error) {
	style := ""
	for _, match := range cellPattern.FindAllStringSubmatch(sheetXML, -1) {
		if match[1] == ref {
			if styleMatch := stylePattern.FindStringSubmatch(match[0]); styleMatch != nil {
				style = ` s="` + styleMatch[1] + `"`
			}
			break
		}
	}
	cellXML := buildCellXML(ref, style, value)
	if cellPattern.MatchString(sheetXML) {
		replaced := false
		updated := cellPattern.ReplaceAllStringFunc(sheetXML, func(match string) string {
			sub := cellPattern.FindStringSubmatch(match)
			if sub != nil && sub[1] == ref {
				replaced = true
				return cellXML
			}
			return match
		})
		if replaced {
			return updated, nil
		}
	}

	rowNumber := cellRow(ref)
	for _, location := range rowPattern.FindAllStringSubmatchIndex(sheetXML, -1) {
		number, _ := strconv.Atoi(sheetXML[location[2]:location[3]])
		if number == rowNumber {
			rowStart, rowEnd := location[0], location[1]
			rowXML := sheetXML[rowStart:rowEnd]
			rowXML = insertCellIntoRow(rowXML, ref, cellXML)
			return sheetXML[:rowStart] + rowXML + sheetXML[rowEnd:], nil
		}
	}

	newRow := fmt.Sprintf(`<row r="%d">%s</row>`, rowNumber, cellXML)
	closeIndex := strings.Index(sheetXML, "</sheetData>")
	if closeIndex < 0 {
		return "", fmt.Errorf("工作表缺少 sheetData")
	}
	for _, location := range rowPattern.FindAllStringSubmatchIndex(sheetXML, -1) {
		number, _ := strconv.Atoi(sheetXML[location[2]:location[3]])
		if number > rowNumber {
			return sheetXML[:location[0]] + newRow + sheetXML[location[0]:], nil
		}
	}
	return sheetXML[:closeIndex] + newRow + sheetXML[closeIndex:], nil
}

func insertCellIntoRow(rowXML, ref, cellXML string) string {
	column := columnIndex(ref)
	insertAt := strings.LastIndex(rowXML, "</row>")
	if insertAt < 0 {
		return rowXML
	}
	for _, location := range cellPattern.FindAllStringSubmatchIndex(rowXML, -1) {
		existing := rowXML[location[2]:location[3]]
		if columnIndex(existing) > column {
			return rowXML[:location[0]] + cellXML + rowXML[location[0]:]
		}
	}
	return rowXML[:insertAt] + cellXML + rowXML[insertAt:]
}

func buildCellXML(ref, style, value string) string {
	escaped := escapeXMLText(value)
	if strings.HasPrefix(value, "=") {
		return fmt.Sprintf(`<c r="%s"%s><f>%s</f></c>`, ref, style, escapeXMLText(strings.TrimPrefix(value, "=")))
	}
	if _, err := strconv.ParseFloat(strings.TrimSpace(value), 64); err == nil && strings.TrimSpace(value) != "" {
		return fmt.Sprintf(`<c r="%s"%s><v>%s</v></c>`, ref, style, strings.TrimSpace(value))
	}
	if value == "" {
		return fmt.Sprintf(`<c r="%s"%s/>`, ref, style)
	}
	return fmt.Sprintf(`<c r="%s"%s t="inlineStr"><is><t xml:space="preserve">%s</t></is></c>`, ref, style, escaped)
}

func attrValue(start xml.StartElement, name string) string {
	for _, attr := range start.Attr {
		if attr.Name.Local == name {
			return attr.Value
		}
	}
	return ""
}

func attrInt(start xml.StartElement, name string) int {
	number, _ := strconv.Atoi(attrValue(start, name))
	return number
}

func cellRow(ref string) int {
	number := 0
	for _, char := range ref {
		if char >= '0' && char <= '9' {
			number = number*10 + int(char-'0')
		}
	}
	return number
}

func columnIndex(ref string) int {
	index := 0
	for _, char := range ref {
		if char < 'A' || char > 'Z' {
			break
		}
		index = index*26 + int(char-'A'+1)
	}
	return index
}
