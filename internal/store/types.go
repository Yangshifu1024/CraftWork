package store

// Thread 是一个对话线程。mode 取 code / work。
type Thread struct {
	ID          string `json:"id"`
	Mode        string `json:"mode"`
	Title       string `json:"title"`
	ModelID     string `json:"modelId"`
	WorkspaceID string `json:"workspaceId"`
	Status      string `json:"status"` // idle / running / awaiting / error
	Archived    bool   `json:"archived"`
	Tokens      int64  `json:"tokens"`
	CreatedAt   int64  `json:"createdAt"`
	UpdatedAt   int64  `json:"updatedAt"`
}

// Workspace 是用户选择的本地目录根。
type Workspace struct {
	ID           string `json:"id"`
	Name         string `json:"name"`
	Path         string `json:"path"`
	Trust        string `json:"trust"` // trusted / restricted
	CreatedAt    int64  `json:"createdAt"`
	LastOpenedAt int64  `json:"lastOpenedAt"`
}

// Message 是从事件表物化出来的消息，直接返回给前端。
type Message struct {
	ID        string `json:"id"`
	ThreadID  string `json:"threadId"`
	Role      string `json:"role"`   // user / assistant
	Status    string `json:"status"` // streaming / complete / incomplete / error
	CreatedAt int64  `json:"createdAt"`
	Parts     []Part `json:"parts"`
}

// Part 是消息中的一个内容块。Data 的具体形状由 Kind 决定，
// 与 internal/events 中的事件载荷保持一致。
type Part struct {
	ID   string `json:"id"`
	Kind string `json:"kind"`
	Seq  int64  `json:"seq"`
	Data any    `json:"data"`
}

// EventRecord 是持久化事件的输入。
type EventRecord struct {
	ID        string
	ThreadID  string
	MessageID string
	RunID     string
	Kind      string
	Payload   any
	CreatedAt int64
}

// Run 是一次 Agent 执行。
type Run struct {
	ID           string `json:"id"`
	ThreadID     string `json:"threadId"`
	MessageID    string `json:"messageId"`
	Status       string `json:"status"` // running / done / canceled / error / interrupted
	ModelID      string `json:"modelId"`
	StartedAt    int64  `json:"startedAt"`
	FinishedAt   int64  `json:"finishedAt,omitempty"`
	ErrorCode    string `json:"errorCode,omitempty"`
	ErrorMessage string `json:"errorMessage,omitempty"`
}

// Approval 是一次工具审批。
type Approval struct {
	ID          string `json:"id"`
	ThreadID    string `json:"threadId"`
	RunID       string `json:"runId"`
	ToolCallID  string `json:"toolCallId"`
	Tool        string `json:"tool"`
	Risk        string `json:"risk"`
	Decision    string `json:"decision"` // 空表示等待中；allow / deny / canceled
	Scope       string `json:"scope"`    // once / session / workspace
	RequestedAt int64  `json:"requestedAt"`
	DecidedAt   int64  `json:"decidedAt,omitempty"`
}

// AuditEntry 是不可变的审计记录。
type AuditEntry struct {
	ID       int64  `json:"id"`
	TS       int64  `json:"ts"`
	ThreadID string `json:"threadId"`
	RunID    string `json:"runId"`
	Tool     string `json:"tool"`
	Risk     string `json:"risk"`
	Decision string `json:"decision"`
	Scope    string `json:"scope"`
	Detail   string `json:"detail"`
}

// UsageRow 是一次模型调用的用量。
type UsageRow struct {
	ID              int64  `json:"id"`
	TS              int64  `json:"ts"`
	ThreadID        string `json:"threadId"`
	ModelID         string `json:"modelId"`
	InputTokens     int64  `json:"inputTokens"`
	OutputTokens    int64  `json:"outputTokens"`
	CacheTokens     int64  `json:"cacheTokens"`
	ReasoningTokens int64  `json:"reasoningTokens"`
	CostMicros      int64  `json:"costMicros"`
}

// UsageSummary 是设置页与审计面板使用的聚合。
type UsageSummary struct {
	Today   UsageTotals  `json:"today"`
	Week    UsageTotals  `json:"week"`
	Month   UsageTotals  `json:"month"`
	ByModel []ModelUsage `json:"byModel"`
	ByDay   []DailyUsage `json:"byDay"`
}

// UsageTotals 是某个时间段的合计。
type UsageTotals struct {
	InputTokens  int64 `json:"inputTokens"`
	OutputTokens int64 `json:"outputTokens"`
	CacheTokens  int64 `json:"cacheTokens"`
	CostMicros   int64 `json:"costMicros"`
	Runs         int64 `json:"runs"`
}

// ModelUsage 是按模型聚合的用量。
type ModelUsage struct {
	ModelID      string `json:"modelId"`
	InputTokens  int64  `json:"inputTokens"`
	OutputTokens int64  `json:"outputTokens"`
	CostMicros   int64  `json:"costMicros"`
	Runs         int64  `json:"runs"`
}

// DailyUsage 是按天聚合的用量。
type DailyUsage struct {
	Day          string `json:"day"`
	InputTokens  int64  `json:"inputTokens"`
	OutputTokens int64  `json:"outputTokens"`
	CostMicros   int64  `json:"costMicros"`
	Runs         int64  `json:"runs"`
}

// Backup 是一条文件写入前的备份记录。
type Backup struct {
	ID         string `json:"id"`
	Path       string `json:"path"`
	BackupPath string `json:"backupPath"`
	Hash       string `json:"hash"`
	Size       int64  `json:"size"`
	CreatedAt  int64  `json:"createdAt"`
}

// Snapshot 是线程的完整视图：线程本身、消息与事件序号。
type Snapshot struct {
	Thread    Thread     `json:"thread"`
	Messages  []Message  `json:"messages"`
	LastSeq   int64      `json:"lastSeq"`
	Approvals []Approval `json:"approvals"`
}
