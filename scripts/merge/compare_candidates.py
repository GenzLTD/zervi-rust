#!/usr/bin/env python3
"""候选三方业务表对账 —— 对应 docs/single_source_authority_lockdown.md 第 2 项。

目的：判定「哪个 SQLite 是权威源」。

Round 10 实测结论（本脚本第一次跑出来的）：
    清单第 0 项列了 3 个候选，但此前只跑过 Vultr 那一方。
    拉进 Mac 本地 looma.db 后发现：Mac 与 SG 的 users 共同 UUID 只有 1 个，
    各独有 42 / 10 —— 三方不是副本/新旧关系，而是三个独立演化的库，
    没有任何一方是另外两方的超集。

用法：
    compare_candidates.py 名称=路径.db [名称=路径.db ...] [--poems]

    --poems   连 poems 一起对比（默认排除；它占 99.8% 行数会淹没判据，
              且 30 万行级对比很慢）

示例：
    compare_candidates.py \
      Mac-local=$HOME/Projects/looma-zervi/backend/data/looma.db \
      SG-1022=$HOME/snapshots/looma-sg-20261022.db
"""
import argparse
import sqlite3
import sys

# 清单第 2 项点名的业务表。poems 是派生表，默认排除。
BIZ = ['users', 'consents', 'timeline_events', 'orders', 'query_logs',
       'candidates', 'llm_usage', 'signal_dlq', 'signal_outbox',
       'trust_memories', 'applications', 'documents', 'enterprises',
       'game_profiles', 'quota_records', 'mission_completions']


def parse_args():
    ap = argparse.ArgumentParser()
    ap.add_argument('candidates', nargs='+',
                    help='形如 名称=路径.db，至少两个')
    ap.add_argument('--poems', action='store_true')
    return ap.parse_args()


def connect(path):
    return sqlite3.connect('file:%s?mode=ro' % path, uri=True)


def has_table(db, t):
    return db.execute(
        "select count(*) from sqlite_master where type='table' and name=?",
        (t,)).fetchone()[0] == 1


def cols(db, t):
    return [r[1] for r in db.execute('PRAGMA table_info("%s")' % t)]


def pk_col(db, t):
    c = cols(db, t)
    for cand in ('id', 'rowid'):
        if cand in c:
            return cand
    return c[0] if c else None


def snap(db, t):
    if not has_table(db, t):
        return None
    c = cols(db, t)
    pk = pk_col(db, t)
    n = db.execute('select count(*) from "%s"' % t).fetchone()[0]
    keys = set()
    if n and pk:
        try:
            keys = {str(r[0]) for r in
                    db.execute('select "%s" from "%s"' % (pk, t))}
        except Exception:
            keys = None
    span = None
    if 'created_at' in c and n:
        r = db.execute(
            'select min(created_at), max(created_at) from "%s"' % t).fetchone()
        span = ((str(r[0]) or '')[:19] + ' .. ' + (str(r[1]) or '')[:19])
    return {'n': n, 'keys': keys, 'span': span}


def main():
    a = parse_args()
    cands = {}
    for item in a.candidates:
        if '=' not in item:
            print('参数必须形如 名称=路径.db：%s' % item, file=sys.stderr)
            return 1
        name, path = item.split('=', 1)
        cands[name] = connect(path)

    tabs = list(BIZ) + (['poems'] if a.poems else [])
    names = list(cands)

    # ---- 业务表对拍
    hdr = '%-20s' % 'table' + ''.join('%10s' % n for n in names)
    print(hdr)
    print('-' * (20 + 10 * len(names)))
    for t in tabs:
        snaps = {n: snap(cands[n], t) for n in names}
        if all(s is None for s in snaps.values()):
            print('%-20s  (全部缺失)' % t)
            continue
        cells = ''
        for n in names:
            s = snaps[n]
            cells += '%10s' % ('-' if s is None else s['n'])
        print('%-20s%s' % (t, cells))
        for n in names:
            s = snaps[n]
            if s and s['span']:
                print('      %-12s created_at: %s' % (n, s['span']))

    print()
    print('=== 主键集合关系（两两） ===')
    for i in range(len(names)):
        for j in range(i + 1, len(names)):
            x, y = names[i], names[j]
            print('--- %s  vs  %s' % (x, y))
            for t in tabs:
                sx, sy = snap(cands[x], t), snap(cands[y], t)
                if sx is None or sy is None:
                    continue
                kx, ky = sx['keys'], sy['keys']
                if kx is None or ky is None:
                    continue
                if kx == ky:
                    continue
                if kx > ky:
                    rel = '%s ⊃ %s，多 %d' % (x, y, len(kx - ky))
                elif ky > kx:
                    rel = '%s ⊃ %s，多 %d' % (y, x, len(ky - kx))
                else:
                    rel = '交叉：各独有多 %d / %d' % (len(kx - ky), len(ky - kx))
                print('    %-20s %s' % (t, rel))

    # ---- users 专项：UUID 与 email 分开看（同 email 可能不同 UUID）
    print()
    print('=== users 专项（UUID 集合 vs email 集合分开看） ===')
    store = {}
    for n in names:
        db = cands[n]
        if not has_table(db, 'users'):
            continue
        c = cols(db, 'users')
        uuids = {str(r[0]) for r in db.execute('select id from users')} \
            if 'id' in c else set()
        emails = {str(r[0]) for r in db.execute('select email from users')} \
            if 'email' in c else set()
        store[n] = (uuids, emails)
        print('%s: uuids=%d emails=%d' % (n, len(uuids), len(emails)))

    for i in range(len(names)):
        for j in range(i + 1, len(names)):
            x, y = names[i], names[j]
            if x not in store or y not in store:
                continue
            ux, ex = store[x]
            uy, ey = store[y]
            print()
            print('%s vs %s' % (x, y))
            print('  UUID  仅%s有: %d  仅%s有: %d  共同: %d'
                  % (x, len(ux - uy), y, len(uy - ux), len(ux & uy)))
            print('  email 仅%s有: %d  仅%s有: %d  共同: %d'
                  % (x, len(ex - ey), y, len(ey - ex), len(ex & ey)))
            if len(ex - ey) <= 30:
                print('  仅%s有的 email: %s' % (x, sorted(ex - ey)))
            if len(ey - ex) <= 30:
                print('  仅%s有的 email: %s' % (y, sorted(ey - ex)))

    # ---- 判定提示
    print()
    print('=== 判定提示（清单第 3 项：(a) 绿 (b) 最新 (c) PK 不落后）===')
    for i in range(len(names)):
        for j in range(i + 1, len(names)):
            x, y = names[i], names[j]
            sx, sy = snap(cands[x], 'users'), snap(cands[y], 'users')
            if not sx or not sy:
                continue
            print('  %s: users=%d span=%s' % (x, sx['n'], sx['span']))
            print('  %s: users=%d span=%s' % (y, sy['n'], sy['span']))
    return 0


if __name__ == '__main__':
    sys.exit(main())
