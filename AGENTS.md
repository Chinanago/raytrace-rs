# AGENTS.md

Codex・Claude CodeなどのAIエージェント向けの作業ガイドです。利用者向けの説明は [README](README.md) と [docs/](docs/) にあります。

## プロジェクト

Rust製の幾何光学レイトレーサーです。ZMX・JSONの光学系を読み込み、シーケンシャル／非シーケンシャルで光線を追跡し、スクリーンの光量分布と光量収支を出力します。

- 長さはmm、CLIとJSONの波長はμm、GUIの波長入力はnmです。
- Cargo workspaceの6クレートで、依存方向は `core ← formats ← io ← app ← {cli, gui}` です。

| クレート | 役割 |
|---|---|
| `crates/raytrace-core` | 形状・光源・追跡・コーティング・分散・スクリーン解析の計算核 |
| `crates/raytrace-formats` | ZMX／AGF／SPCD・STEPの入出力（プロジェクトモデルに依存しない） |
| `crates/raytrace-io` | `Project`・組立・計算記録・互換性レポート・出力 |
| `crates/raytrace-app` | パラメーター走査・収束確認・検証ケース |
| `crates/raytrace-cli` | CLI `raytrace-rs`（`src/main.rs` 1ファイル） |
| `crates/raytrace-gui` | egui製GUI `raytrace-gui` |
| 直下の `src/lib.rs` | 各クレートを再公開する `raytrace_rs` クレート。`tests/` と `examples/` もここに属する |

## コマンド

LinuxのクラウドサンドボックスではGUIを除く次のセットを使います（CIの `linux-core` と同じ）。

```bash
cargo build --locked -p raytrace-cli
cargo fmt --all --check
cargo clippy --locked -p raytrace-core -p raytrace-formats -p raytrace-io -p raytrace-app -p raytrace-cli -p raytrace-rs --all-features --all-targets -- -D warnings
cargo test --locked -p raytrace-core -p raytrace-formats -p raytrace-io -p raytrace-app -p raytrace-cli -p raytrace-rs --all-features
```

単一テストは `cargo test --locked -p raytrace-rs --test reliability <テスト名>` のように実行します。Windowsでは `--workspace` でGUIも含めて検査します。

CLIの動作確認：

```bash
cargo run --locked -p raytrace-cli -- examples/sequential_lens.json --rays-per-source 256 --output results/lens.json --json
```

`results/` はGit管理外です。生成物はそこに置きます。

## 変更時の決まり

- **CIはRust 1.95.0** です。ローカルのツールチェーンが新しくても、1.95より新しい標準ライブラリAPIは使いません。
- `cargo fmt` と `clippy -D warnings` を通します。workspace lintで `unsafe_code` はdeny、`unused_qualifications` はwarnです。
- `unsafe` は `crates/raytrace-core/src/packet.rs` のAVX2カーネルだけに許可し、各ブロックに `// SAFETY:` を書きます。
- ライブラリのエラーは各クレートの `Error` 列挙型（thiserror、`#[non_exhaustive]`）で返します。エラー文、CLI・GUIの表示文は日本語です。
- 計算核は集計順を固定しています。SIMD経路・BVH・小分け生成は従来経路と**全レポートが一致する**ことをテストで確認しているため、計算核を変えるときはこの一致を崩さないでください。
- 新しいファイルをコミットするときは `scripts/check_publish.py` の許可リストに追加します。対象外は `src/`・`tests/` の `.rs` と `crates/*/src/**/*.rs`・`crates/*/Cargo.toml` だけです。追加しないとCIの「Check publication contents」が失敗します。`git add` 後に `python3 scripts/check_publish.py` で確認できます。
- ZMX・AGF・STEP・CSV・画像・JSONなどは `.gitignore` で除外しています。利用者の光学データや、元のZMXを埋め込むプロジェクト・計算記録JSONはコミットしません。同梱するJSONは `examples/` の自作サンプル4件だけです。
- CLIのオプションを変えたら、`crates/raytrace-cli/src/main.rs` の `HELP`、`docs/GUIDE.md` の「コマンドライン」、`skills/raytrace-rs/references/workflows.md` を合わせて更新します。利用者から見える機能の変更は `docs/FEATURES.md`・`docs/GUIDE.md` にも反映します。
- コミットメッセージは英語の命令形で、変更内容を1行目に書きます（`git log` を参照）。

## 光学計算を実行するとき

CLIやGUIで計算を実行・解釈する手順は [skills/raytrace-rs/SKILL.md](skills/raytrace-rs/SKILL.md) と [references/workflows.md](skills/raytrace-rs/references/workflows.md) にあります。

- CLIは `--json` を付けると標準出力が1行のJSON概要になります。`ok`・`command`・`output`（保存したファイル）・主要な結果を読み、詳細は `output` のファイルを開きます。失敗時は `"ok": false` と `error`、終了コード1です。
- ZMXは追跡の前に `--inspect --json` で読み込み、`traceable` と `compatibility.components[].entries[].status` を確認します。`requires_override`・`unsupported` は追跡を止めます。
- 入力モデルの構造は `examples/*.json`、`crates/raytrace-core/src/model.rs`、`crates/raytrace-io/src/project.rs`、`crates/raytrace-io/src/assembly.rs` が正です。走査・収束確認の計画JSONは `docs/GUIDE.md` にあります。
