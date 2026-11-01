// 本文件实现 MCP 的 stdio 客户端：启动服务器子进程，按行收发
// JSON-RPC 消息，并把并发请求按编号分发到各自的等待通道。
// 读取循环与进程退出都会关闭 done 通道，让所有等待中的调用及时返回。

package mcp

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os/exec"
	"sync"
	"sync/atomic"
	"time"

	"craftwork/internal/platform"
)

// 客户端的时间与容量限制：握手较快，普通请求更宽松，
// 单条消息最多 16 MB，防止异常服务器塞爆内存。
const (
	// initializeTimeout 是 MCP 握手的最长等待时间。
	initializeTimeout = 15 * time.Second
	// requestTimeout 是单次工具列表或工具调用的最长等待时间。
	requestTimeout = 120 * time.Second
	// maxMessageBytes 是单条 JSON-RPC 消息允许的最大字节数。
	maxMessageBytes = 16 << 20
)

// Client 是一个已启动的 MCP stdio 客户端。
type Client struct {
	// command 是服务器子进程，退出时用于清理进程组。
	command *exec.Cmd
	// stdin 是向服务器写入 JSON-RPC 消息的管道。
	stdin io.WriteCloser
	// scanner 按行读取服务器返回的消息。
	scanner *bufio.Scanner
	// stderr 保存服务器标准错误，进程异常退出时用于提示原因。
	stderr *platform.LimitedBuffer

	// writeMu 串行化写入，保证每行消息完整且不交错。
	writeMu sync.Mutex
	// mu 保护 pending、closed 等共享状态。
	mu sync.Mutex
	// pending 按请求编号保存等待响应的通道。
	pending map[int64]chan rpcResponse
	// nextID 是单调递增的请求编号。
	nextID int64
	// closed 表示连接已经关闭，不再接受新请求。
	closed bool
	// done 在连接结束或进程退出时关闭，用于唤醒所有等待者。
	done chan struct{}
	// closeOnce 保证关闭逻辑只执行一次。
	closeOnce sync.Once
}

// rpcRequest 是发往服务器的 JSON-RPC 请求；通知（notify）不填 ID。
type rpcRequest struct {
	// JSONRPC 固定为 2.0。
	JSONRPC string `json:"jsonrpc"`
	// ID 是请求编号，通知消息省略。
	ID int64 `json:"id,omitempty"`
	// Method 是 MCP 方法名，例如 tools/list。
	Method string `json:"method"`
	// Params 是方法参数。
	Params any `json:"params,omitempty"`
}

// rpcError 是服务器返回的 JSON-RPC 错误对象。
type rpcError struct {
	// Code 是协议错误码。
	Code int `json:"code"`
	// Message 是错误说明。
	Message string `json:"message"`
}

// Error 把协议错误转换成可读文本。
func (e *rpcError) Error() string {
	return fmt.Sprintf("MCP 错误 %d: %s", e.Code, e.Message)
}

// rpcResponse 是服务器返回的 JSON-RPC 响应，结果与错误二者其一。
type rpcResponse struct {
	// JSONRPC 固定为 2.0。
	JSONRPC string `json:"jsonrpc"`
	// ID 是响应对应的请求编号。
	ID json.RawMessage `json:"id"`
	// Result 是成功结果，由调用方自行解析。
	Result json.RawMessage `json:"result"`
	// Error 是失败信息，非空时忽略 Result。
	Error *rpcError `json:"error"`
}

// Start 启动 MCP 服务器进程并建立 JSON-RPC 通道。调用方需要随后调用 Initialize。
//
// 环境变量经过清洗，子进程独立成组，便于退出时整体回收。
func Start(ctx context.Context, config ServerConfig) (*Client, error) {
	if config.Command == "" {
		return nil, fmt.Errorf("MCP 服务器命令不能为空")
	}
	command := exec.Command(config.Command, config.Args...)
	command.Env = platform.SanitizedEnv()
	command.SysProcAttr = platform.ProcessGroupAttr()

	stdin, err := command.StdinPipe()
	if err != nil {
		return nil, fmt.Errorf("创建 MCP 标准输入: %w", err)
	}
	stdout, err := command.StdoutPipe()
	if err != nil {
		return nil, fmt.Errorf("创建 MCP 标准输出: %w", err)
	}
	stderr, err := command.StderrPipe()
	if err != nil {
		return nil, fmt.Errorf("创建 MCP 标准错误: %w", err)
	}
	if err := command.Start(); err != nil {
		return nil, fmt.Errorf("启动 MCP 服务器: %w", err)
	}

	client := &Client{
		command: command,
		stdin:   stdin,
		scanner: bufio.NewScanner(stdout),
		stderr:  platform.NewLimitedBuffer(8192),
		pending: map[int64]chan rpcResponse{},
		done:    make(chan struct{}),
	}
	// 初始缓冲 64 KB，上限 16 MB，避免长消息触发行长度限制。
	client.scanner.Buffer(make([]byte, 64*1024), maxMessageBytes)
	go client.readLoop()
	go func() {
		// 标准错误只保留末尾一段，避免大量日志占满内存。
		_, _ = io.Copy(client.stderr, stderr)
	}()
	go func() {
		// 进程退出后关闭 done，唤醒所有等待响应的调用。
		_ = command.Wait()
		client.closeDone()
	}()

	return client, nil
}

// Initialize 完成 MCP 握手。
//
// 先发送 initialize 声明协议版本与客户端信息，再发送 initialized
// 通知，表示客户端已准备好接收服务器消息。
func (c *Client) Initialize(ctx context.Context) error {
	timeoutCtx, cancel := context.WithTimeout(ctx, initializeTimeout)
	defer cancel()
	if _, err := c.call(timeoutCtx, "initialize", map[string]any{
		"protocolVersion": "2024-11-05",
		"capabilities":    map[string]any{},
		"clientInfo": map[string]any{
			"name":    "CraftWork",
			"version": "0.1.0",
		},
	}); err != nil {
		return fmt.Errorf("初始化 MCP 服务器失败: %w", err)
	}
	if err := c.notify("notifications/initialized", map[string]any{}); err != nil {
		return err
	}
	return nil
}

// ListTools 返回服务器暴露的工具。
func (c *Client) ListTools(ctx context.Context) ([]ToolInfo, error) {
	// 请求级超时独立于调用方上下文，防止服务器长时间不响应。
	timeoutCtx, cancel := context.WithTimeout(ctx, requestTimeout)
	defer cancel()
	raw, err := c.call(timeoutCtx, "tools/list", map[string]any{})
	if err != nil {
		return nil, err
	}
	var result struct {
		Tools []ToolInfo `json:"tools"`
	}
	if err := json.Unmarshal(raw, &result); err != nil {
		return nil, fmt.Errorf("解析 MCP 工具列表: %w", err)
	}
	return result.Tools, nil
}

// CallTool 调用一个 MCP 工具，返回文本输出与是否由工具报告错误。
//
// MCP 内容块可能是文本以外的类型，这里用 [类型] 占位，保持顺序可读。
func (c *Client) CallTool(ctx context.Context, name string, args map[string]any) (string, bool, error) {
	timeoutCtx, cancel := context.WithTimeout(ctx, requestTimeout)
	defer cancel()
	raw, err := c.call(timeoutCtx, "tools/call", map[string]any{
		"name":      name,
		"arguments": args,
	})
	if err != nil {
		return "", true, err
	}
	var result struct {
		Content []struct {
			Type string `json:"type"`
			Text string `json:"text"`
		} `json:"content"`
		IsError bool `json:"isError"`
	}
	if err := json.Unmarshal(raw, &result); err != nil {
		return "", true, fmt.Errorf("解析 MCP 工具结果: %w", err)
	}
	var builder bytes.Buffer
	for _, item := range result.Content {
		if item.Type == "text" {
			builder.WriteString(item.Text)
		} else {
			builder.WriteString("[" + item.Type + "]")
		}
	}
	output := builder.String()
	if output == "" {
		output = "MCP 工具没有返回文本内容"
	}
	return output, result.IsError, nil
}

// Close 关闭客户端并终止服务器进程组。
//
// 先关闭标准输入让服务器自行退出，再强制回收整个进程组，
// 保证服务器派生的子进程不会残留。
func (c *Client) Close() error {
	var closeErr error
	c.closeOnce.Do(func() {
		_ = c.stdin.Close()
		c.closeDone()
		if c.command.Process != nil {
			if err := platform.KillProcessGroup(c.command); err != nil {
				closeErr = err
			}
		}
	})
	return closeErr
}

// call 发送请求并等待对应编号的响应。
//
// 使用带缓冲的通道接收结果，读取循环即使在调用方超时后才收到响应
// 也不会阻塞。等待期间响应、上下文取消、连接结束三者谁先发生就返回谁。
func (c *Client) call(ctx context.Context, method string, params any) (json.RawMessage, error) {
	id := atomic.AddInt64(&c.nextID, 1)
	response := make(chan rpcResponse, 1)
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return nil, fmt.Errorf("MCP 连接已关闭")
	}
	c.pending[id] = response
	c.mu.Unlock()
	defer func() {
		c.mu.Lock()
		delete(c.pending, id)
		c.mu.Unlock()
	}()

	if err := c.write(rpcRequest{JSONRPC: "2.0", ID: id, Method: method, Params: params}); err != nil {
		return nil, err
	}
	select {
	case result := <-response:
		if result.Error != nil {
			return nil, result.Error
		}
		return result.Result, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-c.done:
		return nil, fmt.Errorf("MCP 服务器已退出: %s", c.stderr.String())
	}
}

// notify 发送不期待响应的 JSON-RPC 通知。
func (c *Client) notify(method string, params any) error {
	return c.write(rpcRequest{JSONRPC: "2.0", Method: method, Params: params})
}

// write 序列化一条消息并以换行结尾写入标准输入。
//
// MCP stdio 以行分隔消息，因此必须持有写锁，避免两条消息交错。
func (c *Client) write(message rpcRequest) error {
	data, err := json.Marshal(message)
	if err != nil {
		return err
	}
	data = append(data, '\n')
	c.writeMu.Lock()
	defer c.writeMu.Unlock()
	_, err = c.stdin.Write(data)
	return err
}

// readLoop 持续读取服务器输出，把响应分发给等待中的请求。
//
// 无法解析的行、通知消息与未知编号都会被忽略；扫描结束说明
// 服务器已经关闭输出，此时关闭 done 并唤醒所有等待者。
func (c *Client) readLoop() {
	for c.scanner.Scan() {
		line := bytes.TrimSpace(c.scanner.Bytes())
		if len(line) == 0 {
			continue
		}
		var response rpcResponse
		if err := json.Unmarshal(line, &response); err != nil {
			continue
		}
		if len(response.ID) == 0 || string(response.ID) == "null" {
			continue
		}
		var id int64
		if err := json.Unmarshal(response.ID, &id); err != nil {
			continue
		}
		c.mu.Lock()
		channel := c.pending[id]
		c.mu.Unlock()
		if channel != nil {
			select {
			case channel <- response:
			default:
			}
		}
	}
	c.closeDone()
}

// closeDone 标记连接已关闭并关闭 done 通道，重复调用安全。
func (c *Client) closeDone() {
	c.mu.Lock()
	if !c.closed {
		c.closed = true
		close(c.done)
	}
	c.mu.Unlock()
}
