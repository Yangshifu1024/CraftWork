// 本文件定义存储层的数据结构：线程、工作区、消息与部件、事件、运行、审批、
// 审计、用量、备份与快照。
//
// 这些结构同时是返回给前端的 JSON 形状，字段标签与 internal/events 的载荷保持一致。

package store

// Thread 是一个对话线程。mode 取 code / work。
type Thread struct {
	// ID 是线程的唯一标识。
	ID string `json:"id"`
	// Mode 是线程所属模式，取值为 code 或 work。
	Mode string `json:"mode"`
	// Title 是侧边栏展示的标题。
	Title string `json:"title"`
	// ModelID 是当前使用的模型标识。
	ModelID string `json:"modelId"`
	// WorkspaceID 是绑定的工作区标识，可为空。
	WorkspaceID string `json:"workspaceId"`
	// Status 是线程运行状态，取值为 idle / running / awaiting / error。
	Status string `json:"status"` // idle / running / awaiting / error
	// Archived 表示线程是否已归档。
	Archived bool `json:"archived"`
	// Tokens 是该线程累计消耗的 token 数。
	Tokens int64 `json:"tokens"`
	// CreatedAt 是创建时间，毫秒时间戳。
	CreatedAt int64 `json:"createdAt"`
	// UpdatedAt 是最近活动时间，毫秒时间戳。
	UpdatedAt int64 `json:"updatedAt"`
}

// Workspace 是用户选择的本地目录根。
type Workspace struct {
	// ID 是工作区的唯一标识。
	ID string `json:"id"`
	// Name 是展示名称，默认取目录名。
	Name string `json:"name"`
	// Path 是目录的绝对路径，在表中唯一。
	Path string `json:"path"`
	// Trust 是信任级别，取值为 trusted / restricted。
	Trust string `json:"trust"` // trusted / restricted
	// CreatedAt 是首次添加时间，毫秒时间戳。
	CreatedAt int64 `json:"createdAt"`
	// LastOpenedAt 是最近打开时间，用于排序。
	LastOpenedAt int64 `json:"lastOpenedAt"`
}

// Message 是从事件表重建出来的消息，直接返回给前端。
type Message struct {
	// ID 是消息标识，同时用于把事件归组。
	ID string `json:"id"`
	// ThreadID 是所属线程。
	ThreadID string `json:"threadId"`
	// Role 是消息角色，取值为 user / assistant。
	Role string `json:"role"` // user / assistant
	// Status 是消息状态，取值为 streaming / complete / incomplete / error。
	Status string `json:"status"` // streaming / complete / incomplete / error
	// CreatedAt 是消息创建时间，毫秒时间戳。
	CreatedAt int64 `json:"createdAt"`
	// Parts 是消息中的内容块，按事件序号排列。
	Parts []Part `json:"parts"`
}

// Part 是消息中的一个内容块。Data 的具体形状由 Kind 决定，
// 与 internal/events 中的事件载荷保持一致。
type Part struct {
	// ID 是部件的唯一标识，也是对应事件的 ID。
	ID string `json:"id"`
	// Kind 是部件类型，例如 text、reasoning、tool、approval。
	Kind string `json:"kind"`
	// Seq 是事件在线程内的序号，前端用它排序与断点续传。
	Seq int64 `json:"seq"`
	// Data 是部件的具体内容，形状由 Kind 决定。
	Data any `json:"data"`
}

// EventRecord 是持久化事件的输入。
type EventRecord struct {
	// ID 是事件标识，通常与部件 ID 相同。
	ID string
	// ThreadID 是事件所属线程，序号按线程分配。
	ThreadID string
	// MessageID 是事件归属的消息。
	MessageID string
	// RunID 是产生事件的那次运行，可为空。
	RunID string
	// Kind 是事件类型，决定重建后的部件类型。
	Kind string
	// Payload 是事件内容，写入时序列化为 JSON 文本。
	Payload any
	// CreatedAt 是创建时间，毫秒时间戳；为零时由写入方补当前时间。
	CreatedAt int64
}

// Run 是一次 Agent 执行。
type Run struct {
	// ID 是运行的唯一标识。
	ID string `json:"id"`
	// ThreadID 是所属线程。
	ThreadID string `json:"threadId"`
	// MessageID 是本次运行产出回复对应的消息。
	MessageID string `json:"messageId"`
	// Status 是运行状态，取值为 running / done / canceled / error / interrupted。
	Status string `json:"status"` // running / done / canceled / error / interrupted
	// ModelID 是本次运行使用的模型。
	ModelID string `json:"modelId"`
	// StartedAt 是开始时间，毫秒时间戳。
	StartedAt int64 `json:"startedAt"`
	// FinishedAt 是结束时间；仍在运行时为零。
	FinishedAt int64 `json:"finishedAt,omitempty"`
	// ErrorCode 是失败时的统一错误码。
	ErrorCode string `json:"errorCode,omitempty"`
	// ErrorMessage 是失败时展示给用户的错误说明。
	ErrorMessage string `json:"errorMessage,omitempty"`
}

// Approval 是一次工具审批。
type Approval struct {
	// ID 是审批的唯一标识。
	ID string `json:"id"`
	// ThreadID 是所属线程。
	ThreadID string `json:"threadId"`
	// RunID 是触发审批的那次运行。
	RunID string `json:"runId"`
	// ToolCallID 是模型给出的工具调用标识。
	ToolCallID string `json:"toolCallId"`
	// Tool 是工具名称。
	Tool string `json:"tool"`
	// Risk 是风险评估等级的文字形式。
	Risk string `json:"risk"`
	// Decision 是用户决定：空表示等待中，取值为 allow / deny / canceled。
	Decision string `json:"decision"` // 空表示等待中；allow / deny / canceled
	// Scope 是授权范围，取值为 once / session / workspace。
	Scope string `json:"scope"` // once / session / workspace
	// RequestedAt 是发起审批的时间，毫秒时间戳。
	RequestedAt int64 `json:"requestedAt"`
	// DecidedAt 是作出决定的时间；未决定时为零。
	DecidedAt int64 `json:"decidedAt,omitempty"`
}

// AuditEntry 是不可变的审计记录。
type AuditEntry struct {
	// ID 是自增主键，由数据库生成。
	ID int64 `json:"id"`
	// TS 是记录时间，毫秒时间戳。
	TS int64 `json:"ts"`
	// ThreadID 是所属线程，可为空。
	ThreadID string `json:"threadId"`
	// RunID 是所属运行，可为空。
	RunID string `json:"runId"`
	// Tool 是工具名称。
	Tool string `json:"tool"`
	// Risk 是风险评估等级。
	Risk string `json:"risk"`
	// Decision 是最终决定，取值为 allow / deny / canceled。
	Decision string `json:"decision"`
	// Scope 是授权范围。
	Scope string `json:"scope"`
	// Detail 是补充说明，例如操作摘要或“用户审批”。
	Detail string `json:"detail"`
}

// UsageRow 是一次模型调用的用量。
type UsageRow struct {
	// ID 是自增主键，由数据库生成。
	ID int64 `json:"id"`
	// TS 是记录时间，毫秒时间戳。
	TS int64 `json:"ts"`
	// ThreadID 是所属线程。
	ThreadID string `json:"threadId"`
	// ModelID 是实际使用的模型标识。
	ModelID string `json:"modelId"`
	// InputTokens 是输入 token 数。
	InputTokens int64 `json:"inputTokens"`
	// OutputTokens 是输出 token 数。
	OutputTokens int64 `json:"outputTokens"`
	// CacheTokens 是命中缓存的 token 数。
	CacheTokens int64 `json:"cacheTokens"`
	// ReasoningTokens 是推理过程消耗的 token 数。
	ReasoningTokens int64 `json:"reasoningTokens"`
	// CostMicros 是按模型价格换算的费用，单位是百万分之一货币单位。
	CostMicros int64 `json:"costMicros"`
}

// UsageSummary 是设置页与审计面板使用的聚合。
type UsageSummary struct {
	// Today 是今天的合计。
	Today UsageTotals `json:"today"`
	// Week 是最近七天的合计。
	Week UsageTotals `json:"week"`
	// Month 是最近三十天的合计。
	Month UsageTotals `json:"month"`
	// ByModel 是按模型拆分的用量。
	ByModel []ModelUsage `json:"byModel"`
	// ByDay 是按天拆分的用量。
	ByDay []DailyUsage `json:"byDay"`
}

// UsageTotals 是某个时间段的合计。
type UsageTotals struct {
	// InputTokens 是输入 token 合计。
	InputTokens int64 `json:"inputTokens"`
	// OutputTokens 是输出 token 合计。
	OutputTokens int64 `json:"outputTokens"`
	// CacheTokens 是缓存 token 合计。
	CacheTokens int64 `json:"cacheTokens"`
	// CostMicros 是费用合计。
	CostMicros int64 `json:"costMicros"`
	// Runs 是模型调用次数。
	Runs int64 `json:"runs"`
}

// ModelUsage 是按模型聚合的用量。
type ModelUsage struct {
	// ModelID 是模型标识。
	ModelID string `json:"modelId"`
	// InputTokens 是输入 token 合计。
	InputTokens int64 `json:"inputTokens"`
	// OutputTokens 是输出 token 合计。
	OutputTokens int64 `json:"outputTokens"`
	// CostMicros 是费用合计。
	CostMicros int64 `json:"costMicros"`
	// Runs 是调用次数。
	Runs int64 `json:"runs"`
}

// DailyUsage 是按天聚合的用量。
type DailyUsage struct {
	// Day 是日期，格式为 YYYY-MM-DD。
	Day string `json:"day"`
	// InputTokens 是当天输入 token 合计。
	InputTokens int64 `json:"inputTokens"`
	// OutputTokens 是当天输出 token 合计。
	OutputTokens int64 `json:"outputTokens"`
	// CostMicros 是当天费用合计。
	CostMicros int64 `json:"costMicros"`
	// Runs 是当天调用次数。
	Runs int64 `json:"runs"`
}

// Backup 是一条文件写入前的备份记录。
type Backup struct {
	// ID 是备份的唯一标识。
	ID string `json:"id"`
	// Path 是被备份文件的原始路径。
	Path string `json:"path"`
	// BackupPath 是备份文件在备份目录中的路径。
	BackupPath string `json:"backupPath"`
	// Hash 是备份内容的哈希，用于校验与去重。
	Hash string `json:"hash"`
	// Size 是备份文件大小，单位是字节。
	Size int64 `json:"size"`
	// CreatedAt 是备份时间，毫秒时间戳。
	CreatedAt int64 `json:"createdAt"`
}

// Snapshot 是线程的完整视图：线程本身、消息与事件序号。
type Snapshot struct {
	// Thread 是线程本身的属性。
	Thread Thread `json:"thread"`
	// Messages 是按创建顺序排列的消息，每条消息都带有部件。
	Messages []Message `json:"messages"`
	// LastSeq 是当前线程最大的事件序号，前端据此接续增量事件。
	LastSeq int64 `json:"lastSeq"`
	// Approvals 是线程内尚未决定的审批。
	Approvals []Approval `json:"approvals"`
	// HasMore 表示当前页之前还有更早的消息。
	HasMore bool `json:"hasMore"`
	// OldestSeq 是当前页第一条消息中最小的部件序号，用于向前翻页。
	OldestSeq int64 `json:"oldestSeq"`
}
