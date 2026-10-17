"""T1.0/T2.0 对拍夹具：最小的 MCP stdio 服务端（非生产）。

为什么需要它：
    T1.0 要验的是「rmcp（Rust SDK）能不能和 mcp（Python SDK）对上话」，
    T2.0 要验的是「四个业务方法能不能真的往返」。这两个契约都与 cortex-mcp 的
    业务逻辑无关，只与 **SDK 实现 + stdio 帧 + protocolVersion 协商 + 工具名** 有关。
    所以在没有 cortex-mcp 运行环境的机器上（例如 Dell/Win 侧未装 litellm/psycopg），
    用本夹具就能先把通路验通；Mac 侧再用同一套二进制对拍真实服务端。

与 cortex-mcp 保持一致的地方（**以 `tools/list` 为准，不靠记忆**）：
    - import 路径 `mcp.server.fastmcp.FastMCP`（cortex-mcp/src/cortex_mcp/server.py 同款）；
    - 以 `mcp.run()` 走 stdio；
    - 四个工具名：remember / recall / forget / **consolidate_memories**
      （⚠️ 第 4 个**不是** `consolidate` —— T1.0 时本夹具写错过，会导致
        `tool not found`，已在此修正）；
    - 入参签名同款（remember 的 metadata 是 **JSON 字符串**，不是对象）。

额外能力（仅夹具，生产环境没有）：
    - 进程内内存表，因此 remember → recall → forget 可真实往返；
    - `FIXTURE_RECALL_DELAY_MS`：让 recall 故意变慢，用来验证客户端的
      **超时上限**确实能中断等待（降级语义）。

用法：
    <venv>/python.exe -m probe_server          # 由 mcp-probe / mcp-memory-smoke 拉起
"""

import json
import os
import sys
import time
import uuid

from mcp.server.fastmcp import FastMCP

mcp = FastMCP("probe-fixture", instructions="T1.0/T2.0 对拍夹具，非生产服务端")

# 进程内记忆表：id -> 记录
_STORE: dict[str, dict] = {}


@mcp.tool()
def remember(content: str, kind: str = "semantic", metadata: str = "{}", importance: float = 0.5) -> str:
    """写入一条记忆（夹具：写入进程内内存表）。"""
    mid = str(uuid.uuid4())
    try:
        meta = json.loads(metadata) if metadata else {}
    except json.JSONDecodeError:
        meta = {"_raw": metadata}
    _STORE[mid] = {
        "id": mid,
        "content": content,
        "kind": kind,
        "metadata": meta,
        "importance": importance,
        "created_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
    }
    # 文案与真实 cortex-mcp 一致（server.py: "remembered {mem.id} ({mem.kind})"）
    return f"remembered {mid} ({kind})"


@mcp.tool()
def recall(query: str, kind: str | None = None, k: int = 5) -> str:
    """检索记忆（夹具：子串匹配，足够验证往返；真实侧是向量检索）。"""
    delay = int(os.environ.get("FIXTURE_RECALL_DELAY_MS", "0") or "0")
    if delay > 0:
        time.sleep(delay / 1000.0)

    q = query.strip()
    hits = [
        rec
        for rec in _STORE.values()
        if (kind is None or rec["kind"] == kind)
        and (not q or q in rec["content"] or q in json.dumps(rec["metadata"], ensure_ascii=False))
    ]
    if not hits:
        return "no relevant memories"
    hits.sort(key=lambda r: r["importance"], reverse=True)
    return json.dumps(hits[:k], ensure_ascii=False, indent=2)


@mcp.tool()
def forget(memory_id: str) -> str:
    """按 id 删除一条记忆（夹具：同真实侧的文案）。"""
    return "forgotten" if _STORE.pop(memory_id, None) is not None else f"{memory_id} not found"


@mcp.tool()
def consolidate_memories() -> str:
    """元认知巩固（夹具：只衰减 + 清理，模拟 stats 返回）。"""
    decayed, pruned = 0, 0
    for mid in list(_STORE):
        _STORE[mid]["importance"] *= 0.95
        decayed += 1
        if _STORE[mid]["importance"] < 0.05:
            _STORE.pop(mid, None)
            pruned += 1
    return json.dumps(
        {"decayed": decayed, "pruned": pruned, "remaining": len(_STORE)}, ensure_ascii=False
    )


def main() -> None:
    logger_note = f"[probe-fixture] starting, python={sys.version.split()[0]}, pid={os.getpid()}"
    # ⚠️ stdio 模式下 stdout 是 JSON-RPC 的专用通道，
    #    任何 print() 都会破坏帧格式 —— 诊断信息一律走 stderr。
    print(logger_note, file=sys.stderr)
    mcp.run()


if __name__ == "__main__":
    main()
