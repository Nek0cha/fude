#!/bin/bash
# mac の配布物を作る（10d の設計書 §5.2）。fude.app を組んで署名し、dist/fude-<版>-macos-arm64.dmg にする。
# mac の上で実行する（CI の macos のジョブも同じものを使う）。使い方: bash scripts/build-macos.sh
#
# 署名と公証は環境変数で選ぶ。何も無ければ ad-hoc 署名にし、公証しない（失敗にしない）。
#   MACOS_CERT_P12_BASE64, MACOS_CERT_PASSWORD : Developer ID Application の証明書（.p12 を base64 にしたもの）とパスワード。
#                                                 あれば一時的なキーチェーンに入れて署名する（hardened runtime 付き）。
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD : 公証の資格（App 用パスワード）。証明書があり、3 つ揃っていれば公証して staple する。
# Developer ID の署名と公証の経路は、証明書が無いため未確認。
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET=aarch64-apple-darwin
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
# crates/fude/Cargo.toml の [package] の版。Info.plist には数字の部分（-rc.1 などを除いたもの）を入れる。
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' crates/fude/Cargo.toml | head -n 1)
[ -n "$VERSION" ] || { echo "crates/fude/Cargo.toml から版を読めない" >&2; exit 1; }
PLIST_VERSION="${VERSION%%-*}"

cargo build --release -p fude --locked --target "$TARGET"

OUT=dist/macos
APP="$OUT/fude.app"
DMG="dist/fude-$VERSION-macos-arm64.dmg"
rm -rf "$OUT" "$DMG"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$TARGET_DIR/$TARGET/release/fude" "$APP/Contents/MacOS/fude"
cp assets/icon/fude.icns "$APP/Contents/Resources/fude.icns"
sed "s/@VERSION@/$PLIST_VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"

# 署名。証明書があれば一時的なキーチェーンに入れて Developer ID で、無ければ ad-hoc で署名する。
IDENTITY="-"
KEYCHAIN=""
OLD_KEYCHAINS=""
cleanup() {
    if [ -n "$KEYCHAIN" ]; then
        # shellcheck disable=SC2086
        security list-keychains -d user -s $OLD_KEYCHAINS || true
        security delete-keychain "$KEYCHAIN" || true
    fi
}
trap cleanup EXIT

if [ -n "${MACOS_CERT_P12_BASE64:-}" ]; then
    : "${MACOS_CERT_PASSWORD:?MACOS_CERT_P12_BASE64 があるのに MACOS_CERT_PASSWORD が無い}"
    KEYCHAIN="$(mktemp -d)/fude-sign.keychain-db"
    KEYCHAIN_PASSWORD="$(uuidgen)"
    P12="$(dirname "$KEYCHAIN")/cert.p12"
    echo "$MACOS_CERT_P12_BASE64" | base64 --decode > "$P12"
    OLD_KEYCHAINS=$(security list-keychains -d user | tr -d '"')
    security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
    security set-keychain-settings -lut 21600 "$KEYCHAIN"
    security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
    security import "$P12" -k "$KEYCHAIN" -P "$MACOS_CERT_PASSWORD" -T /usr/bin/codesign
    security set-key-partition-list -S apple-tool:,apple: -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" > /dev/null
    # shellcheck disable=SC2086
    security list-keychains -d user -s "$KEYCHAIN" $OLD_KEYCHAINS
    rm -f "$P12"
    IDENTITY="Developer ID Application"
    echo "Developer ID で署名する"
    codesign --force --options runtime --timestamp --keychain "$KEYCHAIN" --sign "$IDENTITY" "$APP"
else
    echo "MACOS_CERT_P12_BASE64 が無いので ad-hoc で署名する"
    codesign --force --sign - "$APP"
fi
codesign --verify --deep --strict --verbose=2 "$APP"

# .dmg。fude.app と「アプリケーション」へのリンクを並べたフォルダを固める。
STAGE="$OUT/dmg"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname fude -srcfolder "$STAGE" -format UDZO -ov "$DMG"
if [ "$IDENTITY" != "-" ]; then
    codesign --force --timestamp --keychain "$KEYCHAIN" --sign "$IDENTITY" "$DMG"
fi

# 公証。Developer ID で署名し、資格が 3 つ揃っているときだけ。
if [ "$IDENTITY" != "-" ] && [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ] && [ -n "${APPLE_APP_PASSWORD:-}" ]; then
    echo "公証する"
    xcrun notarytool submit "$DMG" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD" --wait
    xcrun stapler staple "$DMG"
else
    echo "公証しない（Developer ID の証明書か、APPLE_ID、APPLE_TEAM_ID、APPLE_APP_PASSWORD が無い）"
fi

echo "$DMG"
