#!/bin/sh
# 依存クレートのライセンスの一覧 licenses/THIRD-PARTY.md を cargo-about で作る。
# 依存を足したとき、上げたときに実行する。`--check` を付けると作り直して比べ、違えば失敗する。
# cargo-about は `cargo install cargo-about --locked --features cli` で入れる。
set -eu
cd "$(dirname "$0")/.."
OUT=licenses/THIRD-PARTY.md
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT
cargo about generate --workspace --fail --locked -o "$TMP" about.hbs
# 改行を LF に揃える（Windows で作っても同じ中身になるように）。
tr -d '\r' < "$TMP" > "$TMP.lf" && mv "$TMP.lf" "$TMP"
if [ "${1:-}" = "--check" ]; then
    if tr -d '\r' < "$OUT" | cmp -s - "$TMP"; then
        echo "$OUT は最新"
    else
        echo "$OUT が古い（sh scripts/gen-licenses.sh を実行する）" >&2
        tr -d '\r' < "$OUT" | diff - "$TMP" | head -20 >&2 || true
        exit 1
    fi
else
    cp "$TMP" "$OUT"
    echo "$OUT ($(wc -c < "$OUT") バイト)"
fi
