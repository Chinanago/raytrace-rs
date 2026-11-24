# 変更履歴

## 0.13.0（ベータ）

GUIを開いたまま、CLIやMCPから同じ光学系を編集・計算できるようになりました。Windows・macOSのGUI/CLI配布物、Claude Desktop用MCPB、Linux版CLIを同じ版で提供します。

### 主な変更

- **開いているGUIへ接続**：`raytrace-gui --automation-session 接続ファイル` で起動し、CLIの `--connect 接続ファイル`、MCPの `mcp --connect 接続ファイル` から操作できます。文書・Undo/Redo・保存状態・直近の計算結果を共有します。
- **並行編集の競合検出**：外部の変更には `expected_revision` が必要です。読込・計算は作業スレッドで準備し、その間にGUIで編集・保存・Undo/Redoが行われた場合は古い状態の確定を拒否します。中止・走査の進捗にも対応します。
- **計算記録と表示用光路を分離**：表示用に間引いた光路は `viewer_paths` へ返し、保存する `report.paths` は実際の発射光線の記録を維持します。表示サンプルを要求した場合も、保存レポートを同じ条件で完全に再計算できます。
- **結果の鮮度判定を統一**：GUIと外部解析が計算時の文書の同一性で `stale` を判定します。Undo/Redoで同じ文書に戻ると結果も最新へ戻り、文書の版番号自体は増え続けます。
- **堅牢性の検査を強化**：製品コードの `unreachable`・assert系マクロを機械で検査し、不変条件の違反をエラーへ変更しました。ZMX・AGF・STEPのファジングをCIに追加しました。
- **配布の改善**：Linuxのライセンス情報生成に必要な全依存を取得します。新規リリースは各OSの配布・MCPBの検証とアップロードを終えてから公開します。
- **設計原則P12**：アプリは速く・決定的な評価を提供し、最適化・公差解析などの戦略はエージェント側へ置く方針を明文化しました。

### GUIへ接続する

自分だけが使える既存フォルダーに、まだ存在しない接続ファイルの絶対パスを指定します。

```text
raytrace-gui --automation-session /absolute/private/gui-session.json
raytrace-rs mcp --connect /absolute/private/gui-session.json
raytrace-rs --connect /absolute/private/gui-session.json session.status
```

MCPクライアント設定の `args` は `["mcp", "--connect", "接続ファイルの絶対パス"]` です。GUIとクライアントは同じv0.13.0を使います。まず `session.status` で版を読み、文書を変える操作に `expected_revision` を渡してください。接続ファイルには認証トークンが入るため共有せず、入出力には絶対パスを使ってください。詳細は[GUIへの接続](https://github.com/Chinanago/raytrace-rs/blob/v0.13.0/docs/GUIDE.md#開いているguiへ接続する)にあります。

### ベータの互換性

ベータ期間は後方互換性を保証しません。`analysis.trace` の表示用サンプルと `run.open viewer_paths:=true` の表示用光路は、`record.report.paths` と分けて応答の `viewer_paths` で受け取ります。以前の版で保存した計算記録を新しい版で再計算する互換は保証せず、必要に応じて元のモデルから計算し直してください。

## 0.12.0（ベータ）

v0.11.0からの主な変更です。ベータの間は後方互換性を保ちません。前の版のファイルや呼び出しが動かなくなることがあります。

### 新機能

- **JSONエンジンとコマンドID**：計算、読込、保存はすべて `<領域>.<動作>` 形式のIDを持つコマンド（`analysis.trace`・`doc.open`・`run.replay`・`study.scan` など）として呼びます。CLI、GUI、MCPが同じコマンドを使い、入出力の形式は `schemas/` のJSONスキーマで公開しています。
- **スキーマ駆動のCLI**：`raytrace-rs COMMAND a.b=文字列 a.b:=JSON` の形で呼びます。コマンドごとのオプションはありません。`--help`、`command.list`、`command.describe` でコマンドと引数の説明を確認できます。標準出力はリクエストごとに1行のJSONです。
- **`run -` のセッション**：標準入力に続けたリクエストを1つのセッションで実行します。`doc.open` で開いた文書は、`model` を省略したコマンドの対象になります。
- **設計文書 `raytrace-design-v1`**：部品（`Part`）と配置（`Occurrence`）、安定したIDで光学系を表します。`doc.*` と `component.*` のコマンドで追加、複製、削除、配置の編集ができます。ZMXの互換性レポートも設計文書に付きます。
- **セッション、取り消し、変更履歴**：`doc.set` で文書の一部を書き換え、`session.undo` で戻せます。`group` で連続した編集を取り消しの1単位にまとめ、`session.log` で確定した変更を確認できます。GUIの編集も同じ経路です。
- **MCPサーバー**：`raytrace-rs mcp` が標準入出力でMCPサーバーを起動します。ツールは `list_commands`・`describe_command`・`execute`・`batch` の4つで、コマンドを足しても自動で `execute` から使えます。進捗通知と中止にも対応しています。
- **解析コマンド**：計算記録の後処理を `analysis.budget`（光量収支）と `analysis.screen`（スクリーンの分布、半値幅、断面、ROI）として呼べます。GUIのスクリーンタブも同じ解析の結果を表示します。
- **Linux・macOSの配布物**：依存ライブラリなしで動く静的リンクのLinux版CLI、CLIとGUIを含むmacOS版（Apple SiliconとIntelの両対応）を用意しました。リリースの資産には、版なしの固定名（`raytrace-rs-windows-x64.zip`・`raytrace-rs-macos.tar.gz`・`raytrace-rs-linux-x64.tar.gz`）も添付します。
- **Claude Desktop拡張（`.mcpb`）**：WindowsとmacOSのMCPB拡張を用意しました。ファイルを開くだけで、Claude Desktopから `raytrace-rs` のMCPサーバーを使えます。
- **インストールスクリプト**：`install.sh`（macOS・Linux）と `install.ps1`（Windows）が、最新版（または `RAYTRACE_VERSION`・`-Version` で指定した版）を取得して `.sha256` を照合し、管理者権限なしでユーザーのフォルダーに入れます。最後に、Claude Code・Claude Desktop・Codexの設定を絶対パスで表示します。`--uninstall`・`-Uninstall` で削除できます。リリースの全資産に `.sha256` を添付します。
- **AIから使う手引き**：Claude Code・Claude Desktop・Codexの設定、最初の頼み方、困ったときの対処を[docs/AI.md](docs/AI.md)にまとめました。
- **Claude Codeプラグインとスキル**：`/plugin install raytrace-rs --marketplace Chinanago/raytrace-rs` でスキルとMCPサーバーを導入できます。CodexにもAgent Skillを使えます。スキルは0.12向けに書き直し、MCPを優先して、`doc.open`・`doc.inspect`・`analysis.trace`・`analysis.budget`・`analysis.screen` の流れと結果の読み方を示します。

### 改善

- 追跡の高速化（SoAとAVX2のSIMD経路、面のBVH、光線の小分け生成）。集計順は固定で、従来経路と全レポートが一致します。
- 損失の集計、平面ベースの偶数次非球面、経路選択の不具合を修正しました。
- GUIをDADSベースの配色と同梱のNotoフォントに刷新しました。エラーはパニックせず画面に表示します。
- 製品コードで `unwrap`・`expect`・`panic` を使わず、エラーは日本語のメッセージで返します。

### 破壊的変更

- **CLIにオプションはありません**：`--inspect`・`--replay` などは使えません。`COMMAND a.b=値` の形で呼びます。
- **コマンドはドット区切りのIDだけ**：以前の短い名前（`trace`・`inspect`・`replay`・`scan`・`resume_scan`・`convergence`・`open`・`save_design`・`save_run`・`export_zmx`・`export_screen`・`schema`・`version`）は使えません。
- **古いファイルは読み込めません**：旧プロジェクト形式（`raytrace-project-v1`〜`v3`）、旧版の計算記録、旧版の走査記録（`raytrace-scan-v1`）は読めません。設計文書に作り直し、計算と走査をやり直してください。
- **走査のパラメーターは配置のIDで指定します**：`component_position`・`component_rotation`・`rod_size` は `occurrence` にIDを書きます。
- **CLIの標準出力はJSONです**：リクエストごとに1行の `{ok, command, revision, result | error}` を返し、失敗時の終了コードは1です。人向けの表示は標準エラーに出ます。

### ベータについて

0.12.0はベータ版です。ベータの間は、コマンド、JSONの形式、保存ファイルを予告なく変えることがあり、後方互換性はありません。不具合や要望は [Issues](https://github.com/Chinanago/raytrace-rs/issues) へお寄せください。
