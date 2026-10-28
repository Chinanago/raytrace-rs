# JSON操作とMCP

光学系の読込・部品配置・設定変更・計算を、型付きのJSONコマンドで実行できます。GUIの編集と計算にも共通の `raytrace-engine::Session` を使います。これらは現在のソース版をビルドして利用する機能です。

## スクリプトを実行する

```powershell
cargo build --release --locked --workspace --bins
./target/release/raytrace-rs.exe run examples/automation_assembly.json --output results/automation/script_report.json
./target/release/raytrace-gui.exe results/automation/run.json
```

サンプルは円板光源・分散を持つレンズ・ロッド・スクリーンを組み立て、配置と計算記録を保存します。ロッドはサンプルで明示した一定屈折率1.5です。供給されたガラスの分散・吸収を使う場合は、下記のAGF指定を使います。`source_disk.json` は組立へ読み込む光源部品です。

スクリプトの形式は `raytrace-script-v1`、`commands` は実行順のコマンド列です。相対パスの基準はスクリプトを置いたディレクトリです。途中の失敗で実行を終了し、レポートへ0から数える失敗位置とエラーを記録します。各コマンドは独立した操作なので、既に完了した保存・出力は残ります。

複数の編集を一つにまとめるには `transaction` を使います。全編集が成功した時点で反映し、Undoも一回で戻します。メンバーに指定できるのは部品の追加・複製・削除・プロパティ／材料変更です。読込によるプロジェクト置換、保存、計算は個別に実行します。

## 部品を追加し、配置する

光源とレンズは `.zmx` または部品JSONを別々に読み込みます。

```json
{"command":"add_component","kind":"source","path":"source.zmx","alias":"source"}
{"command":"add_component","kind":"optic","path":"lens.zmx","alias":"lens"}
{"command":"set_property","target":"lens","change":{"property":"position_mm","value":[0,0,20]}}
```

`kind` は `source`、`optic`、`system`。追加時の `alias` は後続操作から使う名前です。返り値の `created` には `c1` などの保存可能な部品IDが入ります。aliasはそのSession内で有効で、Undo／Redoと一緒に復元されます。保存したプロジェクトを別Sessionで開いたときは、問い合わせで固定IDを取得します。

部品IDは配置・複製・削除・有効無効切替で追跡します。部品内の面・光源は `c1/s2`、`c1/l1` のように指定します。計算用の配列番号と固定IDの対応は `bindings` で取得できます。ロッドの6面は寸法から生成するため、編集対象はロッド部品の位置・回転・寸法です。

```json
{"command":"add_rod","material":{"type":"catalog","directory":"glass","catalog":"SCHOTT","name":"N-BK7"},"size_mm":[2,2,30],"alias":"rod"}
{"command":"set_rod_material","target":"rod","material":{"type":"existing","name":"材料名"}}
{"command":"add_detector","size_mm":[20,20],"pixels":[64,64],"alias":"screen"}
```

`catalog` はネイティブAGFパーサーで分散と内部吸収を読み込みます。`existing` はプロジェクトの材料名と完全一致で指定します。計算波長と材料の有効範囲は計算時に確認します。

長さはmm、回転はdeg、波長はμmです。部品の位置は配置先の座標系、部品内の面と光源の位置は部品内の局所座標です。回転は既存の `Pose` の規則を使います。GUIの波長欄はnm表示です。

## 状態と操作仕様を取得する

```powershell
./target/release/raytrace-rs.exe commands > commands.schema.json
./target/release/raytrace-rs.exe session
```

`session` は標準入力の1行につき一つのJSONを読み、同じSessionを維持して標準出力へ1行の結果を返します。

```json
{"command":"query","query":{"query":"project"}}
{"command":"query","query":{"query":"components"}}
{"command":"query","query":{"query":"properties","target":"c2"}}
{"command":"query","query":{"query":"bindings"}}
{"command":"query","query":{"query":"materials","target":"c2"}}
{"command":"query","query":{"query":"history"}}
```

`project` は構成の要約、revision、alias、読込上の問題を返します。`components` は部品と子要素のID、`properties` は編集可能な項目と現在値・単位・範囲、`bindings` は計算結果の面／光源／検出器とIDの対応を返します。大きなメッシュやZMX原文を含む全モデルはプロジェクト保存で取得します。

`materials` は指定した部品内の材料名・番号・分散／吸収の定義を返します。`surface_media` の番号はその面が属する部品内の番号です。`properties` の `material_scope` を指定して材料を問い合わせます。`target` を省略した材料の問い合わせは、組立後の光学系を対象にします。

`set_property` の `change` は、項目名と値が型で結び付いています。位置・回転、ロッド寸法、検出器寸法・画素数、光線数・seed・波長・光量・Sobol設定、Source Diodeの分布と非点収差、スペクトル、曲率・円錐定数・媒質、追跡上限などを扱います。使える項目は対象ごとの `properties` とJSON Schemaを参照してください。

スペクトルを持つ光源の単色比較には `target: "project"` の `monochromatic_override_um` を使います。元のスペクトルを保持したまま計算条件を上書きできます。`spectrum` の値を `null` にする操作は、光源自体のスペクトルを解除します。

空の光学系や組立の途中でも配置を編集できます。編集時には項目の型・値・参照を確認し、追跡開始時には材料・形状・光源を含む光学系全体を検証します。

## 計算ジョブ

```json
{"command":"trace","threads":4,"rays_per_source":4096,"paths":64,"output":"run.json","alias":"run"}
{"command":"query","query":{"query":"job","job":"run"}}
{"command":"wait","job":"run","timeout_ms":60000}
{"command":"query","query":{"query":"report","job":"run"}}
{"command":"cancel","job":"run"}
```

計算は開始時のモデルを保持して別スレッドで実行します。`trace` はジョブIDをすぐに返し、`job` で進捗・完了・失敗・中止を取得します。`wait` は最大60秒待ちます。結果には計算時のrevisionとID対応を添え、編集が進んだときは `stale: true` を返します。

進捗の `completed`／`total` は発射光線の処理本数です。走査・収束確認では現在のケースの本数を示し、`progress_scope` に `current_case_rays` を返します。

`rays_per_source` は各光源の本数を指定値に変更して計算します。`preview_limit` は各光源の本数を指定値で上限にします。元のプロジェクトは保持します。合計光線数は200万本、スレッド数は1〜64です。

スクリプトでは `wait` で完了を確認してから終了してください。実行中ジョブを残した終了はエラーになります。Session終了時には実行中ジョブの中止を要求します。

走査・収束確認も同じジョブ操作で扱います。計画JSONの詳細は[操作ガイド](GUIDE.md)を参照してください。走査計画は保存時に部品・面の固定IDへ結び付けます。以前の配列番号を使う計画は、その計画の基準モデルに対して変換します。

```json
{"command":"scan","plan":"scan_plan.json","output":"scan_results","alias":"scan"}
{"command":"convergence","plan":"convergence_plan.json","output":"convergence_results","alias":"convergence"}
```

JSONの収束確認は現在のSessionのモデルを使います。保存済みメッシュのCAD分割精度を変える場合は、元ZMXに対するCLIの `--convergence` を使います。

`report` は全画素や記録した光線を含み、大きくなる場合があります。通常の進捗確認には `job`、再利用には `output` へ保存した計算記録を使います。

同時実行は8ジョブまでです。完了した結果は8件・推定128 MiBを目安に古いものから解放します。再利用する結果は計算記録へ保存してください。

## 開いているGUIを操作する

```powershell
./target/release/raytrace-gui.exe --automation-session results/gui_session.json
./target/release/raytrace-rs.exe session --connect results/gui_session.json
```

GUIは指定されたときにローカル接続を開き、接続先と認証トークンをSessionファイルへ保存します。接続ファイルはGUI終了時に削除します。同じファイルが存在するときは上書きを止めます。

まず `query.project` でrevisionを読み、編集に `expected_revision` を付けます。

```json
{"expected_revision":0,"command":"set_property","target":"c2","change":{"property":"position_mm","value":[0,0,25]}}
```

GUIからも編集が行われ、revisionが一致しなくなった場合は `revision_conflict` を返します。状態を読み直して次の操作を決めます。外部から行った編集もGUIのUndo／Redoで戻せます。計算結果はGUIの光路・スクリーン画面へ反映されます。

## MCPから使う

MCPクライアントのサーバー設定で、実行ファイルと引数を指定します。

```json
{
  "mcpServers": {
    "raytrace": {
      "command": "C:/path/to/raytrace-rs.exe",
      "args": ["mcp"]
    }
  }
}
```

GUIへ接続するときは `args` を `["mcp", "--connect", "C:/path/to/gui_session.json"]` にします。単独起動ではMCPサーバーがモデルを保持し、GUI接続では開いている光学系へ操作を送ります。

ツール名は `raytrace_command`。入力仕様はRustのコマンド定義から生成したJSON Schemaで、CLIの `commands` と共通です。応答にはテキストと構造化された結果・エラーを含みます。標準出力にはMCPのJSON-RPCメッセージだけを流します。

実装はMCPの[stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)、[lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle)、[tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)に沿っています。進捗と中止は `query.job`・`cancel` コマンドで扱います。

## 保存形式と履歴

編集後のプロジェクトは `raytrace-project-v4` にIDを保存します。v1〜v3と従来のシーンJSONも読み込み、編集可能なIDを付与します。計算記録は既存の `raytrace-run-v1` と保存済みの照射条件を維持します。光線追跡内部の配列・倍精度計算・サンプリングは従来と共通です。

Undo／Redoはモデルのスナップショットを使い、連続したGUIドラッグを一つにまとめます。履歴は64件・推定128 MiBを目安に古いものから制限し、一件のモデルが大きい場合も直前の編集を戻せるよう保持します。GUI側の履歴には選択部品などの表示状態だけを持たせます。
