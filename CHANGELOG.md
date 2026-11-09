# 変更履歴

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
- **Claude Codeプラグインとスキル**：`/plugin install raytrace-rs --marketplace Chinanago/raytrace-rs` でスキルとMCPサーバーを導入できます。CodexにもAgent Skillを使えます。

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
