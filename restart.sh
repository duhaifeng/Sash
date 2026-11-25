#!/usr/bin/env bash
# 在 macOS 和 Linux 上重新编译并启动 sash。
# 用法：
#   ./restart.sh                 调试构建并在后台管理窗口
#   ./restart.sh --release       发布构建后再启动
#   ./restart.sh -- list         其余参数原样传给程序
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$root"

os=$(uname -s)
case "$os" in
  Darwin|Linux) ;;
  *)
    printf '只支持 macOS 和 Linux，当前系统是 %s\n' "$os" >&2
    exit 1
    ;;
esac

if ! command -v cargo >/dev/null 2>&1; then
  printf '未找到 cargo。请先安装 Rust：https://rustup.rs\n' >&2
  exit 1
fi

profile=debug
cargo_args=
if [ "${1:-}" = "--release" ]; then
  profile=release
  cargo_args=--release
  shift
fi
# 允许用 -- 分隔脚本参数和程序参数。
if [ "${1:-}" = "--" ]; then
  shift
fi

stop_running() {
  if ! command -v pgrep >/dev/null 2>&1; then
    return 0
  fi
  # 只结束本仓库编译出来的进程，避免误杀其他同名程序。
  # magnet 是改名之前的二进制，一起停掉，避免和 sash 抢同一套快捷键。
  pids=$(pgrep -f "$root/target/(debug|release)/(sash|magnet)( |$)" || true)
  if [ -z "$pids" ]; then
    return 0
  fi
  printf '停止已在运行的 sash：%s\n' "$pids"
  # shellcheck disable=SC2086
  kill $pids 2>/dev/null || true
  sleep 0.4
  pids=$(pgrep -f "$root/target/(debug|release)/(sash|magnet)( |$)" || true)
  if [ -n "$pids" ]; then
    # shellcheck disable=SC2086
    kill -9 $pids 2>/dev/null || true
  fi
}

# 先停掉旧进程，否则 Linux 上正在运行的可执行文件无法被覆盖。
stop_running

printf '正在编译（%s）…\n' "$profile"
# shellcheck disable=SC2086
cargo build $cargo_args

bin="$root/target/$profile/sash"
printf '启动 %s\n' "$bin"
exec "$bin" "$@"
