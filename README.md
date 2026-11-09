# raytrace-rs

幾何光学の光線追跡ソフトウェアです。光源・レンズ・インテグレーターロッド・スクリーンで照明系や結像系を組み立て、光線を追跡して、スクリーン上の光量分布と光量収支を求めます。GUI・コマンドライン・AIエージェントのどれからでも、同じ計算を同じ結果で実行できます。

- シーケンシャル追跡と非シーケンシャル追跡（屈折・反射・コーティング・材料分散）
- `.zmx` のレンズデータと `.step` の形状をインポート（`.zmx` への書き出しも可能）
- 光量分布・断面・半値幅・光量収支の解析と、パラメーター走査・収束確認
- 計算記録から同じ条件での再計算（入力と設定をすべて記録に残します）
- コマンドと入出力をJSONスキーマで公開し、MCPサーバーからAIエージェントが操作可能

> **ベータ版（0.12.0）です。** ベータの間は後方互換性がなく、コマンド・JSONの形式・保存ファイルを予告なく変えることがあります。変更点は[変更履歴](CHANGELOG.md)にあります。

[最新版をダウンロード](https://github.com/Chinanago/raytrace-rs/releases/latest) · [AIから使う](docs/AI.md) · [操作ガイド](docs/GUIDE.md) · [機能と対応範囲](docs/FEATURES.md) · [変更履歴](CHANGELOG.md)

## インストール

次の1行でインストールできます（管理者権限は不要です。実行ファイルとサンプルを自分のフォルダーに入れ、AIアプリの設定を表示します）。

```bash
# macOS・Linux
curl -fsSL https://raw.githubusercontent.com/Chinanago/raytrace-rs/main/install.sh | sh
```

```powershell
# Windows（PowerShell）
irm https://raw.githubusercontent.com/Chinanago/raytrace-rs/main/install.ps1 | iex
```

手作業で入れるときは、[リリースページ](https://github.com/Chinanago/raytrace-rs/releases/latest)から使うOSのファイルを取得します。版を含まない固定名のファイルもあり、それぞれに `.sha256`（チェックサム）が付きます。

| OS | ファイル | 内容 |
|---|---|---|
| Windows | `raytrace-rs-windows-x64.zip` | GUI、CLI、MCPサーバー |
| macOS（Apple Silicon・Intel） | `raytrace-rs-macos.tar.gz` | GUI、CLI、MCPサーバー（ユニバーサルバイナリ） |
| Claude Desktop | `raytrace-rs-windows.mcpb` / `raytrace-rs-macos.mcpb` | MCPサーバーの拡張。開くとClaude Desktopに導入できる |
| Linux（CLIのみ） | `raytrace-rs-linux-x64.tar.gz` | 依存ライブラリなしで動く静的リンク版 |

**Windows**：ZIPを展開して「Raytraceを起動.cmd」を開くだけです。ほかに用意するものはありません。起動時にDLL不足と表示されたら、[Visual C++ x64ランタイム](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)を入れてください。

**macOS**：`tar xzf raytrace-rs-macos.tar.gz` で展開し、展開先で `./raytrace-gui`（GUI）または `./raytrace-rs`（CLI）を実行します。署名・公証をしていないため、ブラウザーで取得したファイルは初回に開けないことがあります。その場合は `xattr -dr com.apple.quarantine raytrace-rs-*-macos` で隔離属性を外してください。macOS 11以降が必要です。

**Claude Desktop**：`.mcpb` ファイルをClaude Desktopで開くと導入のダイアログが出ます。Claude Code・Codexでの設定は[AIから使う](docs/AI.md)にあります。

**Linux**：最新版は次の1行で取得できます。

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

配布物の展開先で実行します（macOS・Linuxでは `./raytrace-rs`）。引数はリクエストJSONのフィールドをそのまま書きます（`a.b=文字列`、`a.b:=数値などのJSON`、`a.b:=@ファイル`）。

```powershell
# サンプルを計算して計算記録を保存
./raytrace-rs.exe analysis.trace model.path=examples/sequential_lens.json overrides.rays_per_source:=1024 output=results/lens.json

# ZMXを読み込んで、計算できるか確認（追跡はしない）
./raytrace-rs.exe doc.inspect model.path=lens.zmx

# 保存した計算記録を同じ条件で再計算
./raytrace-rs.exe run.replay record.path=results/lens.json output=results/lens_replay.json
```

`./raytrace-rs.exe --help` でコマンドの一覧、`./raytrace-rs.exe analysis.trace --help` で使えるフィールドと説明が出ます。どちらもコマンドのJSONスキーマから生成しています。

結果は標準出力に1行のJSONで返ります（`"ok"`・`"result"`、失敗時は `"error"`）。読みやすい表示は標準エラーに出ます。同じリクエストをJSONで渡すこともできます。

```bash
echo '{"command":"analysis.trace","model":{"path":"examples/sequential_lens.json"},"output":"results/lens.json"}' | ./raytrace-rs run -
```

`run -` に続けて渡したリクエストは1つのセッションで実行されるため、`doc.open` で開いた光学系を `doc.set` で書き換え、`model` なしで `analysis.trace` し、`session.undo` で戻すこともできます。GUIも同じコマンドで計算しています。入出力の形式は [schemas/](schemas/) にあります。パラメーター走査（`study.scan`）や収束確認（`study.convergence`）は[操作ガイド](docs/GUIDE.md#コマンドライン)にあります。長さはmm、波長はμmです。

## AIから使う

Claude Code・Claude Desktop・Codexから、日本語で頼むだけで光学系を開き、編集し、追跡し、結果を読ませられます（`raytrace-rs mcp` のMCPサーバーと、同梱のAgent Skillを使います）。インストールは上の1行で、各アプリの設定とコピーできる例は[AIから使う](docs/AI.md)にあります。

```text
examples/integrator_rod.json を開いて追跡し、スクリーンの半値幅と光量収支を教えて
```

MCPサーバーの仕様は[操作ガイド](docs/GUIDE.md#aiエージェントmcp)に、このリポジトリを開発するエージェント向けの手順は [AGENTS.md](https://github.com/Chinanago/raytrace-rs/blob/main/AGENTS.md) にあります。

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

[MIT License](LICENSE)。依存ライブラリとフォントにはそれぞれのライセンスが適用され、各配布物の `THIRD_PARTY_LICENSES.txt` に同梱しています。
