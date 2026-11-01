package mcp

import (
	"bufio"
	"context"
	"encoding/json"
	"os"
	"strings"
	"testing"
	"time"
)

// TestMain 让同一个测试二进制在设置环境变量后充当 MCP 服务器。
func TestMain(m *testing.M) {
	if os.Getenv("CRAFTWORK_MCP_TEST_SERVER") == "1" {
		runTestServer()
		os.Exit(0)
	}
	os.Exit(m.Run())
}

func runTestServer() {
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 64*1024), 4<<20)
	encoder := json.NewEncoder(os.Stdout)
	for scanner.Scan() {
		var request struct {
			ID     *int64         `json:"id"`
			Method string         `json:"method"`
			Params map[string]any `json:"params"`
		}
		if err := json.Unmarshal(scanner.Bytes(), &request); err != nil || request.ID == nil {
			continue
		}
		response := map[string]any{"jsonrpc": "2.0", "id": *request.ID}
		switch request.Method {
		case "initialize":
			response["result"] = map[string]any{
				"protocolVersion": "2024-11-05",
				"capabilities":    map[string]any{"tools": map[string]any{}},
				"serverInfo":      map[string]any{"name": "test-server", "version": "1.0.0"},
			}
		case "tools/list":
			response["result"] = map[string]any{
				"tools": []map[string]any{{
					"name":        "echo",
					"description": "回显消息",
					"inputSchema": map[string]any{
						"type":       "object",
						"properties": map[string]any{"message": map[string]any{"type": "string"}},
					},
				}},
			}
		case "tools/call":
			name, _ := request.Params["name"].(string)
			arguments, _ := request.Params["arguments"].(map[string]any)
			if name != "echo" {
				response["error"] = map[string]any{"code": -32601, "message": "找不到工具"}
			} else {
				message := ""
				if arguments != nil {
					message, _ = arguments["message"].(string)
				}
				response["result"] = map[string]any{
					"content": []map[string]any{{"type": "text", "text": message}},
				}
			}
		default:
			response["error"] = map[string]any{"code": -32601, "message": "找不到方法"}
		}
		_ = encoder.Encode(response)
	}
}

func startTestClient(t *testing.T) *Client {
	t.Helper()
	t.Setenv("CRAFTWORK_MCP_TEST_SERVER", "1")
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	client, err := Start(ctx, ServerConfig{
		ID:      "test_server",
		Name:    "Test Server",
		Command: os.Args[0],
		Args:    []string{"-test.run=TestMCPTestServer"},
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { client.Close() })
	if err := client.Initialize(ctx); err != nil {
		t.Fatal(err)
	}
	return client
}

func TestClientInitializeListAndCall(t *testing.T) {
	client := startTestClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	toolsList, err := client.ListTools(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(toolsList) != 1 || toolsList[0].Name != "echo" {
		t.Fatalf("工具列表不符: %+v", toolsList)
	}

	output, isError, err := client.CallTool(ctx, "echo", map[string]any{"message": "hello mcp"})
	if err != nil {
		t.Fatal(err)
	}
	if isError || output != "hello mcp" {
		t.Fatalf("调用结果不符: %q isError=%v", output, isError)
	}

	if _, _, err := client.CallTool(ctx, "missing", nil); err == nil {
		t.Fatal("未知工具应返回错误")
	}
}

func TestDynamicToolNameAndSchema(t *testing.T) {
	client := startTestClient(t)
	toolsList, err := client.ListTools(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	server := ServerConfig{ID: "my-server", Name: "My Server"}
	dynamic := NewDynamicTool(client, server, toolsList[0])
	if dynamic.Name() != "mcp__my-server__echo" {
		t.Fatalf("工具名称不符: %s", dynamic.Name())
	}
	if !strings.Contains(dynamic.Description(), "My Server") {
		t.Fatalf("描述应包含服务器名: %s", dynamic.Description())
	}
	if dynamic.Schema()["type"] != "object" {
		t.Fatalf("Schema 不符: %+v", dynamic.Schema())
	}
}

func TestStartRejectsEmptyCommand(t *testing.T) {
	if _, err := Start(context.Background(), ServerConfig{}); err == nil {
		t.Fatal("空命令应返回错误")
	}
}

func TestSanitizeToolName(t *testing.T) {
	if got := sanitize("a b/c"); got != "a_b_c" {
		t.Fatalf("名称清洗不符: %s", got)
	}
	if got := sanitize("///"); got != "tool" {
		t.Fatalf("空名称应回退: %s", got)
	}
}
