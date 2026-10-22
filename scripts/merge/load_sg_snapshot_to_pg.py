#!/usr/bin/env python3
"""把 SG SQLite 快照导入目标 PG 的 sg schema（合并 SQL 草案的「前置步骤 0」）。

用法：
  python3 scripts/merge/load_sg_snapshot_to_pg.py \
      --sqlite /path/to/looma-sg-20261022.db \
      --psql "/Applications/ServBay/bin/psql -h /tmp/pgmerge -p 55432 -U looma -d genzltd"

行为：
  * drop schema if exists sg cascade -> create schema sg
  * 对 SQLite 里每个表，按【PG 侧 public 同名表的列顺序】建 sg 表并 COPY 进去
    （SG 缺的列留空，SG 多出的列忽略 —— 例如 driver_profiles.status）
  * sg 表不带任何约束（NOT NULL 也去掉），纯粹当数据容器
  * 只对 sg schema 写入；public 一行不动

注意：这个脚本会 DROP 目标库的 sg schema。它不碰 public。
把 --psql 指向生产库之前，必须先有新鲜 pg_dump 备份。
"""
import argparse
import os
import shlex
import sqlite3
import subprocess

DEFAULT_SQLITE = 'file:/Users/jason/WorkBuddy/2026-10-21-06-57-56/snapshots/looma-sg-20261022.db?mode=ro'


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--sqlite', default=DEFAULT_SQLITE,
                    help='SG SQLite 快照路径（建议带 ?mode=ro）')
    ap.add_argument('--psql', required=True,
                    help='psql 命令行前缀，例: "/path/psql -h /tmp -p 55432 -U looma -d genzltd"')
    ap.add_argument('--copy-dir', default='/tmp/pgmerge/copy')
    args = ap.parse_args()

    # -A -t：去掉表头与分隔线，pg_cols 才能拿到干净的列清单
    base = shlex.split(args.psql) + ['-v', 'ON_ERROR_STOP=1', '-q', '-A', '-t']

    def psql(sql, capture=False):
        r = subprocess.run(base + ['-c', sql], capture_output=True, text=True)
        if r.returncode != 0:
            raise SystemExit(f'PSQL FAIL: {sql[:200]}\n{r.stderr}')
        return r.stdout if capture else None

    def pg_cols(table):
        out = psql("select column_name from information_schema.columns "
                   f"where table_schema='public' and table_name='{table}' "
                   "order by ordinal_position", capture=True)
        return [l.strip() for l in out.split('\n')
                if l.strip() and not l.startswith('-') and not l.endswith('row)')
                and not l.endswith('rows)') and l.strip() != '(0 rows)']

    uri = args.sqlite if args.sqlite.startswith('file:') else f'file:{args.sqlite}?mode=ro'
    db = sqlite3.connect(uri, uri=True)
    db.text_factory = lambda b: b.decode('utf-8', 'replace')

    tables = [r[0] for r in db.execute(
        "select name from sqlite_master where type='table' "
        "and name not like 'sqlite_%' order by name")]

    psql('drop schema if exists sg cascade')
    psql('create schema sg')
    os.makedirs(args.copy_dir, exist_ok=True)

    report = []
    for t in tables:
        pgc = pg_cols(t)
        if not pgc:
            print(f'SKIP {t}: public 侧没有同名表')
            continue
        sqc = [r[1] for r in db.execute(f'PRAGMA table_info("{t}")')]
        missing = [c for c in sqc if c not in pgc]
        idx = {c: i for i, c in enumerate(sqc)}

        psql(f'create table sg."{t}" (like public."{t}" excluding all)')
        # 容器不需要约束：去掉 NOT NULL，避免 SG 侧缺列（如 driver_profiles.status）导入失败
        psql(f"""do $$ declare r record; begin
              for r in select column_name from information_schema.columns
                       where table_schema='sg' and table_name='{t}' and is_nullable='NO' loop
                execute format('alter table sg."{t}" alter column %I drop not null', r.column_name);
              end loop; end $$;""")

        path = os.path.join(args.copy_dir, f'{t}.tsv')
        n = 0
        with open(path, 'w', encoding='utf-8') as f:
            for row in db.execute(f'select * from "{t}"'):
                vals = []
                for c in pgc:
                    if c not in idx:
                        vals.append('\\N')
                        continue
                    v = row[idx[c]]
                    if v is None:
                        vals.append('\\N')
                    else:
                        s = str(v)
                        vals.append(s.replace('\\', '\\\\').replace('\n', '\\n')
                                     .replace('\r', '\\r').replace('\t', '\\t'))
                f.write('\t'.join(vals) + '\n')
                n += 1
        cols_txt = ','.join(f'"{c}"' for c in pgc)
        psql(f"copy sg.\"{t}\" ({cols_txt}) from '{path}' with (format text, encoding 'UTF8')")
        report.append((t, n, ','.join(missing) or '-'))

    print(f'{"table":24}{"rows":>8}  SG-only-cols')
    for t, n, m in report:
        print(f'{t:24}{n:8}  {m}')
    print('total tables:', len(report))


if __name__ == '__main__':
    main()
