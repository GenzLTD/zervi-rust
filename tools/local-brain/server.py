"""本地 /v1/ask brain。

网关在 TATHA_BASE_URL 指向本进程时才把已验签的请求转过来。
未配置上游时网关仍返回 503，本进程不会被自动调用。

本进程只做 knowledge 意图：用本机 Ollama 生成一句回答。
模型不可用时返回 503，不编造成功答案。
身份以网关注入的 X-Actor-Sub 为准；没有这个头就 401。
"""

from __future__ import annotations

import json
import os
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HOST = os.environ.get("LOCAL_BRAIN_HOST", "127.0.0.1")
PORT = int(os.environ.get("LOCAL_BRAIN_PORT", "8091"))
OLLAMA = os.environ.get("OLLAMA_URL", "http://127.0.0.1:11434").rstrip("/")
MODEL = os.environ.get("LOCAL_BRAIN_MODEL", "qwen2.5:7b")


def generate(prompt: str) -> str:
    body = json.dumps(
        {
            "model": MODEL,
            "prompt": prompt,
            "stream": False,
            "options": {"temperature": 0},
        }
    ).encode()
    req = urllib.request.Request(
        f"{OLLAMA}/api/generate",
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        payload = json.loads(resp.read().decode())
    text = str(payload.get("response") or "").strip()
    if not text:
        raise RuntimeError("ollama returned an empty response")
    return text


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _send(self, code: int, obj: dict) -> None:
        raw = json.dumps(obj, ensure_ascii=False).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self) -> None:
        if self.path.split("?", 1)[0] == "/health":
            self._send(200, {"status": "ok", "model": MODEL})
            return
        self._send(404, {"error": "not found"})

    def do_POST(self) -> None:
        if self.path.split("?", 1)[0] != "/v1/ask":
            self._send(404, {"error": "not found"})
            return
        actor = (self.headers.get("X-Actor-Sub") or "").strip()
        if not actor:
            self._send(401, {"error": "missing X-Actor-Sub"})
            return
        length = int(self.headers.get("Content-Length") or "0")
        raw = self.rfile.read(length) if length else b"{}"
        try:
            incoming = json.loads(raw.decode() or "{}")
        except json.JSONDecodeError:
            self._send(400, {"error": "invalid json"})
            return
        prompt = str(incoming.get("text") or incoming.get("message") or "").strip()
        if not prompt:
            self._send(400, {"error": "empty ask"})
            return
        try:
            answer = generate(prompt)
        except (urllib.error.URLError, TimeoutError, RuntimeError, json.JSONDecodeError) as exc:
            self._send(503, {"error": "local brain unavailable", "detail": str(exc)})
            return
        self._send(
            200,
            {
                "port": "knowledge",
                "status": "ok",
                "result": {"answer": answer, "actor_sub": actor, "model": MODEL},
            },
        )

    def log_message(self, fmt: str, *args) -> None:
        return


def main() -> None:
    httpd = ThreadingHTTPServer((HOST, PORT), Handler)
    print(f"local brain on http://{HOST}:{PORT} model={MODEL}", flush=True)
    httpd.serve_forever()


if __name__ == "__main__":
    main()
