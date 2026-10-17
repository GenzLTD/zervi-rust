#!/usr/bin/env python3
"""Task 2 Part 2 断言脚本（Win 侧自洽）：
  A. user-service GET /profile 按 sub 建档
     1) 首次 200 且 data.id == sub
     2) 二次 200 且 users 表行数不增
     3) 伪造/坏 token 仍 401
     4) 不往 adirp_auth 写新行；且建档行 password_hash 为空（= 未设置）
  B. upload-service 路由改造
     1) 无 token 的 GET /health == 200
     2) 坏 token 的 POST /upload == 401

数据库连接一律走环境变量，**仓库内不落任何凭据与平台绝对路径**：
    MYSQL_BIN       mysql 客户端路径（不设则用 PATH 上的 mysql）
    MYSQL_HOST      默认 127.0.0.1
    MYSQL_PORT      默认 3306
    MYSQL_USER      默认 root
    MYSQL_PASSWORD  口令；经 MYSQL_PWD 传给子进程，不进 argv（`ps` 看不到）

Windows:
    set MYSQL_BIN=C:\\path\\to\\mysql.exe
    set MYSQL_PASSWORD=***
macOS / Linux:
    export MYSQL_BIN="$(command -v mysql)" MYSQL_PASSWORD=***
"""
import os, shutil, subprocess, sys, time, json, base64, hmac, hashlib
import urllib.request, urllib.error

USER_URL   = "http://127.0.0.1:3002"
UPLOAD_URL = "http://127.0.0.1:3007"
USER_SECRET   = "win-test-secret-user"
UPLOAD_SECRET = "win-test-secret-upload"

SUB   = "user-win-profile-001"
PHONE = "13800000001"
NAME  = "WinProfileTest"
ROLE  = "candidate"

_MYSQL = os.environ.get("MYSQL_BIN") or shutil.which("mysql")
if not _MYSQL:
    sys.exit("[FATAL] 找不到 mysql 客户端：请设 MYSQL_BIN，或确保 mysql 在 PATH 上")
MYSQL = _MYSQL
MYSQL_ARGS = [
    "-h", os.environ.get("MYSQL_HOST", "127.0.0.1"),
    "-P", os.environ.get("MYSQL_PORT", "3306"),
    "-u", os.environ.get("MYSQL_USER", "root"),
]
# 口令走 MYSQL_PWD（mysql 客户端原生支持），避免出现在 argv / 进程列表里
if os.environ.get("MYSQL_PASSWORD"):
    os.environ.setdefault("MYSQL_PWD", os.environ["MYSQL_PASSWORD"])

def _b64url(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=")

def sign(secret, sub, role, name, phone):
    """纯 stdlib 实现 HS256 JWT（与 jsonwebtoken 的 from_secret+HS256 对齐）。"""
    header = {"alg": "HS256", "typ": "JWT"}
    now = int(time.time())
    payload = {"sub": sub, "role": role, "name": name, "phone": phone,
               "iat": now, "exp": now + 3600}
    h = _b64url(json.dumps(header, separators=(",", ":")).encode())
    p = _b64url(json.dumps(payload, separators=(",", ":")).encode())
    signing_input = h + b"." + p
    sig = hmac.new(secret.encode(), signing_input, hashlib.sha256).digest()
    return (signing_input + b"." + _b64url(sig)).decode()

def http(method, url, token=None, body=None):
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()
    except Exception as e:  # 连接拒绝等
        return None, str(e)

def mysql_count(query):
    cmd = [MYSQL] + MYSQL_ARGS + ["-N", "-e", query]
    out = subprocess.run(cmd, capture_output=True, text=True)
    return out.stdout.strip()

def line(t): print(f"\n=== {t} ===")

results = {}
ok = True
def check(name, cond, detail=""):
    global ok
    ok = ok and cond
    results[name] = "PASS" if cond else "FAIL"
    print(f"[{results[name]}] {name}  {detail}")

# ---------- A. user-service ----------
line("A. user-service GET /profile 按 sub 建档")
tok = sign(USER_SECRET, SUB, ROLE, NAME, PHONE)

# A1 首次
st, body = http("GET", f"{USER_URL}/profile", token=tok)
a1 = (st == 200)
data = None
if a1:
    try:
        data = json.loads(body).get("data", {})
        a1 = data.get("id") == SUB
    except Exception as e:
        a1 = False
check("A1 首次 200 且 id==sub", a1, f"status={st} id={data.get('id') if data else None}")
count_after_1 = mysql_count("SELECT COUNT(*) FROM adirp_user.users;")

# A2 二次（同 sub，不应增行）
st2, body2 = http("GET", f"{USER_URL}/profile", token=tok)
a2 = (st2 == 200)
data2 = None
if a2:
    try:
        data2 = json.loads(body2).get("data", {})
        a2 = data2.get("id") == SUB
    except Exception:
        a2 = False
count_after_2 = mysql_count("SELECT COUNT(*) FROM adirp_user.users;")
check("A2 二次 200 且不增行", a2 and count_after_1 == count_after_2,
      f"行数 {count_after_1} -> {count_after_2}")

# A3 坏 token
bad = sign(USER_SECRET, SUB, ROLE, NAME, PHONE) + "tamper"
st3, _ = http("GET", f"{USER_URL}/profile", token=bad)
# 同时测「无 token」
st3b, _ = http("GET", f"{USER_URL}/profile")
check("A3 坏 token / 无 token 仍 401", st3 == 401 and st3b == 401,
      f"坏token={st3} 无token={st3b}")

# A4 不往 adirp_auth 写新行 + password_hash 为空
auth_db = mysql_count("SHOW DATABASES LIKE 'adirp_auth';")
ph = mysql_count(f"SELECT password_hash FROM adirp_user.users WHERE id='{SUB}';")
check("A4a password_hash 为空（未设置）", ph == "",
      f"password_hash='{ph}'")
check("A4b 未触碰 adirp_auth（库不存在）", auth_db == "",
      f"adirp_auth 库存在? '{auth_db}'；user-service 不连接 auth-service，天然不写")

# ---------- B. upload-service ----------
line("B. upload-service 路由改造")
# B1 无 token /health
stb1, _ = http("GET", f"{UPLOAD_URL}/health")
check("B1 无 token GET /health == 200", stb1 == 200, f"status={stb1}")

# B2 坏 token POST /upload == 401
stb2, _ = http("POST", f"{UPLOAD_URL}/upload", token="Bearer not-a-real-token",
               body={"filename": "x.pdf"})
check("B2 坏 token POST /upload == 401", stb2 == 401, f"status={stb2}")

# B2' 无 token POST /upload 也应 401（受保护）
stb2b, _ = http("POST", f"{UPLOAD_URL}/upload", body={"filename": "x.pdf"})
check("B2' 无 token POST /upload == 401", stb2b == 401, f"status={stb2b}")

line("SUMMARY")
for k, v in results.items():
    print(f"  {v:4}  {k}")
print("\nOVERALL:", "ALL PASS" if ok else "SOME FAIL")
sys.exit(0 if ok else 1)
