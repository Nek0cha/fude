# リリースの手順

配布物は GitHub Actions（`.github/workflows/release.yml`）が作ります。`v*` のタグを push すると、Windows のインストーラーを作って Releases の下書きに置きます。

## リリースする

1. `crates/fude/Cargo.toml` の `version` を上げ、コミットして master に入れます。
2. master の最新で、タグを打って push します。

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

3. Actions の「release」が終わるのを待ちます（Windows の release ビルドに時間がかかります）。
4. Releases に下書きができています。本文の「変更点」を書き足し、内容を確かめてから公開します。

タグの版（先頭の `v` を除いたもの）と `crates/fude/Cargo.toml` の版が違うと、windows のジョブの最初の手順で失敗します。取り違えた版を配らないためです。

## 打ち直す

同じタグで作り直すときは、先に下書きとタグを消します。下書きが残っていると、Releases に上げる手順が失敗します（上書きはしません）。

1. Releases の画面で下書きを消します。
2. タグを消して、打ち直します。

   ```sh
   git push origin :refs/tags/v0.2.0
   git tag -d v0.2.0
   git tag v0.2.0
   git push origin v0.2.0
   ```

## 手で動かす

Actions の画面の「release」から「Run workflow」を押すと、タグを打たずに配布物を作れます。このときは Releases に上げず、インストーラーは実行の画面の成果物（`installer`）からダウンロードします。ワークフローを変えたときの確かめに使います。

## ジョブ

| ジョブ | runner | すること |
|---|---|---|
| `linux` | ubuntu | WSL に置く Linux 用の fude（`fude-linux`）を musl で作る |
| `windows` | windows | `fude-linux` を埋め込んだ fude.exe とインストーラーを作る（`scripts/build-installer.ps1`） |
| `macos` | macos（Apple Silicon） | ビルドが通るかだけを確かめる。失敗してもリリースは止めない。`.dmg` は後で足す |
| `release` | ubuntu | タグのときだけ、インストーラーを Releases の下書きに上げる |

下書きの本文は `.github/release-notes.md` です。

## 署名

署名の secrets が無ければ、署名をせずに配布物を作ります（失敗にはしません）。コード署名の証明書を手に入れたら、リポジトリの Settings → Secrets and variables → Actions に次を足します。

| secret | 中身 |
|---|---|
| `FUDE_SIGN_PFX_BASE64` | 証明書の `.pfx` を base64 にしたもの（`[Convert]::ToBase64String([IO.File]::ReadAllBytes('cert.pfx'))`） |
| `FUDE_SIGN_PFX_PASSWORD` | `.pfx` のパスワード |

`FUDE_SIGN_PFX_BASE64` があると、windows のジョブは `FUDE_SIGN_MODE=pfx` で `scripts/sign.ps1` を使い、fude.exe、アンインストーラー、インストーラーに署名します。パスワードが無いと署名が失敗します（署名を黙って飛ばしません）。

`.pfx` ではなく署名サービス（クラウドの HSM など）を使うときは、windows のジョブの「署名を用意する」の手順で `FUDE_SIGN_MODE=command` と `FUDE_SIGN_COMMAND` を `GITHUB_ENV` に書くように変えます。`FUDE_SIGN_COMMAND` の `{file}` は署名するファイルのパスに置き換わります。署名し終わるまで待つコマンドにしてください。

署名すれば、Releases の本文と README の SmartScreen の案内は要らなくなります。
