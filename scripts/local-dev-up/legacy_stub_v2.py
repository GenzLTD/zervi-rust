#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
本机联调 legacy stub v2 —— 顶替 looma Flask :5200（仅开发）。

能力：
  · GET  /v1/compliance/consent/status  → {status:{scope:bool}}（ensureConsent 直通）
  · POST /v1/compliance/consent/grant   → 落盘，跨重启保留
  · POST /v1/compliance/consent/revoke
  · 其余路径回声 JSON（via=legacy-stub），供网关 fallback 透传探针

双栈：同时听 127.0.0.1 与 ::1（网关写死 localhost:5200，Windows 常先解析 IPv6）。

用法：
  python legacy_stub_v2.py [--port 5200] [--store /path/consent_store.json]
"""
from __future__ import annotations

import argparse
import base64
import json
import socket
import threading
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

PRIMARY = ("jobseeker_core", "credit_query", "report_share")


def _b64url_json(segment: str) -> dict[str, Any]:
    pad = "=" * (-len(segment) % 4)
    raw = base64.urlsafe_b64decode(segment + pad)
    return json.loads(raw.decode("utf-8"))


def user_key_from_auth(auth: str | None) -> str:
    """开发 stub：不验签，只从 Bearer JWT payload 取 sub；失败则用整段 token 哈希键。"""
    if not auth or not auth.lower().startswith("bearer "):
        return "anon"
    token = auth.split(" ", 1)[1].strip()
    parts = token.split(".")
    if len(parts) >= 2:
        try:
            claims = _b64url_json(parts[1])
            sub = claims.get("sub")
            if isinstance(sub, str) and sub:
                return f"sub:{sub}"
        except Exception:
            pass
    return f"tok:{token[:32]}"


class Store:
    def __init__(self, path: Path):
        self.path = path
        self._lock = threading.Lock()
        self.path.parent.mkdir(parents=True, exist_ok=True)
        if not self.path.exists():
            self._write({"users": {}})

    def _read(self) -> dict[str, Any]:
        try:
            return json.loads(self.path.read_text(encoding="utf-8"))
        except Exception:
            return {"users": {}}

    def _write(self, data: dict[str, Any]) -> None:
        self.path.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")

    def status_map(self, user: str) -> dict[str, bool]:
        with self._lock:
            data = self._read()
            granted = set((data.get("users") or {}).get(user, {}).get("scopes") or [])
        return {s: (s in granted) for s in PRIMARY}

    def grant(self, user: str, scope: str) -> dict[str, Any]:
        with self._lock:
            data = self._read()
            users = data.setdefault("users", {})
            row = users.setdefault(user, {"scopes": [], "ids": {}})
            scopes: list[str] = list(row.get("scopes") or [])
            ids: dict[str, str] = dict(row.get("ids") or {})
            if scope in scopes:
                return {"consent_id": ids.get(scope, "stub-existing"), "already_granted": True}
            cid = f"stub-{uuid.uuid4().hex[:8]}"
            scopes.append(scope)
            ids[scope] = cid
            # jobseeker_core 包：与 Flask 一样展开几个子 scope（最小集）
            if scope == "jobseeker_core":
                for child in ("application_submit", "resume_parse", "jobs_match"):
                    if child not in scopes:
                        scopes.append(child)
                        ids[child] = f"stub-{uuid.uuid4().hex[:8]}"
            row["scopes"] = scopes
            row["ids"] = ids
            self._write(data)
            return {"consent_id": cid, "already_granted": False}

    def revoke(self, user: str, scope: str) -> dict[str, Any]:
        with self._lock:
            data = self._read()
            users = data.setdefault("users", {})
            row = users.setdefault(user, {"scopes": [], "ids": {}})
            scopes = [s for s in (row.get("scopes") or []) if s != scope]
            ids = dict(row.get("ids") or {})
            cid = ids.pop(scope, None)
            row["scopes"] = scopes
            row["ids"] = ids
            self._write(data)
            return {"revoked": True, "consent_id": cid}


def make_handler(store: Store):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, fmt: str, *args: Any) -> None:
            sys_stderr = __import__("sys").stderr
            sys_stderr.write("[legacy-stub] " + (fmt % args) + "\n")

        def _read_json(self) -> dict[str, Any]:
            length = int(self.headers.get("Content-Length") or 0)
            if length <= 0:
                return {}
            raw = self.rfile.read(length)
            try:
                return json.loads(raw.decode("utf-8") or "{}")
            except Exception:
                return {}

        def _send(self, code: int, body: Any) -> None:
            payload = json.dumps(body, ensure_ascii=False).encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(payload)))
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(payload)

        def do_OPTIONS(self) -> None:  # noqa: N802
            self.send_response(204)
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Access-Control-Allow-Methods", "GET,POST,PUT,DELETE,OPTIONS")
            self.send_header("Access-Control-Allow-Headers", "Authorization,Content-Type")
            self.end_headers()

        def do_GET(self) -> None:  # noqa: N802
            parsed = urlparse(self.path)
            path = parsed.path
            auth = self.headers.get("Authorization")
            user = user_key_from_auth(auth)
            if path == "/v1/compliance/consent/status":
                sm = store.status_map(user)
                self._send(200, {"user_id": user, "status": sm, "via": "legacy-stub-v2"})
                return
            if path in ("/health", "/"):
                self._send(200, {"status": "ok", "service": "legacy-stub-v2"})
                return
            self._send(
                200,
                {
                    "via": "legacy-stub",
                    "method": "GET",
                    "path": self.path,
                    "auth": (auth or "")[:24],
                },
            )

        def do_POST(self) -> None:  # noqa: N802
            parsed = urlparse(self.path)
            path = parsed.path
            auth = self.headers.get("Authorization")
            user = user_key_from_auth(auth)
            body = self._read_json()
            if path == "/v1/compliance/consent/grant":
                scope = body.get("scope") or ""
                scopes = body.get("scopes") if isinstance(body.get("scopes"), list) else None
                if scopes:
                    results = [store.grant(user, s) | {"scope": s} for s in scopes if isinstance(s, str)]
                    self._send(200, {"granted": len(results), "results": results, "via": "legacy-stub-v2"})
                    return
                if not scope:
                    self._send(400, {"error": "missing_scope", "message": "请指定 scope"})
                    return
                self._send(200, store.grant(user, scope) | {"via": "legacy-stub-v2"})
                return
            if path == "/v1/compliance/consent/revoke":
                scope = body.get("scope") or ""
                if not scope:
                    self._send(400, {"error": "missing_scope", "message": "请指定 scope"})
                    return
                self._send(200, store.revoke(user, scope) | {"via": "legacy-stub-v2"})
                return
            self._send(
                200,
                {
                    "via": "legacy-stub",
                    "method": "POST",
                    "path": self.path,
                    "auth": (auth or "")[:24],
                    "body_keys": sorted(body.keys()),
                },
            )

        def do_PUT(self) -> None:  # noqa: N802
            self.do_POST()

        def do_DELETE(self) -> None:  # noqa: N802
            self.do_GET()

    return Handler


def _serve(host: str, port: int, handler_cls: type, family: int) -> ThreadingHTTPServer:
    class Server(ThreadingHTTPServer):
        address_family = family

    httpd = Server((host, port), handler_cls)
    httpd.daemon_threads = True
    return httpd


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=5200)
    ap.add_argument(
        "--store",
        default=str(Path(__file__).resolve().parents[2] / ".tmp-pg" / "consent_store.json"),
    )
    args = ap.parse_args()
    store = Store(Path(args.store))
    handler = make_handler(store)

    servers: list[ThreadingHTTPServer] = []
    # IPv4
    servers.append(_serve("127.0.0.1", args.port, handler, socket.AF_INET))
    # IPv6（失败则跳过——某些环境无 IPv6）
    try:
        servers.append(_serve("::1", args.port, handler, socket.AF_INET6))
    except OSError as e:
        print(f"[legacy-stub] IPv6 bind skipped: {e}")

    print(f"[legacy-stub] v2 listening :{args.port} store={args.store}")
    threads = [threading.Thread(target=s.serve_forever, daemon=True) for s in servers]
    for t in threads:
        t.start()
    try:
        threads[0].join()
    except KeyboardInterrupt:
        pass
    for s in servers:
        s.shutdown()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
