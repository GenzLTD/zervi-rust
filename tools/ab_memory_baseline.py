#!/usr/bin/env python3
"""A/B 基线脚本：测「记忆注入前」的噪声底（Dell 侧落地，Mac 侧跑）

为什么需要它
------------
`/upload` 原本只返回分类结果，`extracted_info` / `quality_score` 算完就丢
（既不回传也不落库），所以「有记忆 vs 无记忆」两轮在响应体上看不出差异。
`UPLOAD_EXTRACTION_DIGEST=true` 打开测量面后，响应体才会多一个
`extraction_digest`。本脚本就吃这个字段。

关键设计：两组共享同一个进程
----------------------------
对照组不靠「改 MCP_ENABLED 再重启」——那样后跑的一组会继承前面预热好的
Ollama 模型，测到的是冷启动差异，不是记忆的收益。测量面开启时，服务端允许
请求头 `X-Memory-Mode: off` 单请求跳过记忆，两组因此共享同一进程、同一份
预热模型。

用法
----
    # 1) 服务端先开测量面（重启一次即可）
    UPLOAD_EXTRACTION_DIGEST=true ./target/debug/upload-service

    # 2) 跑基线（3 轮，顺序交替）
    python3 tools/ab_memory_baseline.py \
        --token "$TOKEN" \
        --file ~/Documents/sample_resume.pdf \
        --rounds 3

    # 有 token 文件时
    python3 tools/ab_memory_baseline.py --token-file /tmp/token.txt --file cv.pdf

输出
----
    ab-results/<时间戳>/runs.json     每轮原始记录
    ab-results/<时间戳>/summary.md    中位数对照表 + 噪声判断

判读
----
注入**前**的基线，两组理论上应当**完全一致**（记忆此刻只写不读，不影响 prompt）。
所以本轮的结论不是「记忆有没有用」，而是拿到这台机器的**噪声底**：
  - overall_score / skills_count 的中位数差 = 0 且极差 = 0 ⇒ 确定性足够，注入后
    任何稳定差值都可归因；
  - 极差很大（比如 12 分） ⇒ 先别急着注入，要加轮数或固定环境（CPU 占用、
    模型 keep_alive），否则日后任何结论都会被噪声吃掉。
"""

import argparse
import json
import mimetypes
import os
import statistics
import sys
import time
import urllib.error
import urllib.request
import uuid

NUMERIC_KEYS = [
    ("quality.overall_score", ("quality", "overall_score")),
    ("quality.completeness", ("quality", "completeness")),
    ("quality.clarity", ("quality", "clarity")),
    ("quality.professionalism", ("quality", "professionalism")),
    ("skills_count", ("skills_count",)),
    ("education_count", ("education_count",)),
    ("work_count", ("work_count",)),
    ("extraction_duration_ms", ("extraction_duration_ms",)),
    ("scoring_duration_ms", ("scoring_duration_ms",)),
    ("processing_time_ms", ("processing_time_ms",)),
    ("classification_confidence", ("classification_confidence",)),
    ("extraction_confidence", ("extraction_confidence",)),
]


def dig(obj, path):
    cur = obj
    for key in path:
        if not isinstance(cur, dict) or key not in cur:
            return None
        cur = cur[key]
    return cur


def build_multipart(file_path, field_name="file"):
    boundary = "----zerviAB" + uuid.uuid4().hex
    filename = os.path.basename(file_path)
    ctype = mimetypes.guess_type(filename)[0] or "application/octet-stream"
    with open(file_path, "rb") as fh:
        payload = fh.read()

    head = (
        f"--{boundary}\r\n"
        f'Content-Disposition: form-data; name="{field_name}"; filename="{filename}"\r\n'
        f"Content-Type: {ctype}\r\n\r\n"
    ).encode("utf-8")
    tail = f"\r\n--{boundary}--\r\n".encode("utf-8")
    return head + payload + tail, f"multipart/form-data; boundary={boundary}"


def upload(base_url, token, file_path, memory_mode, timeout):
    body, content_type = build_multipart(file_path)
    req = urllib.request.Request(
        base_url.rstrip("/") + "/upload",
        data=body,
        method="POST",
        headers={
            "Content-Type": content_type,
            "Authorization": f"Bearer {token}",
            # on 不给头（走默认路径），off 才显式跳过，避免多传无意义的头
            **({"X-Memory-Mode": "off"} if memory_mode == "off" else {}),
        },
    )
    started = time.time()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
            status = resp.status
    except urllib.error.HTTPError as exc:
        raw = exc.read()
        status = exc.code
    except urllib.error.URLError as exc:
        return 0, int((time.time() - started) * 1000), None, f"连接失败: {exc.reason}".encode()
    elapsed_ms = int((time.time() - started) * 1000)

    parsed = None
    try:
        parsed = json.loads(raw.decode("utf-8"))
    except Exception:
        pass
    return status, elapsed_ms, parsed, raw[:400]


def one_run(args, token, memory_mode, round_idx):
    status, wall_ms, parsed, raw_head = upload(
        args.base_url, token, args.file, memory_mode, args.timeout
    )
    digest = dig(parsed or {}, ("data", "extraction_digest"))
    record = {
        "round": round_idx,
        "arm": memory_mode,
        "status": status,
        "wall_ms": wall_ms,
        "digest": digest,
        "raw_head": None if digest else raw_head.decode("utf-8", "replace"),
    }
    if digest:
        record["memory_used"] = dig(parsed, ("data", "memory_used"))
        record["memory_skip_reason"] = dig(parsed, ("data", "memory_skip_reason"))
    return record


def aggregate(runs):
    out = {}
    for arm in ("off", "on"):
        rows = [r for r in runs if r["arm"] == arm and r.get("digest")]
        if not rows:
            continue
        stats = {}
        for label, path in NUMERIC_KEYS:
            values = [dig(r["digest"], path) for r in rows]
            values = [v for v in values if isinstance(v, (int, float))]
            if not values:
                stats[label] = None
                continue
            stats[label] = {
                "median": statistics.median(values),
                "min": min(values),
                "max": max(values),
                "spread": max(values) - min(values),
            }
        skills_sets = [tuple(sorted(r["digest"].get("skills") or [])) for r in rows]
        stats["skills_identical_across_rounds"] = len(set(skills_sets)) == 1
        out[arm] = {"n": len(rows), "stats": stats}
    return out


def fmt_num(value):
    """整数不显示 .0（78 而不是 78.0），小数保留原样。"""
    if isinstance(value, float) and value.is_integer():
        return str(int(value))
    return str(value)


def render_summary(agg, runs, args):
    lines = []
    lines.append("# A/B 基线（记忆注入前）")
    lines.append("")
    lines.append(f"- 文件：`{args.file}`")
    lines.append(f"- 端点：`{args.base_url}/upload`")
    lines.append(f"- 轮数：{args.rounds}（顺序每轮交替）")
    lines.append(f"- 对照组：请求头 `X-Memory-Mode: off`；实验组：默认路径")
    lines.append("")

    bad = [r for r in runs if r["status"] != 200]
    if bad:
        lines.append(f"> ⚠️ 有 {len(bad)} 次请求非 200，先看 `runs.json` 的 `raw_head`。")
        lines.append("")

    if "off" not in agg or "on" not in agg:
        lines.append("> ❌ 缺一组数据，无法对照。检查 `UPLOAD_EXTRACTION_DIGEST=true` 是否已开。")
        return "\n".join(lines)

    off, on = agg["off"], agg["on"]
    lines.append(f"| 指标 | 无记忆 中位数 | 有记忆 中位数 | Δ(on-off) | 噪声(极差 max) |")
    lines.append("|---|---|---|---|---|")
    for label, _ in NUMERIC_KEYS:
        o = off["stats"].get(label)
        n = on["stats"].get(label)
        if not o or not n:
            continue
        delta = n["median"] - o["median"]
        noise = max(o["spread"], n["spread"])
        lines.append(
            f"| `{label}` | {fmt_num(o['median'])} | {fmt_num(n['median'])} "
            f"| {'+' if delta else ''}{fmt_num(delta)} | {fmt_num(noise)} |"
        )

    lines.append("")
    lines.append(
        f"- 技能列表两轮完全一致：无记忆={off['stats'].get('skills_identical_across_rounds')}，"
        f"有记忆={on['stats'].get('skills_identical_across_rounds')}"
    )

    os_row = off["stats"].get("quality.overall_score")
    ns_row = on["stats"].get("quality.overall_score")
    sk_o = off["stats"].get("skills_count")
    sk_n = on["stats"].get("skills_count")
    lines.append("")
    lines.append("## 判读")
    if os_row and ns_row and sk_o and sk_n:
        same = (
            os_row["median"] == ns_row["median"]
            and sk_o["median"] == sk_n["median"]
            and max(os_row["spread"], ns_row["spread"]) == 0
            and max(sk_o["spread"], sk_n["spread"]) == 0
        )
        if same:
            lines.append(
                "- **确定性达标**：两组 overall_score / skills_count 中位数与极差均为 0。"
                "注入后的稳定差值可归因于记忆本身。"
            )
        else:
            lines.append(
                "- **噪声非零**：注入前两组就已不同或组内抖动，"
                "先加大 `--rounds`（≥5）或固定环境，再谈注入收益。"
            )
    else:
        lines.append("- 缺 `quality.overall_score` / `skills_count`，无法判读（非简历文件？）。")

    skipped = [r for r in runs if r["arm"] == "on" and r.get("memory_used") is False]
    if skipped:
        lines.append(
            f"- ⚠️ 有 {len(skipped)} 次「有记忆」组实际 `memory_used=false`"
            f"（{skipped[0].get('memory_skip_reason')}）——该轮不是真实验组。"
        )
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description="记忆注入前 A/B 基线")
    ap.add_argument("--base-url", default="http://127.0.0.1:3007")
    ap.add_argument("--token", help="JWT access token")
    ap.add_argument("--token-file", help="从文件读 token（整文件内容去空白）")
    ap.add_argument("--file", required=True, help="用于上传的简历/JD 文件")
    ap.add_argument("--rounds", type=int, default=3, help="每组的轮数（默认 3）")
    ap.add_argument("--timeout", type=int, default=1800, help="单次请求超时秒数")
    ap.add_argument("--out", default=None, help="输出目录（默认 ab-results/<ts>）")
    args = ap.parse_args()

    token = args.token
    if not token and args.token_file:
        with open(args.token_file, "r", encoding="utf-8") as fh:
            token = fh.read().strip()
    if not token:
        sys.exit("缺少 --token 或 --token-file")
    if not os.path.isfile(args.file):
        sys.exit(f"文件不存在: {args.file}")

    out_dir = args.out or os.path.join(
        "ab-results", time.strftime("%Y%m%d-%H%M%S")
    )
    os.makedirs(out_dir, exist_ok=True)

    runs = []
    print(f"baseline: {args.rounds} rounds × 2 arms（顺序每轮交替）")
    for idx in range(1, args.rounds + 1):
        # 交替：奇偶轮换首组，抵消「谁先跑谁吃预热」的顺序效应
        order = ("off", "on") if idx % 2 == 1 else ("on", "off")
        for arm in order:
            rec = one_run(args, token, arm, idx)
            runs.append(rec)
            d = rec.get("digest") or {}
            print(
                f"  round {idx} {arm:3} http={rec['status']} wall={rec['wall_ms']}ms "
                f"skills={d.get('skills_count')} score={dig(d, ('quality', 'overall_score'))} "
                f"mem={rec.get('memory_used')}"
            )
            if not rec.get("digest"):
                print("    ⚠️ 没有 extraction_digest —— 服务端是否 UPLOAD_EXTRACTION_DIGEST=true？")
                print("    body:", rec.get("raw_head"))

    agg = aggregate(runs)
    with open(os.path.join(out_dir, "runs.json"), "w", encoding="utf-8") as fh:
        json.dump({"args": vars(args), "runs": runs, "aggregate": agg}, fh,
                  ensure_ascii=False, indent=2)

    summary = render_summary(agg, runs, args)
    with open(os.path.join(out_dir, "summary.md"), "w", encoding="utf-8") as fh:
        fh.write(summary + "\n")

    print()
    print(summary)
    print(f"\n已写入 {out_dir}/runs.json 与 {out_dir}/summary.md")

    if "off" not in agg or "on" not in agg:
        sys.exit(1)


if __name__ == "__main__":
    main()
