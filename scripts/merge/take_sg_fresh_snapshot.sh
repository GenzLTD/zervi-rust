#!/usr/bin/env bash
# 在 SG（Vultr 境外）活动库上取一致性快照并取回本机。
#
# 目的：满足 docs/single_source_authority_lockdown.md 第 1 项
#       「对实时源做 fresh 快照，别用过期副本」。
#
# 安全边界（务必逐条确认后再跑）：
#   - 全程只读活动库：唯一写入是 VACUUM INTO 生成的新文件，不修改原库
#   - 不进入容器的交互式 shell，不重启容器，不改 /app/data 下任何文件
#   - 取回后自动删除容器内临时快照
#
# 前置：本机 ~/.ssh/config 有 Host vultr-overseas（默认 139.180.184.25 / root）
#
# 用法：
#   ./take_sg_fresh_snapshot.sh [输出目录] [SSH Host]
#
# 示例：
#   ./take_sg_fresh_snapshot.sh ~/snapshots
#   ./take_sg_fresh_snapshot.sh ~/snapshots vultr-overseas
set -euo pipefail

OUT_DIR="${1:-$HOME/snapshots}"
HOST="${2:-vultr-overseas}"
CONTAINER="${CONTAINER:-looma-backend}"
SRC_DB="${SRC_DB:-/app/data/looma.db}"

stamp="$(date -u +%Y%m%d-%H%M%S-utc)"
snap_name="looma-sg-fresh-${stamp}.db"
local_path="${OUT_DIR}/${snap_name}"

mkdir -p "$OUT_DIR"

echo "==> 目标机：${HOST}  容器：${CONTAINER}  源库：${SRC_DB}"
echo "==> 快照名：${snap_name}"

# ---------------------------------------------------------------- 1. 现场画像
echo "==> [1/5] 读取容器现场（容器时区务必确认，很多容器是 UTC 不是北京时间）"
ssh -o BatchMode=yes -o ConnectTimeout=20 "$HOST" \
  "docker exec ${CONTAINER} sh -c 'date; date -u; ls -la ${SRC_DB}'"

# ------------------------------------------------------------ 2. VACUUM INTO
echo "==> [2/5] 在活动库上 VACUUM INTO（只读，不写原库）"
# 注意：ssh 会把命令串交给远端 shell 再解析一层，所以任何引号嵌套都会被吃掉。
# 这里改用 heredoc 经 stdin 送给 `docker exec -i … python3 -`，脚本体原样传输。
remote_out="$(ssh -o BatchMode=yes -o ConnectTimeout=60 "$HOST" \
  "docker exec -i ${CONTAINER} python3 -" <<PY
import sqlite3, os, datetime

src = '${SRC_DB}'
dst = '/tmp/${snap_name}'
if os.path.exists(dst):
    os.remove(dst)

def fmtobj(st):
    return (st.st_size,
            datetime.datetime.fromtimestamp(st.st_mtime, datetime.UTC)
            .strftime('%Y-%m-%d %H:%M:%S UTC'))

before = os.stat(src)
con = sqlite3.connect('file:%s?mode=ro' % src, uri=True)
# VACUUM INTO 不支持参数绑定（会报 parameters are not allowed），只能拼接。
# dst 是本脚本内部生成的时间戳文件名，不接受外部输入，无注入面。
con.execute("VACUUM INTO '%s'" % dst)
con.close()
after = os.stat(src)

bs, bm = fmtobj(before)
as_, am = fmtobj(after)
ss, _ = fmtobj(os.stat(dst))
print('BEFORE_SIZE=%d' % bs)
print('BEFORE_MTIME=%s' % bm)
print('AFTER_SIZE=%d' % as_)
print('AFTER_MTIME=%s' % am)
print('SNAP_SIZE=%d' % ss)
PY
)"

# 指纹交给容器内的 sha256sum，避免在容器里把 30MB 读进 python
remote_sha1="$(ssh -o BatchMode=yes -o ConnectTimeout=60 "$HOST" \
  "docker exec ${CONTAINER} sh -c 'sha256sum /tmp/${snap_name}'" | awk '{print $1}')"
remote_out="${remote_out}"$'\n'"SHA256=${remote_sha1}"

echo "$remote_out" | sed 's/^/    /'

# 活动库 size/mtime 必须前后一致，否则说明取快照期间有并发写
before_size="$(echo "$remote_out" | sed -n 's/^BEFORE_SIZE=\([0-9]*\).*/\1/p')"
after_size="$(echo "$remote_out" | sed -n 's/^AFTER_SIZE=\([0-9]*\).*/\1/p')"
if [ -n "$before_size" ] && [ "$before_size" != "$after_size" ]; then
  echo "!! 警告：活动库 size 在快照期间发生变化（${before_size} -> ${after_size}）" >&2
fi

# ------------------------------------------------------------------ 3. 取回
echo "==> [3/5] 流式取回到 ${local_path}"
# 注意：文件在容器内，scp 走的是宿主机，必须用 docker exec cat 流式传输
ssh -o BatchMode=yes -o ConnectTimeout=60 "$HOST" \
  "docker exec ${CONTAINER} cat /tmp/${snap_name}" > "$local_path"

# ---------------------------------------------------------------- 4. 校验
echo "==> [4/5] 本地复算指纹并与远端比对"
# remote_sha1 已在 [2/5] 用容器内的 sha256sum 取到，这里只在本机复算
local_sha="$(shasum -a 256 "$local_path" | awk '{print $1}')"
local_size="$(wc -c < "$local_path" | tr -d ' ')"
echo "    size  = ${local_size}"
echo "    sha256(远端) = ${remote_sha1}"
echo "    sha256(本机) = ${local_sha}"
if [ "$local_sha" != "$remote_sha1" ]; then
  echo "!! 传输不完整，指纹不一致" >&2
  exit 1
fi
echo "    指纹一致 ✅"

# ---------------------------------------------------------------- 5. 清理
echo "==> [5/5] 清理容器内临时快照"
ssh -o BatchMode=yes -o ConnectTimeout=20 "$HOST" \
  "docker exec ${CONTAINER} sh -c 'rm -f /tmp/${snap_name}; ls /tmp | head -5'"

echo "==> 完成：${local_path}"
echo "==> 下一步：compare_snapshots.py 与既有快照对拍（sha256 不同 ≠ 数据不同）"
