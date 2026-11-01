// 本文件负责把用户或模型给出的路径安全地解析到工作区内：先做词法校验挡住
// ../ 越界，再解析符号链接挡住通过链接逃逸，文件不存在时解析其父目录。
// 所有路径校验都在 Go 侧完成，不信任前端传来的路径。

package policy

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// ErrOutsideWorkspace 表示路径解析后落在工作区之外。
var ErrOutsideWorkspace = errors.New("路径超出工作区")

// Resolve 把用户或模型给出的相对路径解析为工作区内的真实绝对路径。
//
// 规则：
//   - 相对路径以工作区为根拼接；
//   - 绝对路径必须位于工作区内；
//   - 解析符号链接后再次校验，阻止通过链接逃逸；
//   - 文件尚不存在时解析其父目录。
//
// 返回的是解析后的真实绝对路径，调用方应使用返回值访问文件，而不是原始输入。
func Resolve(workspace, rel string) (string, error) {
	if workspace == "" {
		return "", errors.New("工作区未设置")
	}
	// 工作区先转绝对路径，再解析符号链接，得到用于最终比较的真实根目录。
	absRoot, err := filepath.Abs(workspace)
	if err != nil {
		return "", fmt.Errorf("解析工作区: %w", err)
	}
	root, err := filepath.EvalSymlinks(absRoot)
	if err != nil {
		return "", fmt.Errorf("解析工作区符号链接: %w", err)
	}

	// 相对路径以工作区为根拼接；绝对路径保持原样；最后清理 . 与 .. 等
	// 路径片段，得到词法上的规范形式。
	candidate := rel
	if !filepath.IsAbs(candidate) {
		candidate = filepath.Join(absRoot, candidate)
	}
	candidate = filepath.Clean(candidate)

	// Windows UNC 路径与设备路径直接拒绝。
	if strings.HasPrefix(candidate, `\\`) || strings.HasPrefix(candidate, `//`) {
		return "", ErrOutsideWorkspace
	}

	// 先做词法校验：即使目标不存在，也不能接受 ../ 之类的越界路径。
	if !within(absRoot, candidate) {
		return "", ErrOutsideWorkspace
	}

	// 再解析符号链接，防止通过链接逃逸。绝对路径可能使用未解析的工作区
	// 别名（例如 macOS 的 /var 与 /private/var），因此最终以解析结果为准。
	resolved, err := filepath.EvalSymlinks(candidate)
	if err != nil {
		if !os.IsNotExist(err) {
			return "", fmt.Errorf("解析路径: %w", err)
		}
		// 目标尚不存在（例如即将新建的文件）：解析父目录的真实路径，再把
		// 文件名拼回去；父目录也不存在时视为无法解析。
		parent, parentErr := filepath.EvalSymlinks(filepath.Dir(candidate))
		if parentErr != nil {
			return "", fmt.Errorf("解析父目录: %w", parentErr)
		}
		resolved = filepath.Join(parent, filepath.Base(candidate))
	}

	// 用解析后的真实路径与真实根目录比较，符号链接逃逸会在这里被拦下。
	if !within(root, resolved) {
		return "", ErrOutsideWorkspace
	}
	return resolved, nil
}

// Relative 返回路径相对工作区的展示形式。越界路径与计算失败时原样返回，
// 避免界面展示出误导性的相对路径。
func Relative(workspace, path string) string {
	rel, err := filepath.Rel(workspace, path)
	if err != nil || strings.HasPrefix(rel, "..") {
		return path
	}
	return rel
}

// within 判断路径是否等于根目录或位于根目录之下。
//
// 比较时在根目录后补一个路径分隔符，避免 /tmp/work 把 /tmp/work-other 误判为
// 子路径；调用方应传入已清理的绝对路径。
func within(root, path string) bool {
	if path == root {
		return true
	}
	return strings.HasPrefix(path, root+string(os.PathSeparator))
}
