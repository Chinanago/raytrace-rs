# raytrace-rs

光源・レンズ・インテグレータロッドを並べて、光の通り道とスクリーン上の明るさの分布を計算するソフトです。Zemaxの `.zmx` をそのまま読み込めます。日本語のGUIとコマンドライン（CLI）があります。

[Windows版をダウンロード（v0.11.0）](https://github.com/Chinanago/raytrace-rs/releases/download/v0.11.0/raytrace-rs-0.11.0-windows-x64.zip) · [操作ガイド](docs/GUIDE.md) · [機能と対応範囲](docs/FEATURES.md) · [変更履歴](https://github.com/Chinanago/raytrace-rs/releases)

## インストール

**Windows**：ZIPを展開して「Raytraceを起動.cmd」を開くだけです。RustやOpticStudioは要りません。起動時にDLL不足と表示されたら、[Visual C++ x64ランタイム](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)を入れてください。

**Linux（CLIのみ）**：v0.11.0より後のリリースには、依存ライブラリなしで動く `raytrace-rs-<版>-linux-x64.tar.gz` を添付します。最新版は次の1行で取得できます。

```bash
curl -L https://github.com/Chinanago/raytrace-rs/releases/latest/download/raytrace-rs-linux-x64.tar.gz | tar xz
```

Rustがあれば、どのOSでも `cargo install --git https://github.com/Chinanago/raytrace-rs --locked raytrace-cli` でCLIを入れられます。

## GUIで使う

1. 起動するとサンプル（光源＋レンズ＋ロッド＋スクリーン）が開きます。
2. **計算する** を押すと、光路図に光線が描かれます。
3. **スクリーン** タブで、明るさの分布・断面・半値幅・光量の内訳を確認します。

自分のデータを使うときは、**ファイル → 新規の光学系** から **＋ 追加** で光源やレンズの `.zmx`、ロッド、スクリーンを足し、ドラッグか数値で配置します。**保存…** で設計JSONに保存できます。詳しくは[操作ガイド](docs/GUIDE.md)を見てください。

## コマンドラインで使う

ZIPの展開先で実行します（Linuxでは `./raytrace-rs`）。引数はリクエストJSONのフィールドをそのまま書きます（`a.b=文字列`、`a.b:=数値などのJSON`、`a.b:=@ファイル`）。

```powershell
# サンプルを計算して計算記録を保存
./raytrace-rs.exe analysis.trace model.path=examples/sequential_lens.json overrides.rays_per_source:=1024 output=results/lens.json

# ZMXを読み込んで、計算できるか確認（追跡はしない）
./raytrace-rs.exe doc.inspect model.path=lens.zmx

# 保存した計算記録を同じ条件で再計算
./raytrace-rs.exe run.replay record.path=results/lens.json output=results/lens_replay.json
```

`./raytrace-rs.exe --help` でコマンドの一覧、`./raytrace-rs.exe analysis.trace --help` で使えるフィールドと説明が出ます。どちらもコマンドのJSONスキーマから生成しているので、CLIに独自のオプションはありません。コマンドはドット区切りのIDだけで呼べます（以前の短い名前は使えません）。

結果は標準出力に1行のJSONで返ります（`"ok"`・`"result"`、失敗時は `"error"`）。読みやすい表示は標準エラーに出ます。同じリクエストをJSONで渡すこともできます。

```bash
echo '{"command":"analysis.trace","model":{"path":"examples/sequential_lens.json"},"output":"results/lens.json"}' | ./raytrace-rs run -
```

`run -` に続けて渡したリクエストは1つのセッションで実行されるため、`doc.open` で開いた光学系を `doc.set` で書き換え、`model` なしで `analysis.trace` し、`session.undo` で戻すこともできます。GUIも同じコマンドで計算しています。入出力の形式は [schemas/](schemas/) にあります。パラメーター走査（`study.scan`）や収束確認（`study.convergence`）は[操作ガイド](docs/GUIDE.md#コマンドライン)にあります。長さはmm、波長はμmです。

## AIエージェントで使う

CodexやClaude Codeに光学計算を任せるための[Agent Skill](https://github.com/Chinanago/raytrace-rs/tree/main/skills/raytrace-rs)を同梱しています。

**Codex**：次のように頼むとインストールされます。使うときは `$raytrace-rs` で呼び出します。

```text
$skill-installer で Chinanago/raytrace-rs の skills/raytrace-rs をインストールして。
```

**Claude Code**：プラグインとしてインストールします。

```text
/plugin install raytrace-rs --marketplace Chinanago/raytrace-rs
```

古い版のClaude Codeでは `/plugin marketplace add Chinanago/raytrace-rs` の後に `/plugin install raytrace-rs@raytrace-rs` を実行します。

**MCPサーバー**：`raytrace-rs mcp` が、セッションを保ったままコマンドを呼べるMCPサーバーを標準入出力で起動します（ツールは `list_commands`・`describe_command`・`execute`・`batch`）。`claude mcp add raytrace-rs -- raytrace-rs mcp` で登録でき、Claude Codeのプラグインはリポジトリ直下の `.mcp.json` で同じサーバーを起動します。どちらも `raytrace-rs` がPATHにあることが前提です。Codexの設定例は [操作ガイド](docs/GUIDE.md#aiエージェントmcp) にあります。

エージェントはCLIのJSON出力と [schemas/](https://github.com/Chinanago/raytrace-rs/tree/main/schemas) を読んで、モデルの作成から結果の解釈まで行えます。このリポジトリを開発するエージェント向けの手順は [AGENTS.md](https://github.com/Chinanago/raytrace-rs/blob/main/AGENTS.md) にあります。

## できないこと

幾何光学の光線追跡だけを扱います。偏光・位相、回折・干渉、MTF/PSF、自動最適化、公差解析はできません。対応するZMXの面・光源・STEP形状は[機能と対応範囲](docs/FEATURES.md)を見てください。

## ソースからビルドする

Rust 1.95以降が必要です。

```bash
cargo build --release --locked --workspace --bins   # GUIとCLI
cargo build --release --locked -p raytrace-cli      # CLIだけ
```

テスト・検証・配布物の作成は[開発と検証](docs/DEVELOPMENT.md)にあります。

## ライセンス

[MIT License](LICENSE)。依存ライブラリとフォントにはそれぞれのライセンスが適用され、Windows版ZIPの `THIRD_PARTY_LICENSES.txt` に同梱しています。
