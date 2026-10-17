"""T1.0 对拍夹具：最小的 MCP stdio 服务端（非生产）。

为什么需要它：
    T1.0 要验的是「rmcp（Rust SDK）能不能和 mcp（Python SDK）对上话」。
    这个契约与 cortex-mcp 的业务逻辑无关，只与 **SDK 实现 + stdio 帧 + protocolVersion 协商**有关。
    所以在没有 cortex-mcp 运行环境的机器上（例如 Dell/Win 侧未装 litellm/psycopg），
    用本夹具就能先把通路验通；等 Mac 侧 cortex-mcp 起来后，再用同一探针对拍真实服务端。

刻意与 cortex-mcp 保持一致的地方：
    - 同一个 import 路径 `mcp.server.fastmcp.FastMCP`（cortex-mcp/src/cortex_mcp/server.py:14 同款）；
    - 同样以 `mcp.run()` 走 stdio；
    - 四个工具同名（remember/recall/forget/consolidate），便于比对 tools/list 结果。

用法：
    <venv>/python.exe -m probe_server          # 由 mcp-probe 作为子进程拉起
    或
    <venv>/python.exe probe_server.py
"""

import sys

from mcp.server.fastmcp import FastMCP

mcp = FastMCP("probe-fixture", instructions="T1.0 握手探针夹具，非生产服务端")


@mcp.tool()
def remember(content: str, kind: str = "semantic") -> str:
    """空壳：写入一条记忆（不落库）。"""
    return f"ok:{kind}:{len(content)}"


@mcp.tool()
def recall(query: str, k: int = 5) -> str:
    """空壳：检索记忆（恒返回空数组）。"""
    return "[]"


@mcp.tool()
def forget(memory_id: str) -> str:
    """空壳：删除一条记忆。"""
    return "ok"


@mcp.tool()
def consolidate() -> str:
    """空壳：合并/整理记忆。"""
    return "ok"


if __name__ == "__main__":
    # ⚠️ stdio 模式下 stdout 是 JSON-RPC 的专用通道，
    #    任何 print() 都会破坏帧格式 —— 诊断信息一律走 stderr。
    print(f"[probe-fixture] starting, python={sys.version.split()[0]}", file=sys.stderr)
    mcp.run()
