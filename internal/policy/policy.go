// Package policy 负责工具风险分级、审批档位与路径安全校验。
//
// 三道防线互相独立：工具自身做参数校验，策略引擎决定是否需要审批，
// 路径解析拒绝任何越界访问。前端传入的路径一律视为不可信。
//
// 本文件定义风险等级、审批档位与处置动作的判断规则，并给出命令的
// 破坏性与网络访问分类；路径解析的实现放在同包的 resolve.go。
package policy

import (
	"strconv"
	"strings"
)

// Risk 是工具调用的风险等级。
type Risk int

// 风险等级从只读到破坏性递增，数字越大限制越严。
const (
	// R0 只读：自动允许并记录审计。
	R0 Risk = iota
	// R1 写工作区：每次询问（文档写入必须每次）。
	R1
	// R2 执行：每次询问，或按命令前缀允许。
	R2
	// R3 网络或越界：每次询问，不提供永久允许。
	R3
	// R4 破坏性：阻止，双重确认也不放行。
	R4
)

// String 返回 R0–R4。
func (r Risk) String() string {
	return "R" + strconv.Itoa(int(r))
}

// Level 是审批档位。
type Level string

// 审批档位决定不同风险等级的默认处置方式。
const (
	// LevelGraded 分级：R0 自动；R1–R3 询问；R4 阻止。
	LevelGraded Level = "graded"
	// LevelStrict 严格：写入与执行逐次询问，不提供永久允许。
	LevelStrict Level = "strict"
	// LevelReadonly 只读：只允许 R0。
	LevelReadonly Level = "readonly"
	// LevelFree 自由：放行所有操作，包括 R4；切换时前端必须先弹出风险确认。
	LevelFree Level = "free"
)

// ParseLevel 把前端字符串转换为审批档位，未知值回退到分级。
func ParseLevel(value string) Level {
	switch Level(value) {
	case LevelStrict:
		return LevelStrict
	case LevelReadonly:
		return LevelReadonly
	case LevelFree:
		return LevelFree
	default:
		return LevelGraded
	}
}

// Action 是策略引擎对一次调用的处置。
type Action string

// 策略引擎对一次调用的三种可能处置。
const (
	// ActionAllow 直接执行。
	ActionAllow Action = "allow"
	// ActionAsk 挂起等待用户审批。
	ActionAsk Action = "ask"
	// ActionDeny 直接拒绝。
	ActionDeny Action = "deny"
)

// Assessment 是一次工具调用的策略结论。
type Assessment struct {
	// Risk 是本次调用最终判定的风险等级。
	Risk Risk `json:"risk"`
	// Reason 是给用户看的判定理由，会出现在审批卡上。
	Reason string `json:"reason"`
	// Target 是审批展示的目标，通常是命令或解析后的路径。
	Target string `json:"target,omitempty"`
}

// Engine 根据审批档位、工作区与信任状态判断工具调用。
type Engine struct {
	// level 是当前审批档位。
	level Level
	// workspace 是当前工作区根目录的绝对路径。
	workspace string
	// restricted 表示工作区未受信任，网络访问直接拒绝。
	restricted bool
}

// New 创建策略引擎。workspace 必须是已经解析过的绝对路径。
func New(level Level, workspace string) *Engine {
	return &Engine{level: level, workspace: workspace}
}

// Level 返回当前审批档位。
func (e *Engine) Level() Level {
	return e.level
}

// SetLevel 更新审批档位。
func (e *Engine) SetLevel(level Level) {
	e.level = level
}

// Restricted 返回工作区是否处于受限模式。
func (e *Engine) Restricted() bool {
	return e.restricted
}

// SetRestricted 设置工作区信任状态。受限模式下网络访问直接拒绝，
// 且不提供任何永久授权。
func (e *Engine) SetRestricted(restricted bool) {
	e.restricted = restricted
}

// Workspace 返回当前工作区根目录。
func (e *Engine) Workspace() string {
	return e.workspace
}

// SetWorkspace 更新工作区根目录。
func (e *Engine) SetWorkspace(workspace string) {
	e.workspace = workspace
}

// ToolMeta 是评估一次工具调用所需的静态元数据，由工具自己声明。
type ToolMeta struct {
	// Name 是工具名称。
	Name string
	// Risk 是工具的默认风险等级。
	Risk Risk
	// PathArg 是需要解析的路径参数名，空表示没有。
	PathArg string
	// CommandArg 是需要分类的命令参数名，空表示没有。
	CommandArg string
}

// ReasonFor 返回风险等级的默认说明。
func ReasonFor(risk Risk) string {
	switch risk {
	case R0:
		return "只读操作"
	case R1:
		return "写入工作区"
	case R2:
		return "执行命令"
	case R3:
		return "网络或越界访问"
	default:
		return "破坏性操作"
	}
}

// ClassifyCommand 对 Shell 命令做风险分类。
func ClassifyCommand(command string) (Risk, string) {
	if strings.TrimSpace(command) == "" {
		return R2, "执行命令"
	}
	if isDestructiveCommand(command) {
		return R4, "包含破坏性命令"
	}
	if isNetworkCommand(command) {
		return R3, "命令可能访问网络"
	}
	return R2, "在工作区内执行命令"
}

// Action 根据审批档位与信任状态给出处置。
func (e *Engine) Action(risk Risk) Action {
	// 自由档位放行一切（包括 R4 与受限工作区的 R3）；
	// 这是用户在风险确认后做出的显式选择，策略层不再拦截，
	// 安全完全依赖审计记录与写入前备份兜底。
	if e.level == LevelFree {
		return ActionAllow
	}
	if e.level == LevelReadonly {
		if risk == R0 {
			return ActionAllow
		}
		return ActionDeny
	}
	// 受限工作区默认断网：R3 直接拒绝，而不是交给用户审批。
	if e.restricted && risk == R3 {
		return ActionDeny
	}
	if e.level == LevelStrict {
		if risk == R0 {
			return ActionAllow
		}
		if risk == R4 {
			return ActionDeny
		}
		return ActionAsk
	}
	// LevelGraded
	switch risk {
	case R0:
		return ActionAllow
	case R1, R2, R3:
		return ActionAsk
	default:
		return ActionDeny
	}
}

// AllowsPermanent 判断该风险是否允许“本会话 / 本工作区”授权。
// R3、R4 永远不提供永久允许；严格、只读与受限工作区也不提供。
func (e *Engine) AllowsPermanent(risk Risk) bool {
	// 只有分级档位且工作区受信任时，写工作区与执行命令才允许
	// 一次授权覆盖本会话或本工作区；其余情况每次都问。
	if e.level != LevelGraded || e.restricted {
		return false
	}
	return risk == R1 || risk == R2
}

// AllowsWrite 判断当前档位是否允许写操作。
func (e *Engine) AllowsWrite() bool {
	return e.level != LevelReadonly
}

// IsWorkspaceTrusted 判断工作区信任状态。受限工作区只允许只读工具。
func IsWorkspaceTrusted(trust string) bool {
	return trust == "trusted"
}

// destructivePatterns 命中后直接判 R4，不提供任何放行入口。
var destructivePatterns = []string{
	"rm -rf /", "rm -rf ~", "rm -rf .", "mkfs", "dd if=",
	"chmod 777", "chmod -r 777", "chown -r", "> /dev/",
	":(){:|:&};:", "shutdown", "reboot", "halt",
	"git push --force", "git reset --hard", "git clean -fdx",
}

// isDestructiveCommand 判断命令是否包含破坏性特征。
//
// 采用小写子串匹配，宁可误判也不放过；一旦命中按 R4 阻止执行。
func isDestructiveCommand(command string) bool {
	lower := strings.ToLower(command)
	if strings.Contains(lower, "sudo ") {
		return true
	}
	for _, pattern := range destructivePatterns {
		if strings.Contains(lower, pattern) {
			return true
		}
	}
	return false
}

// networkCommands 是可能访问网络的命令与包管理器前缀。
var networkCommands = []string{
	"curl ", "wget ", "ssh ", "scp ", "sftp ", "nc ", "netcat ",
	"npm install", "npm i ", "pnpm add", "yarn add", "go get", "pip install",
	"docker pull", "git clone", "git fetch", "git pull", "git push",
}

// isNetworkCommand 判断命令是否可能访问网络。
//
// 同样是子串匹配，覆盖常见下载、远程登录与依赖安装命令；
// 命中后按 R3 处理，受限工作区会直接拒绝。
func isNetworkCommand(command string) bool {
	lower := strings.ToLower(command)
	for _, prefix := range networkCommands {
		if strings.Contains(lower, prefix) {
			return true
		}
	}
	return false
}
