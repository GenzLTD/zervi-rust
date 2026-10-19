"""本地 mock Supabase Auth：只做两件事 —— 发 JWKS、用密码换一枚 ES256 token。

用途：在拿到真项目凭据之前，自测 lane-b.sh 的判定逻辑（不联网、不涉密）。
需要 cryptography（用 C:/Users/dell/.workbuddy/binaries/python/envs/cortex/Scripts/python.exe）。

用法：python mock_supabase.py [port]
"""
import base64
import json
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature

KEY = ec.generate_private_key(ec.SECP256R1())
KID = "mock-es256-key"
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 54321


def b64u(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def jwks() -> dict:
    nums = KEY.public_key().public_numbers()
    return {
        "keys": [
            {
                "kty": "EC",
                "crv": "P-256",
                "kid": KID,
                "x": b64u(nums.x.to_bytes(32, "big")),
                "y": b64u(nums.y.to_bytes(32, "big")),
                "alg": "ES256",
                "use": "sig",
            }
        ]
    }


def mint(email: str) -> str:
    header = {"alg": "ES256", "typ": "JWT", "kid": KID}
    payload = {
        "sub": "11111111-2222-3333-4444-555555555555",
        "aud": "authenticated",
        "role": "authenticated",
        "email": email,
        "iat": int(time.time()),
        "exp": int(time.time()) + 3600,
    }
    signing_input = (
        b64u(json.dumps(header, separators=(",", ":")).encode())
        + "."
        + b64u(json.dumps(payload, separators=(",", ":")).encode())
    )
    der = KEY.sign(signing_input.encode(), ec.ECDSA(hashes.SHA256()))
    r, s = decode_dss_signature(der)
    raw = r.to_bytes(32, "big") + s.to_bytes(32, "big")
    return signing_input + "." + b64u(raw)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _send(self, code: int, obj: dict):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if urlparse(self.path).path.endswith("/.well-known/jwks.json"):
            self._send(200, jwks())
        else:
            self._send(404, {"error": "not found"})

    def do_POST(self):
        u = urlparse(self.path)
        if u.path.endswith("/token"):
            length = int(self.headers.get("Content-Length") or 0)
            try:
                body = json.loads(self.rfile.read(length) or b"{}")
            except Exception:
                body = {}
            email = body.get("email") or "mock@example.com"
            qs = parse_qs(u.query)
            if qs.get("grant_type", [""])[0] != "password":
                self._send(400, {"error": "unsupported_grant_type"})
                return
            self._send(
                200,
                {
                    "access_token": mint(email),
                    "token_type": "bearer",
                    "expires_in": 3600,
                    "refresh_token": "mock-refresh",
                    "user": {"id": "11111111-2222-3333-4444-555555555555", "email": email},
                },
            )
        else:
            self._send(404, {"error": "not found"})

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    print(f"mock supabase on http://127.0.0.1:{PORT} kid={KID}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
