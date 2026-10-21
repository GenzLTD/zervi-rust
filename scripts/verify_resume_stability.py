#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""7B 稳定性验证：同一份简历连跑 N 次，比对 extracted 每次是否完全一致。"""
import json
import os
import sys
import time
import urllib.error
import urllib.request
import uuid

BASE = "http://127.0.0.1:3333"
for k in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy"):
    os.environ.pop(k, None)
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

N = int(sys.argv[2]) if len(sys.argv) > 2 else 3
RESUME = sys.argv[1] if len(sys.argv) > 1 else "/tmp/real_resume.txt"
EMAIL = f"zervi7bs+{uuid.uuid4().hex[:6]}@test.local"
PASSWORD = "Zervi7b#2026"


def post(path, payload, token=None, timeout=60):
    req = urllib.request.Request(BASE + path, data=json.dumps(payload).encode(), method="POST")
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    with opener.open(req, timeout=timeout) as r:
        return json.loads(r.read().decode("utf-8", "replace"))


def upload(path, filepath, token, timeout=300):
    boundary = "----ZerviBoundary" + uuid.uuid4().hex
    with open(filepath, "rb") as f:
        content = f.read()
    body = b"".join([
        f"--{boundary}\r\n".encode(),
        f'Content-Disposition: form-data; name="file"; filename="{os.path.basename(filepath)}"\r\n'.encode(),
        b"Content-Type: text/plain; charset=utf-8\r\n\r\n",
        content,
        f"\r\n--{boundary}--\r\n".encode(),
    ])
    req = urllib.request.Request(BASE + path, data=body, method="POST")
    req.add_header("Content-Type", f"multipart/form-data; boundary={boundary}")
    req.add_header("Authorization", "Bearer " + token)
    with opener.open(req, timeout=timeout) as r:
        return json.loads(r.read().decode("utf-8", "replace"))


post("/v1/auth/register", {"email": EMAIL, "password": PASSWORD, "name": "稳定性"})
token = post("/v1/auth/login", {"email": EMAIL, "password": PASSWORD})["token"]

runs = []
times = []
for i in range(1, N + 1):
    t0 = time.time()
    try:
        res = upload("/v1/resume/upload", RESUME, token)
    except urllib.error.HTTPError as e:
        print(f"#{i} HTTP {e.code}: {e.read().decode()[:300]}")
        continue
    dt = time.time() - t0
    times.append(dt)
    ex = res.get("extracted")
    runs.append(ex)
    keys = sorted(ex.keys()) if ex else []
    print(f"#{i}  {dt:5.1f}s  keys={len(keys)}  name={ex.get('name') if ex else '-'}  "
          f"company={ex['experience'][0].get('company') if ex and ex.get('experience') else '-'}  "
          f"expect={ex['expectation'].get('position') if ex and ex.get('expectation') else '-'}")

print("\n=== 一致性比对 ===")
first = json.dumps(runs[0], ensure_ascii=False, sort_keys=True)
identical = all(json.dumps(r, ensure_ascii=False, sort_keys=True) == first for r in runs)
for i, r in enumerate(runs[1:], start=2):
    same = json.dumps(r, ensure_ascii=False, sort_keys=True) == first
    if not same:
        a, b = runs[0], r
        diffs = [k for k in set(a) | set(b) if json.dumps(a.get(k), ensure_ascii=False, sort_keys=True)
                 != json.dumps(b.get(k), ensure_ascii=False, sort_keys=True)]
        print(f"  #1 vs #{i}: 差异字段 {diffs}")
print(f"  {N} 次输出完全一致: {'是 ✅' if identical else '否 ❌（见上方差异）'}")
print(f"  耗时: min={min(times):.1f}s max={max(times):.1f}s avg={sum(times)/len(times):.1f}s")
