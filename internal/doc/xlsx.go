// 本文件负责 Excel（.xlsx）的文本读取与单元格写入。xlsx 是一个 ZIP
//（压缩包）：工作簿结构在 xl/workbook.xml，关系映射在
// xl/_rels/workbook.xml.rels，工作表内容在 xl/worksheets/*.xml，
// 共享字符串在 xl/sharedStrings.xml。写入只替换目标单元格，保留样式。

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

// xlsxSheet 是一个工作表的名称与对应的 ZIP 条目路径。
type xlsxSheet struct {
	name string // 显示名称，来自 workbook.xml
	file string // 工作表 XML 在压缩包中的路径
}

// cellRefPattern 匹配规范的单元格引用，例如 A1、AB12。
var cellRefPattern = regexp.MustCompile(`^[A-Z]+[1-9][0-9]*$`)

// readXLSX 读取工作簿中的所有工作表，输出“单元格引用: 值”的文本。
//
// 每个工作表以“工作表：名称”开头；行按行号升序、单元格按列序升序
// 输出，同一行的多个单元格用制表符分隔，避免受 XML 中元素顺序影响。
// 公式只输出公式文本，不计算结果。
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
		// 行号来自 map，必须显式排序才能得到稳定的输出顺序。
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
			// 单元格同样来自 map，按列号排序而不是按字符串排序，
			// 否则 Z 会排在 AA 之后、A10 会排在 A2 之前。
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

// xlsxSheets 解析 workbook.xml 与关系文件，返回按工作簿顺序排列的工作表。
//
// workbook.xml 里的 sheet 只给出名称与关系 ID，真正的工作表路径要从
// xl/_rels/workbook.xml.rels 中查；两者对不上时跳过该工作表。
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
				// 关系 ID 通常写作 r:id，带命名空间；普通的 id 属性不是关系 ID。
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

// xlsxRelationships 解析关系文件，返回“关系 ID 到工作表路径”的映射。
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

// normalizeXLSXTarget 把关系文件里的 Target 统一成压缩包内的绝对路径。
//
// Target 可能是 "/xl/worksheets/sheet1.xml" 这样的绝对形式，也可能是
// "worksheets/sheet1.xml" 这样的相对形式（相对 xl/ 目录）；统一后
// 才能直接用 readZipEntry 按名称查找。
func normalizeXLSXTarget(target string) string {
	target = strings.TrimPrefix(target, "/")
	if strings.HasPrefix(target, "xl/") {
		return target
	}
	cleaned := path.Clean("xl/" + target)
	return strings.TrimPrefix(cleaned, "../")
}

// xlsxSharedStrings 读取共享字符串表。
//
// 单元格类型为 s 时，值保存的是这里的下标。共享字符串文件可以缺失，
// 此时返回空表；字符串里的富文本会被拆成多个 t 节点，这里按顺序拼接。
func xlsxSharedStrings(reader *zip.ReadCloser) ([]string, error) {
	data, err := readZipEntry(reader, "xl/sharedStrings.xml")
	if err != nil {
		// 没有共享字符串表的工作簿是合法的，此时所有文本都在单元格内联存储。
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
				// 每个 si 是一条共享字符串。
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

// xlsxRows 解析一个工作表的 XML，返回“行号到单元格引用再到文本”的两级映射。
//
// 单元格类型 t 决定取值方式：s 表示共享字符串下标，inlineStr 表示
// 内联字符串，b 表示布尔值，其余按数字或普通文本原样返回。有公式时
// 优先输出公式原文，不计算公式结果。空单元格不会写入结果。
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

	// flush 在单元格结束时把累积内容转换成文本并写入结果。
	flush := func() {
		if !inCell || ref.Len() == 0 {
			return
		}
		text := ""
		switch {
		case formula.Len() > 0:
			// 公式优先：文件里可能同时存有缓存值，但这里不计算也不采信。
			text = "=" + formula.String()
		case cellType == "s":
			// 共享字符串：值是一个指向共享字符串表的下标。
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
			// 引用统一转成大写，避免大小写不同的引用被当成两个单元格。
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
				// 单元格开始：记录引用与类型，并清空上一次的累积内容。
				inCell = true
				ref.Reset()
				value.Reset()
				formula.Reset()
				inline.Reset()
				ref.WriteString(attrValue(typed, "r"))
				cellType = attrValue(typed, "t")
			case "v":
				// v 保存普通值或共享字符串下标。
				if inCell {
					inValue = true
				}
			case "f":
				// f 保存公式表达式，不含前导等号。
				if inCell {
					inFormula = true
				}
			case "is":
				// inlineStr 的容器，本身没有内容，真正文本在内部的 t 节点。
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

// setXLSXCell 修改一个单元格的值或公式，保留其余 ZIP 部件与样式。
//
// 工作表可以用名称指定，也可以用从 1 开始的序号指定；为空时使用
// 第一个工作表。单元格不存在时会按列顺序插入，所在行不存在时会
// 新建整行，不会整体重排 XML。
func setXLSXCell(filePath, sheetName, ref, value string) error {
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
		// 未指定工作表时写入第一个，方便只处理单表文件的调用方。
		target = sheets[0].file
	} else if index, err := strconv.Atoi(sheetName); err == nil && index >= 1 && index <= len(sheets) {
		// 纯数字按从 1 开始的工作表序号处理。
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

// 用于定位与修改工作表 XML 中单元格、行与样式的正则表达式。
var (
	// cellPattern 匹配一个完整单元格，并捕获它的引用。
	cellPattern = regexp.MustCompile(`(?s)<c r="([A-Z]+[0-9]+)"[^>]*(?:/>|>.*?</c>)`)
	// rowPattern 匹配一整行，并捕获行号。
	rowPattern = regexp.MustCompile(`(?s)<row r="([0-9]+)"[^>]*>.*?</row>`)
	// stylePattern 从单元格标签中提取样式编号，用于写入时沿用原样式。
	stylePattern = regexp.MustCompile(`s="([0-9]+)"`)
)

// setCellXML 在单个工作表的 XML 文本中写入一个单元格。
//
// 这里用正则做定点替换而不是解析后整体重新序列化，目的是保留原有
// 命名空间、属性顺序与样式，只改动目标片段；代价是要求单元格与行
// 标签结构规整。处理顺序是：已有单元格则原位替换，否则在对应行中
// 按列顺序插入，再否则新建行，最后追加到 sheetData 末尾。
func setCellXML(sheetXML, ref, value string) (string, error) {
	// 先找出目标单元格原有的样式编号，写入时继续沿用。
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

	// 没有现成单元格：找到目标行，在其中按列顺序插入。
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

	// 连目标行都不存在：新建一行，并按行号顺序放到正确位置。
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
	// 所有现有行的行号都更小，直接追加到 sheetData 末尾。
	return sheetXML[:closeIndex] + newRow + sheetXML[closeIndex:], nil
}

// insertCellIntoRow 把单元格插入到一行中按列顺序正确的位置。
//
// 遇到第一列号更大的单元格就插在它前面；都不更大时插到 </row> 之前，
// 也就是该行末尾。
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

// buildCellXML 生成一个单元格的 XML 文本。
//
// 按值的形态选择写法：以等号开头按公式写入；能被解析为数字的按数值
// 写入；空字符串写成自闭合空单元格；其余写成内联字符串。使用
// inlineStr 而不是共享字符串，是为了避免同时改写 sharedStrings.xml。
func buildCellXML(ref, style, value string) string {
	escaped := escapeXMLText(value)
	if strings.HasPrefix(value, "=") {
		// 等号只是用户表达公式的写法，XML 的 f 节点里不包含它。
		return fmt.Sprintf(`<c r="%s"%s><f>%s</f></c>`, ref, style, escapeXMLText(strings.TrimPrefix(value, "=")))
	}
	if _, err := strconv.ParseFloat(strings.TrimSpace(value), 64); err == nil && strings.TrimSpace(value) != "" {
		return fmt.Sprintf(`<c r="%s"%s><v>%s</v></c>`, ref, style, strings.TrimSpace(value))
	}
	if value == "" {
		return fmt.Sprintf(`<c r="%s"%s/>`, ref, style)
	}
	// xml:space="preserve" 保证首尾空格不会被 Excel 丢弃。
	return fmt.Sprintf(`<c r="%s"%s t="inlineStr"><is><t xml:space="preserve">%s</t></is></c>`, ref, style, escaped)
}

// attrValue 返回 XML 起始标签中指定名称的属性值，不存在时返回空串。
func attrValue(start xml.StartElement, name string) string {
	for _, attr := range start.Attr {
		if attr.Name.Local == name {
			return attr.Value
		}
	}
	return ""
}

// attrInt 返回 XML 起始标签中指定属性的整数值，缺失或无法解析时返回 0。
func attrInt(start xml.StartElement, name string) int {
	number, _ := strconv.Atoi(attrValue(start, name))
	return number
}

// cellRow 从单元格引用中取出行号，例如 AB12 返回 12。
//
// 只拼接数字字符，因此不要求引用已经通过格式校验。
func cellRow(ref string) int {
	number := 0
	for _, char := range ref {
		if char >= '0' && char <= '9' {
			number = number*10 + int(char-'0')
		}
	}
	return number
}

// columnIndex 把列字母转换成从 1 开始的列号，用于排序与比较位置。
//
// 这是 26 进制但每一位从 1 开始计数：A=1、Z=26、AA=27、AB=28。
// 遇到非大写字母时停止，因此传入完整引用也能工作。
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
