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
func Resolve(workspace, rel string) (string, error) {
	if workspace == "" {
		return "", errors.New("工作区未设置")
	}
	root, err := filepath.Abs(workspace)
	if err != nil {
		return "", fmt.Errorf("解析工作区: %w", err)
	}
	root, err = filepath.EvalSymlinks(root)
	if err != nil {
		return "", fmt.Errorf("解析工作区符号链接: %w", err)
	}

	candidate := rel
	if !filepath.IsAbs(candidate) {
		candidate = filepath.Join(root, candidate)
	}
	candidate = filepath.Clean(candidate)

	// Windows UNC 路径与设备路径直接拒绝。
	if strings.HasPrefix(candidate, `\\`) || strings.HasPrefix(candidate, `//`) {
		return "", ErrOutsideWorkspace
	}

	// 先做词法校验：即使目标不存在，也不能接受 ../ 之类的越界路径。
	if !within(root, candidate) {
		return "", ErrOutsideWorkspace
	}

	resolved, err := filepath.EvalSymlinks(candidate)
	if err != nil {
		if !os.IsNotExist(err) {
			return "", fmt.Errorf("解析路径: %w", err)
		}
		parent, parentErr := filepath.EvalSymlinks(filepath.Dir(candidate))
		if parentErr != nil {
			return "", fmt.Errorf("解析父目录: %w", parentErr)
		}
		resolved = filepath.Join(parent, filepath.Base(candidate))
	}

	if !within(root, resolved) {
		return "", ErrOutsideWorkspace
	}
	return resolved, nil
}

// Relative 返回路径相对工作区的展示形式。越界路径原样返回。
func Relative(workspace, path string) string {
	rel, err := filepath.Rel(workspace, path)
	if err != nil || strings.HasPrefix(rel, "..") {
		return path
	}
	return rel
}

func within(root, path string) bool {
	if path == root {
		return true
	}
	return strings.HasPrefix(path, root+string(os.PathSeparator))
}
