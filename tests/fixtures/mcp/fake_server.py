#!/usr/bin/env python3
"""最小 MCP 服务器（stdio + NDJSON），用于 CraftWork 的端到端测试。

只依赖 Python 标准库，不引第三方 SDK。实现 MCP 客户端联调所需的四个交互：

  initialize                  握手（回 protocolVersion / capabilities / serverInfo）
  notifications/initialized   客户端就绪通知（无响应）
  tools/list                  返回三个工具：echo / fail / screenshot
  tools/call                  执行对应工具

tools/call 的取值刻意覆盖三类结果，用来钉住结果归一逻辑：
  echo        只返回文本
  fail        返回 isError=true
  screenshot  返回一张 1x1 PNG 的 image 内容块（验证图片链路）

协议要点：stdio 传输用「一行一条 JSON-RPC 消息」的 NDJSON 分帧，stdout 只走协议，
诊断信息一律写 stderr。
"""

import json
import sys

PROTOCOL_VERSION = "2025-06-18"

# 1x1 红色 PNG（67 字节），用于图片链路验证
TINY_PNG_BASE64 = (
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg=="
)

TOOLS = [
    {
        "name": "echo",
        "description": "原样返回输入文本（验证文本结果通道）",
        "inputSchema": {
            "type": "object",
            "properties": {"text": {"type": "string", "description": "要回显的文本"}},
            "required": ["text"],
        },
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "fail",
        "description": "总是返回工具级错误（验证 isError 通道）",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "screenshot",
        "description": "返回一张 1x1 的 PNG 图片（验证图片通道）",
        "inputSchema": {"type": "object", "properties": {}},
        "annotations": {"readOnlyHint": True},
    },
]


def log(message):
    """诊断信息只写 stderr，绝不污染 stdout 的协议流。"""
    print(f"[fake-mcp] {message}", file=sys.stderr, flush=True)


def send(payload):
    """写一条 NDJSON 消息到 stdout。"""
    sys.stdout.write(json.dumps(payload, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def call_tool(name, arguments):
    if name == "echo":
        text = str(arguments.get("text", ""))
        return {"content": [{"type": "text", "text": f"回显：{text}"}], "isError": False}
    if name == "fail":
        return {"content": [{"type": "text", "text": "这是一次预期内的失败"}], "isError": True}
    if name == "screenshot":
        return {
            "content": [
                {"type": "text", "text": "截图如下（1x1 红色像素）"},
                {"type": "image", "data": TINY_PNG_BASE64, "mimeType": "image/png"},
            ],
            "isError": False,
            "structuredContent": {"width": 1, "height": 1},
        }
    return {
        "content": [{"type": "text", "text": f"未知工具：{name}"}],
        "isError": True,
    }


def handle(message):
    method = message.get("method")
    request_id = message.get("id")
    params = message.get("params") or {}

    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": {"name": "craftwork-fake-mcp", "version": "0.1.0"},
                "instructions": "这是 CraftWork 测试用的假 MCP 服务器。",
            },
        }
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": request_id, "result": {"tools": TOOLS}}
    if method == "tools/call":
        result = call_tool(params.get("name", ""), params.get("arguments") or {})
        return {"jsonrpc": "2.0", "id": request_id, "result": result}
    if method in ("notifications/initialized", "notifications/cancelled"):
        # 通知没有 id，不回响应
        return None
    if method == "ping":
        return {"jsonrpc": "2.0", "id": request_id, "result": {}}
    if request_id is None:
        return None
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": -32601, "message": f"Method not found: {method}"},
    }


def main():
    log("启动完成，等待请求")
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError as error:
            # 客户端发来非协议内容不应让我们退出，记日志继续
            log(f"无法解析的输入：{error}")
            continue
        response = handle(message)
        if response is not None:
            send(response)
    log("stdin 关闭，退出")


if __name__ == "__main__":
    main()
