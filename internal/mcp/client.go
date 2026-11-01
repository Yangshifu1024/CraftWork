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

const (
	initializeTimeout = 15 * time.Second
	requestTimeout    = 120 * time.Second
	maxMessageBytes   = 16 << 20
)

// Client 是一个已启动的 MCP stdio 客户端。
type Client struct {
	command *exec.Cmd
	stdin   io.WriteCloser
	scanner *bufio.Scanner
	stderr  *platform.LimitedBuffer

	writeMu   sync.Mutex
	mu        sync.Mutex
	pending   map[int64]chan rpcResponse
	nextID    int64
	closed    bool
	done      chan struct{}
	closeOnce sync.Once
}

type rpcRequest struct {
	JSONRPC string `json:"jsonrpc"`
	ID      int64  `json:"id,omitempty"`
	Method  string `json:"method"`
	Params  any    `json:"params,omitempty"`
}

type rpcError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
}

func (e *rpcError) Error() string {
	return fmt.Sprintf("MCP 错误 %d: %s", e.Code, e.Message)
}

type rpcResponse struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id"`
	Result  json.RawMessage `json:"result"`
	Error   *rpcError       `json:"error"`
}

// Start 启动 MCP 服务器进程并建立 JSON-RPC 通道。调用方需要随后调用 Initialize。
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
	client.scanner.Buffer(make([]byte, 64*1024), maxMessageBytes)
	go client.readLoop()
	go func() {
		_, _ = io.Copy(client.stderr, stderr)
	}()
	go func() {
		_ = command.Wait()
		client.closeDone()
	}()

	return client, nil
}

// Initialize 完成 MCP 握手。
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

func (c *Client) notify(method string, params any) error {
	return c.write(rpcRequest{JSONRPC: "2.0", Method: method, Params: params})
}

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

func (c *Client) closeDone() {
	c.mu.Lock()
	if !c.closed {
		c.closed = true
		close(c.done)
	}
	c.mu.Unlock()
}
