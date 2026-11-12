#!/usr/bin/env python3
"""测试用 MCP 服务器：行为与 fake_server.py 相同，但 **stdin EOF 后不退出**。

用途：验证应用退出时是否真的走了 `close_all`（rmcp 的 transport.close → 子进程组终止）。
一个靠 EOF 自行退出的服务器无法区分「优雅关闭」与「父进程消失」——真实的日志型 /
长驻型 MCP 服务器很多并不会因为 stdin 关闭而退出，孤儿进程就是这么留下的。

连接期行为：从 tests/fixtures/mcp/fake_server.py 复制最小实现（initialize / tools/list /
tools/call），只暴露一个 echo 工具。EOF 之后进入空转，等待被显式杀死。
"""

import json
import sys
import time

SERVER_INFO = {"name": "craftwork-stubborn-mcp", "version": "0.1.0"}
PROTOCOL_VERSION = "2024-11-05"

TOOLS = [
    {
        "name": "echo",
        "description": "回显输入文本",
        "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}},
        "annotations": {"readOnlyHint": True},
    }
]


def send(payload):
    sys.stdout.write(json.dumps(payload) + "\n")
    sys.stdout.flush()


def handle(request):
    method = request.get("method")
    request_id = request.get("id")
    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": SERVER_INFO,
            },
        }
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": request_id, "result": {"tools": TOOLS}}
    if method == "tools/call":
        arguments = request.get("params", {}).get("arguments", {})
        return {
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {"content": [{"type": "text", "text": str(arguments.get("text", ""))}]},
        }
    if request_id is None:
        return None  # 通知类消息不回
    return {"jsonrpc": "2.0", "id": request_id, "error": {"code": -32601, "message": "未知方法"}}


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue
        response = handle(request)
        if response is not None:
            send(response)
    # stdin 关闭后不退出：真实长驻服务器常这样，孤儿进程正是从这里产生
    while True:
        time.sleep(1)


if __name__ == "__main__":
    main()
