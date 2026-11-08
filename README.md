# raytrace-rs

光源・レンズ・インテグレータロッドを並べて、光の通り道とスクリーン上の明るさの分布を計算するソフトです。Zemaxの `.zmx` をそのまま読み込めます。日本語のGUIとコマンドライン（CLI）があります。

[Windows版をダウンロード（v0.11.0）](https://github.com/Chinanago/raytrace-rs/releases/download/v0.11.0/raytrace-rs-0.11.0-windows-x64.zip) · [操作ガイド](docs/GUIDE.md) · [機能と対応範囲](docs/FEATURES.md) · [変更履歴](https://github.com/Chinanago/raytrace-rs/releases)

## インストール

ZIPを展開して「Raytraceを起動.cmd」を開くだけです。RustやOpticStudioは要りません。

起動時にDLL不足と表示されたら、[Visual C++ x64ランタイム](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)を入れてください。

## GUIで使う

1. 起動するとサンプル（光源＋レンズ＋ロッド＋スクリーン）が開きます。
2. **計算する** を押すと、光路図に光線が描かれます。
3. **スクリーン** タブで、明るさの分布・断面・半値幅・光量の内訳を確認します。

自分のデータを使うときは、**ファイル → 新規の光学系** から **＋ 追加** で光源やレンズの `.zmx`、ロッド、スクリーンを足し、ドラッグか数値で配置します。**保存…** でJSONプロジェクトに保存できます。詳しくは[操作ガイド](docs/GUIDE.md)を見てください。

## コマンドラインで使う

ZIPの展開先で実行します。

```powershell
# サンプルを計算して結果をJSONに保存
./raytrace-rs.exe examples/sequential_lens.json --output results/lens.json

# ZMXを読み込んで、計算できるか確認（追跡はしない）
./raytrace-rs.exe lens.zmx --inspect --output results/lens.project.json

# 保存した結果を同じ条件で再計算
./raytrace-rs.exe --replay results/lens.json --output results/lens_replay.json
```

よく使うオプション：

| オプション | 内容 |
|---|---|
| `--rays-per-source N` | 光源ごとの光線数 |
| `--wavelength-um 0.465` | 波長（μm）を1つに固定 |
| `--mode sequential` / `non_sequential` | 追跡方式 |
| `--threads N` | 使うスレッド数 |
| `--json` | 結果の概要を1行のJSONで出力 |

全オプションは `--help`、パラメーター走査（`--scan`）や収束確認（`--convergence`）は[操作ガイド](docs/GUIDE.md#コマンドライン)にあります。長さはmm、CLIの波長はμmです。

## AIエージェントで使う

CodexやClaude Codeに光学計算を任せるための[Agent Skill](https://github.com/Chinanago/raytrace-rs/tree/main/skills/raytrace-rs)を同梱しています。

**Codex**：次のように頼むとインストールされます。使うときは `$raytrace-rs` で呼び出します。

```text
$skill-installer で Chinanago/raytrace-rs の skills/raytrace-rs をインストールして。
```

**Claude Code**：`skills/raytrace-rs` フォルダーを `~/.claude/skills/` にコピーします。「Chinanago/raytrace-rs の skills/raytrace-rs を ~/.claude/skills/ にコピーして」と頼んでも構いません。

CLIに `--json` を付けると、エージェントが結果をそのまま読めます。このリポジトリを開発するエージェント向けの手順は [AGENTS.md](https://github.com/Chinanago/raytrace-rs/blob/main/AGENTS.md) にあります。

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
