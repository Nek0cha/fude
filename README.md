# fude

**English summary:** fude is a lightweight, terminal-centric code editor built for working with Claude Code. It is written in Rust with GPUI and runs on Windows 11 and macOS, with first-class support for WSL. Licensed under the Apache License 2.0. The documentation is mainly in Japanese.

## fude とは

fude（筆）は、Claude Code と一緒に使うことを前提にした、ターミナルが中心の軽いエディタです。

- ターミナルを主役にして、ファイルツリー、エディタ、Markdown のプレビュー、全体検索をその脇に置きます。
- Claude Code が返事を待っていると、タブとプロジェクトの印で知らせます。
- Windows 11 と macOS で動きます。Windows では WSL の中のフォルダも、Windows のフォルダと同じように開けます。
- Rust と [GPUI](https://www.gpui.rs/) で書いています。

## 入れ方

GitHub Releases に配布物（Windows のインストーラー、macOS の `.dmg`）を置く予定です（準備中）。

### 最初に開くときの警告

配布物には、まだコード署名をしていません（署名の証明書を持っていないため）。そのため、最初に開くときに OS が警告を出します。

- **Windows**：SmartScreen の「Windows によって PC が保護されました」が出たら、「詳細情報」を押してから「実行」を押します。
- **macOS**：「開発元を確認できないため開けません」が出たら、システム設定の「プライバシーとセキュリティ」を開き、下のほうにある fude の「このまま開く」を押します。

## ビルドの仕方

Rust の版は `rust-toolchain.toml` で決めています（rustup が自動で入れます）。

### Windows

fude.exe は、WSL に置く Linux 用の fude（`fude-linux`）を中に埋め込みます。そのため、先に WSL の中で `fude-linux` を作ります。

1. WSL の中で、musl のターゲットを入れます：`rustup target add x86_64-unknown-linux-musl`
2. WSL の中で、リポジトリのルートから `sh scripts/build-linux.sh` を実行します。`target/linux/fude-linux` ができます。
3. Windows で `cargo build --release -p fude` を実行します。

### macOS

`cargo build --release -p fude` を実行します。

### 依存を変えたとき

依存クレートを足したり上げたりしたら、ライセンスの一覧を作り直します。

1. 一度だけ、cargo-about を入れます：`cargo install cargo-about --locked --features cli`
2. `sh scripts/gen-licenses.sh` を実行します。`licenses/THIRD-PARTY.md` が新しくなります。

## ライセンス

fude は [Apache License 2.0](LICENSE) で配布しています。

同梱している書体（HackGen Console NF）、アイコン、テーマの色のライセンスは `licenses/` と [NOTICE](NOTICE) に、依存クレートのライセンスは `licenses/THIRD-PARTY.md` にあります。アプリの中では、コマンドパレットの「ライセンス」で読めます。

## 商標について

Claude と Claude Code は Anthropic, PBC の商標です。fude は個人のプロジェクトで、Anthropic とは関係がなく、Anthropic が推薦や支援をしているものでもありません。
