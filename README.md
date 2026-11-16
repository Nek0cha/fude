# fude

**English summary:** fude is a lightweight, terminal-centric code editor built for working with Claude Code. It is written in Rust with GPUI and runs on Windows 11 and macOS, with first-class support for WSL. Licensed under the Apache License 2.0. The documentation is mainly in Japanese.

## fude とは

fude（筆）は、Claude Code と一緒に使うことを前提にした、ターミナルが中心の軽いエディタです。

- ターミナルを主役にして、ファイルツリー、エディタ、Markdown のプレビュー、全体検索をその脇に置きます。
- Claude Code が返事を待っていると、タブとプロジェクトの印で知らせます。
- Windows 11 と macOS で動きます。Windows では WSL の中のフォルダも、Windows のフォルダと同じように開けます。
- Rust と [GPUI](https://www.gpui.rs/) で書いています。

## 入れ方

[GitHub Releases](https://github.com/Nek0cha/fude/releases) に配布物（Windows のインストーラー、macOS の `.dmg`）を置きます。

### Windows のインストーラー

`fude-<版>-setup.exe` を実行します。管理者の権限は要りません。

- `%LOCALAPPDATA%\Programs\fude` に入り、ユーザーの PATH に足されます。新しく開いたターミナルから `fude <フォルダ>` で開けます。
- スタートメニューに fude が入ります。デスクトップのショートカットは、インストールの途中で選べます。
- 新しい版は、同じようにインストーラーを実行すれば上書きされます。

アンインストールは、Windows の設定の「アプリ」から行います。

- 消えるもの：fude 本体、PATH の項目、ショートカット、fude が登録した Claude Code の hook（この PC と、fude が fude を置いた WSL）、WSL に置いた fude。
- 残るもの：設定（`%APPDATA%\fude\config.toml`）、セッション、最近開いたフォルダの一覧。入れ直すと元に戻ります。
- 入れ直したあとは、Claude Code の hook を設定パネル（Ctrl+,）から登録し直してください。

### macOS の .dmg

Apple Silicon の mac 用です。`fude-<版>-macos-arm64.dmg` を開き、fude を「アプリケーション」に入れます。

- Finder でフォルダやファイルを右クリックし、「このアプリケーションで開く」→ fude で開けます。
- パレット（⌘K）の「シェルコマンドをインストール」を選ぶと、管理者のパスワードを聞かれたあと、ターミナルで `fude <フォルダ>` が使えるようになります（`/usr/local/bin/fude` にリンクを作ります）。
- アンインストールは、先にパレットの「シェルコマンドを削除」を選んでから、「アプリケーション」の fude をゴミ箱に入れます。設定（`~/.config/fude`）は残ります。

### 最初に開くときの警告

配布物には、まだコード署名をしていません（署名の証明書を持っていないため）。そのため、最初に開くときに OS が警告を出します。

- **Windows**：SmartScreen の「Windows によって PC が保護されました」が出たら、「詳細情報」を押してから「実行」を押します。
- **macOS**：「開けません」と出たら「完了」で閉じ、システム設定の「プライバシーとセキュリティ」を開き、下のほうにある fude の「このまま開く」を押してから、もう一度開きます。

## ビルドの仕方

Rust の版は `rust-toolchain.toml` で決めています（rustup が自動で入れます）。

### Windows

fude.exe は、WSL に置く Linux 用の fude（`fude-linux`）を中に埋め込みます。そのため、先に WSL の中で `fude-linux` を作ります。

1. WSL の中で、musl のターゲットを入れます：`rustup target add x86_64-unknown-linux-musl`
2. WSL の中で、リポジトリのルートから `sh scripts/build-linux.sh` を実行します。`target/linux/fude-linux` ができます。
3. Windows で `cargo build --release -p fude` を実行します。

インストーラーを作るときは、3 の代わりに次を行います。

1. 一度だけ、Inno Setup 6 を入れます：`winget install --id JRSoftware.InnoSetup -e`
2. `pwsh -NoProfile -File scripts/build-installer.ps1` を実行します。`dist/fude-<版>-setup.exe` ができます。

署名は `scripts/sign.ps1` が行い、方式は環境変数で選びます。何も設定しなければ署名しません。

| 環境変数 | 意味 |
|---|---|
| `FUDE_SIGN_MODE` | `none`（既定）、`pfx`、`command` |
| `FUDE_SIGN_PFX`、`FUDE_SIGN_PFX_PASSWORD` | `pfx` のときの証明書のファイルとパスワード |
| `FUDE_SIGN_TIMESTAMP_URL` | `pfx` のときの時刻の印のサーバー（既定は `http://timestamp.digicert.com`） |
| `FUDE_SIGN_COMMAND` | `command` のときに実行するコマンド。`{file}` が署名するファイルのパスに置き換わります。署名し終わるまで待つコマンドにします |

配布物（Releases に置くインストーラー）は GitHub Actions が作ります。手順は [docs/releasing.md](docs/releasing.md) にあります。

アプリのアイコン（`assets/icon/fude.ico`）は、`assets/icon/*.svg` を変えたときに `sh scripts/gen-icon.sh` で作り直します（ImageMagick 7 が要ります）。

### macOS

`cargo build --release -p fude` を実行します。

`.app` と `.dmg` を作るときは、`rustup target add aarch64-apple-darwin` を一度だけ行ってから、`bash scripts/build-macos.sh` を実行します。`dist/fude-<版>-macos-arm64.dmg` ができます。

### 依存を変えたとき

依存クレートを足したり上げたりしたら、ライセンスの一覧を作り直します。

1. 一度だけ、cargo-about を入れます：`cargo install cargo-about --locked --features cli`
2. `sh scripts/gen-licenses.sh` を実行します。`licenses/THIRD-PARTY.md` が新しくなります。

## ライセンス

fude は [Apache License 2.0](LICENSE) で配布しています。

同梱している書体（HackGen Console NF）、アイコン、テーマの色のライセンスは `licenses/` と [NOTICE](NOTICE) に、依存クレートのライセンスは `licenses/THIRD-PARTY.md` にあります。アプリの中では、コマンドパレットの「ライセンス」で読めます。

## 商標について

Claude と Claude Code は Anthropic, PBC の商標です。fude は個人のプロジェクトで、Anthropic とは関係がなく、Anthropic が推薦や支援をしているものでもありません。
