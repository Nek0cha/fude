# リリースの手順

配布物は GitHub Actions（`.github/workflows/release.yml`）が作ります。`v*` のタグを push すると、Windows のインストーラーと macOS の `.dmg` を作って Releases の下書きに置きます。

## リリースする

1. `crates/fude/Cargo.toml` の `version` を上げ、コミットして master に入れます。
2. master の最新で、タグを打って push します。

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

3. Actions の「release」が終わるのを待ちます（release ビルドに 10〜20 分ほどかかります）。
4. Releases に下書きができています。本文の「変更点」を書き足し、内容を確かめてから公開します。

タグの版（先頭の `v` を除いたもの）と `crates/fude/Cargo.toml` の版が違うと、windows と macos のジョブの最初の手順で失敗します。取り違えた版を配らないためです。

## 打ち直す

同じタグで作り直すときは、先に下書きとタグを消します。同じタグのリリースか下書きが残っていると、release のジョブは下書きを作る前に失敗します（下書きが重ならないようにするためです）。

1. Releases の画面で下書きを消します。
2. タグを消して、打ち直します。

   ```sh
   git push origin :refs/tags/v0.2.0
   git tag -d v0.2.0
   git tag v0.2.0
   git push origin v0.2.0
   ```

## 手で動かす

Actions の画面の「release」から「Run workflow」を押すと、タグを打たずに配布物を作れます。このときは Releases に上げず、インストーラーと `.dmg` は実行の画面の成果物（`installer`、`dmg`）からダウンロードします。ワークフローを変えたときの確かめに使います。

## ジョブ

| ジョブ | runner | すること |
|---|---|---|
| `linux` | ubuntu | WSL に置く Linux 用の fude（`fude-linux`）を musl で作る |
| `windows` | windows | `fude-linux` を埋め込んだ fude.exe とインストーラーを作る（`scripts/build-installer.ps1`） |
| `macos` | macos（Apple Silicon） | fude.app を組んで署名し、`.dmg` にする（`scripts/build-macos.sh`） |
| `release` | ubuntu | タグのときだけ、windows と macos の両方が成功したら、インストーラーと `.dmg` を Releases の下書きに上げる |

下書きの本文は `.github/release-notes.md` です。中の `<版>` と `<version>` は、release のジョブがタグの版（例：`0.2.0`）に置き換えます。

## 署名（Windows）

署名の secrets が無ければ、署名をせずに配布物を作ります（失敗にはしません）。コード署名の証明書を手に入れたら、リポジトリの Settings → Secrets and variables → Actions に次を足します。

| secret | 中身 |
|---|---|
| `FUDE_SIGN_PFX_BASE64` | 証明書の `.pfx` を base64 にしたもの（`[Convert]::ToBase64String([IO.File]::ReadAllBytes('cert.pfx'))`） |
| `FUDE_SIGN_PFX_PASSWORD` | `.pfx` のパスワード |

`FUDE_SIGN_PFX_BASE64` があると、windows のジョブは `FUDE_SIGN_MODE=pfx` で `scripts/sign.ps1` を使い、fude.exe、アンインストーラー、インストーラーに署名します。パスワードが無いと署名が失敗します（署名を黙って飛ばしません）。

`.pfx` ではなく署名サービス（クラウドの HSM など）を使うときは、windows のジョブの「署名を用意する」の手順で `FUDE_SIGN_MODE=command` と `FUDE_SIGN_COMMAND` を `GITHUB_ENV` に書くように変えます。`FUDE_SIGN_COMMAND` の `{file}` は署名するファイルのパスに置き換わります。署名し終わるまで待つコマンドにしてください。

署名すれば、Releases の本文と README の SmartScreen の案内は要らなくなります。

## 署名と公証（macOS）

secrets が無ければ、ad-hoc 署名（`codesign --sign -`）にして公証しません（失敗にはしません）。Apple Developer Program に入って Developer ID の証明書を手に入れたら、次を足します。

| secret | 中身 |
|---|---|
| `MACOS_CERT_P12_BASE64` | Developer ID Application の証明書を書き出した `.p12` を base64 にしたもの（`base64 -i cert.p12`） |
| `MACOS_CERT_PASSWORD` | `.p12` のパスワード |
| `APPLE_ID` | 公証に使う Apple ID |
| `APPLE_TEAM_ID` | チーム ID |
| `APPLE_APP_PASSWORD` | Apple ID の App 用パスワード |

証明書があると、`scripts/build-macos.sh` は一時的なキーチェーンに証明書を入れ、hardened runtime を付けて fude.app と `.dmg` に署名します。さらに公証の 3 つが揃っていれば、`xcrun notarytool` で公証してから `stapler staple` します。

**この経路は、証明書が無いためまだ一度も試していません。** 初めて使うときは、公証で entitlements が足りないと言われるおそれがあります。

署名と公証をすれば、Releases の本文と README の Gatekeeper の案内は要らなくなります。

手元の mac で作るときは、リポジトリのルートで `bash scripts/build-macos.sh` を実行します（`rustup target add aarch64-apple-darwin` が要ります）。
