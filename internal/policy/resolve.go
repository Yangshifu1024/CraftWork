// 本文件负责把用户或模型给出的路径安全地解析到工作区内：先做词法校验挡住
// ../ 越界，再解析符号链接挡住通过链接逃逸，文件不存在时解析其父目录。
// 所有路径校验都在 Go 侧完成，不信任前端传来的路径。

package policy

import (
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
)

// ErrOutsideWorkspace 表示路径解析后落在全部工作区目录之外。
var ErrOutsideWorkspace = errors.New("路径超出工作区及附加目录")

// Resolved 是多根解析的结果：命中哪个根、解析后的绝对路径与相对该根的展示路径。
type Resolved struct {
	// RootIndex 是命中的根在传入列表中的下标，0 即主要目录。
	RootIndex int
	// Root 是命中根归一化（EvalSymlinks）之后的真实绝对路径。
	Root string
	// Abs 是解析后的真实绝对路径，调用方应使用它访问文件系统。
	Abs string
	// Rel 是相对命中根的展示路径。
	Rel string
}

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
	lexRoot, realRoot, err := evalRoot(workspace)
	if err != nil {
		return "", err
	}
	return resolveAt(lexRoot, realRoot, rel)
}

// ResolveAny 把路径解析到多个源文件夹之一，第一个根是主要目录。
//
// 解析按根顺序进行：取第一个词法命中且符号链接校验通过的根；链接逃逸
// 直接拒绝，不尝试后续根。某个根不存在或不可达时跳过该根（例如附加目录
// 事后被删除），只有全部根都不可用或路径都未命中时才报错。rel 为空串时
// 返回第一个可用根本身。
func ResolveAny(roots []string, rel string) (Resolved, error) {
	return resolveMulti(roots, rel, false)
}

// ResolveExisting 在 ResolveAny 的基础上优先命中目标真实存在的根。
//
// 适用「路径必须已经存在」的场景（例如 git 的文件参数）：主根里同名路径
// 尚不存在、而附加根里真实存在时，应命中附加根而不是把主根的虚构路径
// 交给调用方。全部根都不存在时回落到词法判定（新建文件的合法场景）。
func ResolveExisting(roots []string, rel string) (Resolved, error) {
	return resolveMulti(roots, rel, true)
}

// resolveMulti 是 ResolveAny 与 ResolveExisting 的共同实现。
//
// preferExisting 为 true 时，词法命中的结果先不返回，等到 Stat 确认目标
// 真实存在才命中；全程没有真实存在的目标时，返回第一个词法命中的根。
func resolveMulti(roots []string, rel string, preferExisting bool) (Resolved, error) {
	var lexical Resolved
	haveLexical := false
	for index, workspace := range roots {
		if workspace == "" {
			continue
		}
		lexRoot, realRoot, err := evalRoot(workspace)
		if err != nil {
			// 主要目录不可用仍要报错（没有它工作区失去意义）；附加目录
			// 不可用则跳过，不拖垮整体解析。
			if index == 0 {
				return Resolved{}, err
			}
			continue
		}
		abs, err := resolveAt(lexRoot, realRoot, rel)
		// 未命中（越界）或路径在该根下不存在（例如主根没有这个目录，而
		// 附加根有）都继续尝试下一根；其余错误（权限等）视为硬失败。
		if errors.Is(err, ErrOutsideWorkspace) || errors.Is(err, fs.ErrNotExist) {
			continue
		}
		if err != nil {
			return Resolved{}, err
		}
		resolved := Resolved{RootIndex: index, Root: realRoot, Abs: abs, Rel: relativeTo(realRoot, abs)}
		if !preferExisting {
			return resolved, nil
		}
		if _, statErr := os.Stat(abs); statErr == nil {
			return resolved, nil
		}
		if !haveLexical {
			lexical, haveLexical = resolved, true
		}
	}
	if haveLexical {
		return lexical, nil
	}
	return Resolved{}, ErrOutsideWorkspace
}

// relativeTo 返回 abs 相对 root 的展示路径；越界或计算失败时原样返回。
func relativeTo(root, abs string) string {
	rel, err := filepath.Rel(root, abs)
	if err != nil || strings.HasPrefix(rel, "..") {
		return abs
	}
	return rel
}

// evalRoot 返回工作区目录的两个绝对形式：lexRoot 用于词法校验（保留符号
// 链接别名，绝对路径以别名形式给出时仍能通过词法检查），realRoot 用于
// 最终比较（解析符号链接后的真实路径，例如 macOS 的 /var 与 /private/var）。
func evalRoot(workspace string) (string, string, error) {
	absRoot, err := filepath.Abs(workspace)
	if err != nil {
		return "", "", fmt.Errorf("解析工作区: %w", err)
	}
	realRoot, err := filepath.EvalSymlinks(absRoot)
	if err != nil {
		return "", "", fmt.Errorf("解析工作区符号链接: %w", err)
	}
	return absRoot, realRoot, nil
}

// resolveAt 在单个根目录内解析路径，是 Resolve 与 ResolveAny 共用的核心：
// 词法校验（基于 lexRoot）挡住 ../ 越界，符号链接解析挡住链接逃逸，目标
// 不存在时解析其父目录再拼回文件名，最终与 realRoot 比较确认未逃逸。
func resolveAt(lexRoot, realRoot, rel string) (string, error) {
	// 相对路径以工作区为根拼接；绝对路径保持原样；最后清理 . 与 .. 等
	// 路径片段，得到词法上的规范形式。
	candidate := rel
	if !filepath.IsAbs(candidate) {
		candidate = filepath.Join(lexRoot, candidate)
	}
	candidate = filepath.Clean(candidate)

	// Windows UNC 路径与设备路径直接拒绝。
	if strings.HasPrefix(candidate, `\\`) || strings.HasPrefix(candidate, `//`) {
		return "", ErrOutsideWorkspace
	}

	// 先做词法校验：即使目标不存在，也不能接受 ../ 之类的越界路径。
	if !within(lexRoot, candidate) {
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
	if !within(realRoot, resolved) {
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
