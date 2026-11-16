#!/bin/sh
# アプリのアイコン assets/icon/fude.ico（Windows）と fude.icns（macOS）を、assets/icon/fude.svg と fude-small.svg から作る
# （ImageMagick 7 の magick と python が要る）。
# アイコンの SVG を変えたときに実行し、できた fude.ico と fude.icns もコミットする。
set -eu
cd "$(dirname "$0")/.."
SVG=assets/icon/fude.svg
# 16〜32 px は線を太くした小さい用から作る（細い線がにじんで消えるため）。
SMALL=assets/icon/fude-small.svg
OUT=assets/icon/fude.ico
# Windows の magick は /tmp のような MSYS のパスを読めないので、リポジトリの中の相対パスを使う。
TMP=target/gen-icon
rm -rf "$TMP" && mkdir -p "$TMP"
trap 'rm -rf "$TMP"' EXIT
for s in 256 64 48 40 32 24 20 16; do
    src="$SVG"
    [ "$s" -le 32 ] && src="$SMALL"
    magick -background none -density 384 "$src" -resize "${s}x${s}" "PNG32:$TMP/$s.png"
done
magick "$TMP/256.png" "$TMP/64.png" "$TMP/48.png" "$TMP/40.png" "$TMP/32.png" "$TMP/24.png" "$TMP/20.png" "$TMP/16.png" "$OUT"
echo "$OUT"

# macOS の .icns。ImageMagick は ICNS を書けないので、PNG を ICNS の入れ物に並べる。
# 入れ物の形：'icns' と全体の長さ（ビッグエンディアン 4 バイト）のあとに、種類（4 文字）・長さ・PNG の組が続く。
for s in 1024 512 256 128 64 32 16; do
    src="$SVG"
    [ "$s" -le 32 ] && src="$SMALL"
    [ -f "$TMP/$s.png" ] || magick -background none -density 1536 "$src" -resize "${s}x${s}" "PNG32:$TMP/$s.png"
done
python - "$TMP" assets/icon/fude.icns <<'EOF'
import struct, sys
tmp, out = sys.argv[1], sys.argv[2]
kinds = [("icp4", 16), ("icp5", 32), ("icp6", 64), ("ic07", 128), ("ic08", 256), ("ic09", 512),
         ("ic10", 1024), ("ic11", 32), ("ic12", 64), ("ic13", 256), ("ic14", 512)]
body = b""
for kind, size in kinds:
    png = open(f"{tmp}/{size}.png", "rb").read()
    body += kind.encode() + struct.pack(">I", len(png) + 8) + png
open(out, "wb").write(b"icns" + struct.pack(">I", len(body) + 8) + body)
EOF
echo assets/icon/fude.icns
