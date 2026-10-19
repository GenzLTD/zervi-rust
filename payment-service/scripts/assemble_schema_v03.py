# -*- coding: utf-8 -*-
"""拼装 genzltd_schema_v0.3.sql：
50 张存量表 DDL（gen_schema bin 产物，拓扑排序保证 FK 依赖顺序）
+ IAP 两表原文（schema_v0.1.sql，entity 不承载 DEFAULT/CHECK/索引/RLS）
"""
import re
import sys

BODY = sys.argv[1]          # gen_schema 输出
V01 = sys.argv[2]           # schema_v0.1.sql
OUT = sys.argv[3]           # shared/genzltd_schema_v0.3.sql

body = open(BODY, encoding="utf-8-sig").read()

# 按 "-- [n/50]" 分块
blocks = [b.strip() for b in re.split(r"-- \[\d+/\d+\]", body) if b.strip()]
tables, deps = {}, {}
for b in blocks:
    m = re.search(r'CREATE TABLE "(\w+)"', b)
    if not m:
        sys.exit(f"无法解析表块: {b[:60]}...")
    name = m.group(1)
    tables[name] = b
    deps[name] = set(re.findall(r'REFERENCES "(\w+)"', b)) & set(
        re.findall(r'CREATE TABLE "(\w+)"', body)
    )

# Kahn 拓扑排序（被引用的表先建）
order, placed = [], set()
pending = dict(deps)
while pending:
    ready = sorted(t for t, d in pending.items() if d <= placed)
    if not ready:
        sys.exit(f"FK 依赖成环: {sorted(pending)}")
    for t in ready:
        order.append(t)
        placed.add(t)
        del pending[t]

# IAP 段：schema_v0.1.sql 的两表 DDL + 索引 + RLS（行 27~78 原文）
v01_lines = open(V01, encoding="utf-8").read().splitlines()
iap_section = "\n".join(v01_lines[26:78])

header = """\
-- ============================================================
-- genzltd_schema_v0.3 — 全库基准（2026-10-20 重建入库）
-- ============================================================
-- 背景: genzltd_schema_v0.2.sql 从未入库（仅根 Cargo.toml 注释引用），
--       全库 DDL 基准处于"失踪"状态。本文件以
--       shared/common/src/entities 的 50 张存量 entity（sea-orm 0.12
--       codegen 产物，经 gap-close 线整理）反向重建，并并入
--       payment-service/schema_v0.1.sql 的两张 IAP 表，升级为 v0.3。
-- 生成: cargo run -p payment-service --bin gen_schema
--       （scripts/gen_schema_v03.ps1 一条龙：生成+拓扑排序+拼装）
-- 口径限制（entity 重建的已知差异）:
--   · 存量 50 表不带 DEFAULT / CHECK / 业务索引 —— 原库即由应用层
--     （sea-orm ActiveModel 全字段写入）显式赋值，行为一致
--   · FK 约束来自 entity Relation，拓扑排序后按依赖顺序建表
--   · IAP 两表保留 schema_v0.1.sql 原文（含 DEFAULT/CHECK/索引/RLS）
-- 执行: psql postgres://postgres:postgres@127.0.0.1:5433/genzltd
--       -f shared/genzltd_schema_v0.3.sql（生产 Supabase 先在测试项目验证）
-- ============================================================

-- ============================================================
-- 第一部分: 50 张存量表（entity 反向重建，FK 拓扑序）
-- ============================================================
"""

mid = """
-- ============================================================
-- 第二部分: Apple IAP 两表（schema_v0.1.sql 原文，v0.3 并入）
-- ============================================================
"""

with open(OUT, "w", encoding="utf-8", newline="\n") as f:
    f.write(header)
    for i, t in enumerate(order, 1):
        f.write(f"-- [{i}/{len(order)}] {t}\n")
        f.write(tables[t] + "\n\n")
    f.write(mid)
    f.write(iap_section + "\n")

print(f"OK: {len(order)} legacy tables (FK 拓扑序) + IAP 2 tables -> {OUT}")
