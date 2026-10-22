#!/usr/bin/env python3
"""两个 SQLite 快照的全表对拍：行数 / 主键集合 md5 / 全行 md5 / 表结构。

用途：判定两个 VACUUM 快照是否同源。

背景（Round 10 实测）：两个 30,818,304 字节的快照 sha256 可以完全不同，
但只差 1 个字节（schema_cookie 518 -> 520），而 55 张表的行数、主键集合、
全行内容、表结构全部一致。**sha256 不同 ≠ 数据不同**，必须下沉到层级 2/3/4。

对应 docs/single_source_authority_lockdown.md 第 2 项 rubric：
    L1 sha256  →  L2 主键集合  →  L3 行数+created_at 跨度  →  L4 表结构

用法：
    compare_snapshots.py <A.db> <B.db> [--label-a 名称] [--label-b 名称] [--rows-only]

    --rows-only 只比行数，跳过全行 md5（大库提速用，poems 30 万行时有用）
"""
import argparse
import hashlib
import sqlite3
import sys


def parse_args():
    ap = argparse.ArgumentParser()
    ap.add_argument('snapshot_a')
    ap.add_argument('snapshot_b')
    ap.add_argument('--label-a', default='A')
    ap.add_argument('--label-b', default='B')
    ap.add_argument('--rows-only', action='store_true',
                    help='只比行数，跳过全行 md5')
    return ap.parse_args()


def connect(path):
    return sqlite3.connect('file:%s?mode=ro' % path, uri=True)


def tables(db):
    return [r[0] for r in db.execute(
        "select name from sqlite_master where type='table' order by name")]


def cols(db, t):
    return [r[1] for r in db.execute('PRAGMA table_info("%s")' % t)]


def pk_col(db, t):
    c = cols(db, t)
    for cand in ('id', 'rowid'):
        if cand in c:
            return cand
    return c[0] if c else None


def keys_md5(db, t):
    """主键集合 md5。与 Round 7 口径一致：键排序后拼接再取 md5。"""
    pk = pk_col(db, t)
    if pk is None:
        return None
    ks = sorted(str(r[0]) for r in db.execute('select "%s" from "%s"' % (pk, t)))
    return hashlib.md5(','.join(ks).encode()).hexdigest()


def row_md5(db, t):
    """全行 md5：按首列排序，每行各列以 / 连接（None -> 空串）。"""
    c = cols(db, t)
    if not c:
        return None
    sel = ','.join('"%s"' % x for x in c)
    h = hashlib.md5()
    for row in db.execute('select %s from "%s" order by "%s"' % (sel, t, c[0])):
        h.update(('/'.join('' if v is None else str(v) for v in row)).encode())
        h.update(b'\n')
    return h.hexdigest()


def schema_inventory(db):
    """sqlite_master 的 (type, name, tbl_name) 三元组集合。"""
    return {(r[0], r[1], r[2]) for r in
            db.execute("select type,name,tbl_name from sqlite_master")}


def main():
    a = parse_args()
    da, db_ = connect(a.snapshot_a), connect(a.snapshot_b)
    ta, tb = tables(da), tables(db_)
    ia, ib = schema_inventory(da), schema_inventory(db_)

    print('表清单：%s=%d  %s=%d' % (a.label_a, len(ta), a.label_b, len(tb)))
    only = sorted(set(ta) ^ set(tb))
    print('  仅一侧有：%s' % (only or '无'))
    print('sqlite_master 条目：%d / %d' % (len(ia), len(ib)))
    for tag, d in [('仅 %s 有' % a.label_a, ia - ib),
                   ('仅 %s 有' % a.label_b, ib - ia)]:
        for typ, name, tbl in sorted(d):
            print('  %s: type=%s name=%s tbl=%s' % (tag, typ, name, tbl))

    common = [t for t in ta if t in tb]
    print()
    print('%-28s %9s %9s  %-6s %-6s %s' % (
        'table', a.label_a, a.label_b, '行数', '主键', '全行'))
    print('-' * 78)

    bad_rows, bad_keys, bad_full = [], [], []
    for t in common:
        na = da.execute('select count(*) from "%s"' % t).fetchone()[0]
        nb = db_.execute('select count(*) from "%s"' % t).fetchone()[0]
        if na == 0 and nb == 0:
            print('%-28s %9d %9d  %-6s %-6s %s' % (t, na, nb, '空', '-', '跳过'))
            continue
        ka, kb = keys_md5(da, t), keys_md5(db_, t)
        if a.rows_only:
            fa = fb = None
            mark = '-'
        else:
            fa, fb = row_md5(da, t), row_md5(db_, t)
            mark = '同' if fa == fb else '异'
        print('%-28s %9d %9d  %-6s %-6s %s' % (
            t, na, nb,
            '同' if na == nb else '异',
            '同' if ka == kb else '异', mark))
        if na != nb:
            bad_rows.append(t)
        if ka != kb:
            bad_keys.append(t)
        if (fa is not None) and fa != fb:
            bad_full.append(t)

    print()
    print('L3 行数不同的表：%s' % (bad_rows or '无'))
    print('L2 主键集合不同的表：%s' % (bad_keys or '无'))
    if a.rows_only:
        print('L1 全行 md5：已用 --rows-only 跳过')
    else:
        print('L1 全行 md5 不同的表：%s' % (bad_full or '无'))

    # 表结构（L4）：CREATE TABLE 语句逐字比对
    defs = []
    for t in common:
        sa = da.execute(
            "select sql from sqlite_master where type='table' and name=?", (t,)).fetchone()
        sb = db_.execute(
            "select sql from sqlite_master where type='table' and name=?", (t,)).fetchone()
        if (sa and sa[0]) != (sb and sb[0]):
            defs.append(t)
    print('L4 表定义不同的表：%s' % (defs or '无'))

    same = not (bad_rows or bad_keys or bad_full or defs or only)
    print()
    print('结论：%s' % ('同源（L2/L3/L4 全同）' if same else '不同源，见上表'))
    return 0 if same else 2


if __name__ == '__main__':
    sys.exit(main())
