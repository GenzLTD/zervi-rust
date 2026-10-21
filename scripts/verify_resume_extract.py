#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Zervi 7B 验证：注册/登录 -> 上传真实风格简历 -> 检查 extracted 字段契约"""
import json
import os
import sys
import time
import urllib.error
import urllib.request
import uuid

BASE = "http://127.0.0.1:3333"
# 绕开 WorkBuddy 注入的代理，否则打到假 upstream
for k in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy"):
    os.environ.pop(k, None)
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

EMAIL = os.environ.get("ZV_EMAIL") or f"zervi7b+{uuid.uuid4().hex[:6]}@test.local"
PASSWORD = os.environ.get("ZV_PASSWORD") or "Zervi7b#2026"
RESUME = sys.argv[1] if len(sys.argv) > 1 else "/tmp/real_resume.txt"

EXPECT_KEYS = ["name", "gender", "age", "phone", "email", "education",
               "experience", "skills", "expectation"]


def post(path, payload, token=None, timeout=60):
    req = urllib.request.Request(BASE + path, data=json.dumps(payload).encode(), method="POST")
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    with opener.open(req, timeout=timeout) as r:
        return r.status, json.loads(r.read().decode("utf-8", "replace"))


def upload(path, filepath, token=None, timeout=300):
    boundary = "----ZerviBoundary" + uuid.uuid4().hex
    fn = os.path.basename(filepath)
    with open(filepath, "rb") as f:
        content = f.read()
    body = b"".join([
        f"--{boundary}\r\n".encode(),
        f'Content-Disposition: form-data; name="file"; filename="{fn}"\r\n'.encode(),
        b"Content-Type: text/plain; charset=utf-8\r\n\r\n",
        content,
        f"\r\n--{boundary}--\r\n".encode(),
    ])
    req = urllib.request.Request(BASE + path, data=body, method="POST")
    req.add_header("Content-Type", f"multipart/form-data; boundary={boundary}")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    with opener.open(req, timeout=timeout) as r:
        return r.status, json.loads(r.read().decode("utf-8", "replace"))


def main():
    print(f"[0] 健康检查")
    with opener.open(BASE + "/health", timeout=10) as r:
        print("    gateway:", r.read().decode()[:80])

    print(f"[1] 注册/登录  {EMAIL}")
    try:
        st, res = post("/v1/auth/register", {"email": EMAIL, "password": PASSWORD, "name": "7B验证"})
        print(f"    register -> {st}")
    except urllib.error.HTTPError as e:
        print(f"    register -> {e.code} {e.read().decode()[:200]}")

    st, res = post("/v1/auth/login", {"email": EMAIL, "password": PASSWORD})
    print(f"    login    -> {st}")
    token = res.get("token") or res.get("access_token")
    if not token:
        print("!! 拿不到 token:", json.dumps(res, ensure_ascii=False)[:300])
        return 1
    print(f"    token len = {len(token)}")

    print(f"[2] 上传简历 {RESUME}  (7B 推理，可能 1-3 分钟)")
    t0 = time.time()
    try:
        st, res = upload("/v1/resume/upload", RESUME, token=token)
    except urllib.error.HTTPError as e:
        print(f"    upload -> HTTP {e.code}: {e.read().decode()[:400]}")
        return 1
    dt = time.time() - t0
    print(f"    upload -> {st}   耗时 {dt:.1f}s")
    print(f"    status = {res.get('status')}   resume_id = {res.get('resume_id')}")

    ex = res.get("extracted")
    if not ex:
        print("!! extracted 为空 —— 解析失败")
        print(json.dumps(res, ensure_ascii=False, indent=2)[:1500])
        return 1

    print("\n[3] extracted 字段契约检查")
    keys = list(ex.keys())
    missing = [k for k in EXPECT_KEYS if k not in keys]
    extra = [k for k in keys if k not in EXPECT_KEYS]
    nonascii_keys = [k for k in keys if any(ord(c) > 127 for c in k)]
    print(f"    顶层键      : {keys}")
    print(f"    缺失字段    : {missing or '无 ✅'}")
    print(f"    多余字段    : {extra or '无'}")
    print(f"    中文键名    : {nonascii_keys or '无 ✅'}")
    print("\n[4] 完整输出")
    print(json.dumps(ex, ensure_ascii=False, indent=2))

    ok = not missing and not nonascii_keys
    print("\n==== 结论:", "PASS ✅ 字段契约稳定" if ok else "FAIL ❌ 契约不符", "====")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
