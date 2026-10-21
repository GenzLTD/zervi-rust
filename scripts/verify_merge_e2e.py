#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
合并后端到端契约验证（PR #8 → apple-iap → identity → 7B/local-auth）

验证面：
  1. 网关存活
  2. 本地账号体系 register/login/me（本次新增）
  3. looma /v1/* 端点已挂载（PR #8 带来的 35 端点代表）
  4. /api/v1/* 过渡通路仍在 —— PR #8 重写路由表时曾整段丢失，
     jobfirst-slice 不在 looma 端点之列，丢了无报错，故单独守一条
  5. payment-service 端点可达（Apple IAP 首次进主线）
  6. 7B 简历解析字段契约（字段全为英文键名）

判据：区分「路由不存在(404)」与「路由在但下游/鉴权拒绝(401/502/500)」，
后者算 PASS —— 我们验的是挂载，不是业务成功。

用法: python3 scripts/verify_merge_e2e.py [简历文件路径]
"""
import json
import os
import sys
import urllib.error
import urllib.request
import uuid

BASE = "http://127.0.0.1:3333"
PAY = "http://127.0.0.1:3009"

# 绕过 WorkBuddy 注入的代理，否则拿到的是代理的假响应
for k in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy"):
    os.environ.pop(k, None)
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))

EMAIL = "e2e_merge@test.local"
PASSWORD = "Mergetest123"

results = []


def call(method, url, body=None, token=None, timeout=20):
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = "Bearer " + token
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with OPENER.open(req, timeout=timeout) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:
        return 0, repr(e)


def upload_file(url, filepath, token=None, timeout=300):
    """resume-service 的上传口是 multipart/form-data，字段名 file —— 不是 JSON"""
    boundary = "----ZerviBoundary" + uuid.uuid4().hex
    fn = os.path.basename(filepath)
    with open(filepath, "rb") as f:
        content = f.read()
    body = b"".join([
        ("--%s\r\n" % boundary).encode(),
        ('Content-Disposition: form-data; name="file"; filename="%s"\r\n' % fn).encode(),
        b"Content-Type: text/plain; charset=utf-8\r\n\r\n",
        content,
        ("\r\n--%s--\r\n" % boundary).encode(),
    ])
    req = urllib.request.Request(url, data=body, method="POST")
    req.add_header("Content-Type", "multipart/form-data; boundary=" + boundary)
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with OPENER.open(req, timeout=timeout) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:
        return 0, repr(e)


def check(label, ok, detail=""):
    results.append((label, ok, detail))
    print("  %s  %s%s" % ("✅" if ok else "❌", label, ("  → " + detail) if detail else ""))


def mounted(label, codes, expect_absent=404):
    """404=路由没挂；其它码=挂上了（鉴权/下游拒绝也算挂上）"""
    ok = codes != expect_absent and codes != 0
    check(label, ok, "HTTP %s" % codes)


print("=" * 66)
print("合并后端到端契约验证")
print("=" * 66)

# ── 1. 网关存活 ──────────────────────────────────────────────────
print("\n[1] 网关存活")
st, body = call("GET", BASE + "/health")
check("GET /health", st == 200 and "running" in body, "HTTP %s %s" % (st, body.strip()[:40]))

# ── 2. 本地账号体系（本次新增） ───────────────────────────────────
print("\n[2] 本地账号体系 local_auth")
call("POST", BASE + "/v1/auth/register", {"email": EMAIL, "password": PASSWORD, "name": "合并验证"})
st, body = call("POST", BASE + "/v1/auth/login", {"email": EMAIL, "password": PASSWORD})
token = ""
try:
    token = json.loads(body).get("token") or json.loads(body).get("access_token") or ""
except Exception:
    pass
check("POST /v1/auth/login -> token", st == 200 and bool(token), "HTTP %s token=%d字符" % (st, len(token)))
st, body = call("GET", BASE + "/v1/auth/me", token=token)
check("GET /v1/auth/me 用 token 取回身份", st == 200, "HTTP %s" % st)
# 无 token 必须拒绝，否则等于身份校验失效
st, _ = call("GET", BASE + "/v1/auth/me")
check("GET /v1/auth/me 无 token 应拒绝", st in (401, 403), "HTTP %s" % st)

# ── 3. looma /v1/* 代表端点已挂载（PR #8） ────────────────────────
print("\n[3] looma /v1/* 端点挂载（PR #8 带入）")
st, _ = call("GET", BASE + "/v1/resume/list", token=token)
mounted("GET /v1/resume/list", st)
st, _ = call("GET", BASE + "/v1/jobs/list")
mounted("GET /v1/jobs/list", st)
st, _ = call("POST", BASE + "/v1/enterprise/profile", {}, token=token)
mounted("POST /v1/enterprise/profile", st)

# ── 4. /api/v1/* 过渡通路（PR #8 曾整段丢失） ─────────────────────
print("\n[4] /api/v1/* 过渡通路 —— PR #8 最容易静默丢的一段")
st, _ = call("GET", BASE + "/api/v1/slice/inbox", token=token)
mounted("GET /api/v1/slice/inbox（jobfirst-slice 终端适配器）", st)
st, _ = call("GET", BASE + "/api/v1/user/profile", token=token)
mounted("GET /api/v1/user/profile", st)
st, _ = call("GET", BASE + "/api/v1/upload/list", token=token)
mounted("GET /api/v1/upload/list", st)

# ── 5. payment-service（Apple IAP 首次进主线） ────────────────────
#
# 这一段守的是 2026-10-21 修的那个 gap：payment-service 一直在 :3009 跑着，
# 但网关**没有直通路由**，请求会落到 fallback → looma :5200 ⇒ 客户端经网关
# 永远调不通支付端点，只能直连 :3009。上架审核要走通收据校验，故此处：
#   ① 验经网关可达
#   ② 验经网关与直连的行为一致（防网关层改写/丢失）
#   ③ 验未注册路径确实会落 fallback（反向对照，证明前两条不是在碰巧 fallback）
print("\n[5] payment-service（Apple IAP）—— 经网关 :3333 应可达")
st, body = call("GET", PAY + "/healthz")
check("GET :3009/healthz（直连）", st == 200, "HTTP %s %s" % (st, body.strip()[:20]))

for name, path, body_json in [
    ("verify", "/v1/payment/apple/verify", {"receipt": "mount-probe"}),
    ("restore", "/v1/payment/apple/restore", {"user_id": "probe"}),
    ("webhook", "/v1/payment/apple/webhook", {"signedPayload": "probe"}),
]:
    gw_st, _ = call("POST", BASE + path, body_json, timeout=20)
    dp_st, _ = call("POST", PAY + path, body_json, timeout=20)
    # 502 = 落 fallback(:5200)，说明路由没挂；其它码 = 挂上了
    mounted("经网关 POST %s" % path, gw_st)
    check("  └ 经网关与直连一致", gw_st == dp_st, "网关 %s vs 直连 %s" % (gw_st, dp_st))

st_fb, _ = call("POST", BASE + "/v1/payment/apple/definitely-not-registered", {"x": 1})
check("反向对照：未注册路径应落 fallback(502)", st_fb == 502,
      "HTTP %s —— 若这里也是 422/500 说明上面三条没真正走直通" % st_fb)

# ── 6. 7B 简历解析字段契约 ───────────────────────────────────────
print("\n[6] 7B 简历解析字段契约")
resume = sys.argv[1] if len(sys.argv) > 1 else ""
if resume and os.path.exists(resume):
    st, body = upload_file(BASE + "/v1/resume/upload", resume, token=token, timeout=300)
    ok_upload = st == 200
    check("POST /v1/resume/upload", ok_upload, "HTTP %s" % st)
    if ok_upload:
        try:
            d = json.loads(body)
            ex = d.get("extracted") or {}
            if isinstance(ex, str):
                ex = json.loads(ex)
        except Exception as e:
            ex = {}
            print("     解析响应失败: %r" % e)
        expected = {"name", "gender", "age", "phone", "email", "education",
                    "experience", "skills", "expectation"}
        got = set(ex.keys())
        missing = expected - got
        cn_keys = [k for k in got if any("\u4e00" <= c <= "\u9fff" for c in k)]
        check("九个顶层字段齐全", not missing, "缺: %s" % (sorted(missing) or "无"))
        check("无中文键名", not cn_keys, "中文键: %s" % (cn_keys or "无"))
else:
    print("  ⚠️  未提供简历文件，跳过（用法: %s <简历路径>）" % sys.argv[0])

# ── 汇总 ────────────────────────────────────────────────────────
print("\n" + "=" * 66)
failed = [r for r in results if not r[1]]
print("结果: %d/%d 通过" % (len(results) - len(failed), len(results)))
if failed:
    print("\n失败项:")
    for label, _, detail in failed:
        print("  ❌ %s  %s" % (label, detail))
    print("\n==== 结论: FAIL ❌ ====")
    sys.exit(1)
print("==== 结论: PASS ✅ ====")
