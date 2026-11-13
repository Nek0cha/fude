#!/bin/sh
# WSL の中で実行する。WSL に置く Linux 用の fude（fude-linux）を静的リンクでビルドし、
# target/linux/fude-linux と、その版の印 target/linux/fude-linux.id を作る。
# Windows の fude.exe はビルド時にこの 2 つを埋め込むため、fude.exe より先に実行する。
set -eu
cd "$(dirname "$0")/.."
TARGET=x86_64-unknown-linux-musl
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/fude-target}"
cargo build -p fude-wsl --release --target "$TARGET"
mkdir -p target/linux
cp "$CARGO_TARGET_DIR/$TARGET/release/fude-linux" target/linux/fude-linux.tmp
mv target/linux/fude-linux.tmp target/linux/fude-linux
target/linux/fude-linux version > target/linux/fude-linux.id
echo "target/linux/fude-linux ($(cat target/linux/fude-linux.id))"
