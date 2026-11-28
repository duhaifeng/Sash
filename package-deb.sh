#!/usr/bin/env bash
# 把 snapgrid 打成 linux/amd64 的 .deb。
# 在 Linux x86_64 上直接编译；在 macOS 或其他架构上通过 Docker 编译。
# 用法：./package-deb.sh
# 产物：dist/snapgrid_<版本>_amd64.deb
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$root"

version=$(awk -F '"' '/^version = / { print $2; exit }' "$root/Cargo.toml")
if [ -z "$version" ]; then
  printf '无法从 Cargo.toml 读取版本号\n' >&2
  exit 1
fi

if [ -z "${DEB_MAINTAINER:-}" ]; then
  name=$(git -C "$root" config user.name 2>/dev/null || true)
  email=$(git -C "$root" config user.email 2>/dev/null || true)
  if [ -n "$name" ] && [ -n "$email" ]; then
    DEB_MAINTAINER="$name <$email>"
  else
    DEB_MAINTAINER="snapgrid maintainers <snapgrid@localhost>"
  fi
fi

# 程序用纯 Rust 实现 X11 协议，编译不需要 libxcb 等系统图形库。
build_deps='build-essential ca-certificates dpkg-dev file binutils'

install_build_deps() {
  if [ "$(id -u)" -ne 0 ]; then
    printf '缺少编译依赖。请用 root 安装：\n  sudo apt-get install %s\n' "$build_deps" >&2
    exit 1
  fi
  export DEBIAN_FRONTEND=noninteractive
  apt-get update
  # shellcheck disable=SC2086
  apt-get install -y --no-install-recommends $build_deps
}

ensure_native_tools() {
  missing=
  for cmd in cargo dpkg-deb strip file; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
      missing="$missing $cmd"
    fi
  done
  if [ -n "$missing" ]; then
    printf '缺少工具：%s\nDebian/Ubuntu 可安装：\n  sudo apt-get install %s\n并安装 Rust：https://rustup.rs\n' "$missing" "$build_deps" >&2
    exit 1
  fi
}

pack_deb() {
  if [ -z "${CARGO_TARGET_DIR:-}" ]; then
    export CARGO_TARGET_DIR="$root/target"
  fi
  printf '正在为 x86_64-unknown-linux-gnu 编译发布版本…\n'
  cargo build --release --locked --target x86_64-unknown-linux-gnu

  bin="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/snapgrid"
  if [ ! -f "$bin" ]; then
    printf '没有找到编译产物：%s\n' "$bin" >&2
    exit 1
  fi
  if command -v file >/dev/null 2>&1 && ! file "$bin" | grep -q 'x86-64'; then
    printf '产物不是 amd64 可执行文件：%s\n' "$(file "$bin")" >&2
    exit 1
  fi

  stage=$(mktemp -d)
  trap 'rm -rf "$stage"' EXIT
  install -d -m 0755 \
    "$stage/DEBIAN" \
    "$stage/usr/bin" \
    "$stage/usr/share/applications" \
    "$stage/usr/share/icons/hicolor/scalable/apps" \
    "$stage/etc/xdg/autostart" \
    "$stage/usr/share/gnome-shell/extensions/snapgrid@snapgrid.local/icons"
  install -m 0755 "$bin" "$stage/usr/bin/snapgrid"
  strip --strip-unneeded "$stage/usr/bin/snapgrid"

  cat > "$stage/usr/share/applications/snapgrid.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=SnapGrid
GenericName=Window Tiling
Comment=用快捷键或拖拽把窗口吸附到屏幕分区
Exec=snapgrid
Icon=snapgrid
Terminal=false
Categories=Utility;
Keywords=window;tiling;snap;
StartupNotify=false
EOF
  install -m 0644 "$stage/usr/share/applications/snapgrid.desktop" \
    "$stage/etc/xdg/autostart/snapgrid.desktop"

  install -m 0644 "$root/assets/snapgrid.svg" \
    "$stage/usr/share/icons/hicolor/scalable/apps/snapgrid.svg"

  # GNOME Wayland 后端用的 Shell 扩展。装成系统级，所有用户登录后都能加载；
  # postinst 负责为已有用户启用它。
  ext_src="$root/gnome/snapgrid@snapgrid.local"
  ext_dst="$stage/usr/share/gnome-shell/extensions/snapgrid@snapgrid.local"
  for f in metadata.json extension.js prefs.js icons/snapgrid-symbolic.svg; do
    install -m 0644 "$ext_src/$f" "$ext_dst/$f"
  done
  install -m 0755 "$root/assets/deb/postinst" "$stage/DEBIAN/postinst"

  shlibs=
  if command -v dpkg-shlibdeps >/dev/null 2>&1; then
    set +e
    # The log stays outside the package tree: anything left in $stage is packed
    # into the deb, and another package may already own the same path.
    shlibs_err=$(mktemp)
    # dpkg-shlibdeps is meant for a Debian source tree and insists on
    # debian/control in the current directory. Give it a minimal one.
    shlibs_dir=$(mktemp -d)
    mkdir "$shlibs_dir/debian"
    printf 'Source: snapgrid\nSection: x11\nPriority: optional\nMaintainer: snapgrid\n\nPackage: snapgrid\nArchitecture: amd64\nDescription: snapgrid\n' \
      > "$shlibs_dir/debian/control"
    shlibs_out=$(cd "$shlibs_dir" && dpkg-shlibdeps --ignore-missing-info -O "$stage/usr/bin/snapgrid" 2>"$shlibs_err")
    shlibs_status=$?
    rm -rf "$shlibs_dir"
    set -e
    if [ "$shlibs_status" -ne 0 ] && [ -s "$shlibs_err" ]; then
      printf 'dpkg-shlibdeps 提示：\n' >&2
      cat "$shlibs_err" >&2
    fi
    rm -f "$shlibs_err"
    shlibs=$(printf '%s\n' "$shlibs_out" | sed -n 's/^shlibs:Depends=//p')
  fi
  if [ -z "$shlibs" ]; then
    shlibs='libc6 (>= 2.31), libgcc-s1'
  fi

  installed_size=$(du -sk "$stage" | awk '{print $1}')
  # 0.1.0 的包名是 sash，和 Ubuntu 自带的独立 shell（目前 3.8）撞名。
  # 只替换 1.0 之前的那个包，不动系统里的 sash。
  cat > "$stage/DEBIAN/control" <<EOF
Package: snapgrid
Version: $version
Section: x11
Priority: optional
Architecture: amd64
Maintainer: $DEB_MAINTAINER
Installed-Size: $installed_size
Depends: $shlibs
Conflicts: sash (<< 1.0)
Replaces: sash (<< 1.0)
Description: snap windows by shortcut or by dragging
 把窗口吸附到半屏、三分之一和角落。支持 X11（需遵循 EWMH 的窗口管理器）和
 GNOME Wayland（自带 Shell 扩展，安装后重新登录一次即可加载）。
 安装后会在登录时自动启动。命令是 snapgrid。
EOF

  mkdir -p "$root/dist"
  out="$root/dist/snapgrid_${version}_amd64.deb"
  # 清掉所有旧版本的 amd64 deb，dist 里只留这次生成的；不碰 macOS 的产物。
  rm -f "$root"/dist/snapgrid_*_amd64.deb "$root"/dist/sash_*_amd64.deb
  deb_args=
  if dpkg-deb --help 2>&1 | grep -q root-owner-group; then
    deb_args=--root-owner-group
  fi
  # shellcheck disable=SC2086
  dpkg-deb $deb_args --build "$stage" "$out"
  if [ -n "${HOST_UID:-}" ] && [ -n "${HOST_GID:-}" ]; then
    chown "$HOST_UID:$HOST_GID" "$out" "$root/dist" 2>/dev/null || true
  fi
  printf '已生成 %s\n' "$out"
}

run_in_docker() {
  if ! command -v docker >/dev/null 2>&1; then
    printf '当前系统不是 Linux amd64，需要 Docker 才能打 amd64 deb。\n' >&2
    exit 1
  fi
  if ! docker info >/dev/null 2>&1; then
    printf 'Docker 没有在运行。请先启动 Docker 后再执行 ./package-deb.sh\n' >&2
    exit 1
  fi
  printf '使用 Docker（linux/amd64）打包…\n'
  mkdir -p "$root/dist"
  # 只挂载源码和产物目录，避免把本机 target 送进容器。
  docker run --rm --platform linux/amd64 \
    -e IN_DOCKER=1 \
    -e DEB_MAINTAINER="$DEB_MAINTAINER" \
    -e HOST_UID="$(id -u)" \
    -e HOST_GID="$(id -g)" \
    -e CARGO_TARGET_DIR=/tmp/snapgrid-target \
    -v "$root/Cargo.toml:/work/Cargo.toml:ro" \
    -v "$root/Cargo.lock:/work/Cargo.lock:ro" \
    -v "$root/src:/work/src:ro" \
    -v "$root/assets:/work/assets:ro" \
    -v "$root/gnome:/work/gnome:ro" \
    -v "$root/package-deb.sh:/work/package-deb.sh:ro" \
    -v "$root/dist:/work/dist" \
    -v snapgrid-cargo-registry:/usr/local/cargo/registry \
    -v snapgrid-cargo-git:/usr/local/cargo/git \
    -w /work \
    rust:1-bookworm \
    bash /work/package-deb.sh --inside
}

if [ "${1:-}" = "--inside" ] || [ "${IN_DOCKER:-}" = "1" ]; then
  if [ "$(uname -s)" != "Linux" ] || [ "$(uname -m)" != "x86_64" ]; then
    printf '打包步骤必须在 Linux x86_64 环境中运行。\n' >&2
    exit 1
  fi
  if [ "${IN_DOCKER:-}" = "1" ]; then
    install_build_deps
  else
    ensure_native_tools
  fi
  if command -v rustup >/dev/null 2>&1; then
    rustup target add x86_64-unknown-linux-gnu
  fi
  pack_deb
  exit 0
fi

if [ "$(uname -s)" = "Linux" ] && [ "$(uname -m)" = "x86_64" ]; then
  ensure_native_tools
  if command -v rustup >/dev/null 2>&1; then
    rustup target add x86_64-unknown-linux-gnu
  fi
  pack_deb
else
  run_in_docker
fi
