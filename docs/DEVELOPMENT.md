# 開発と検証

ソースリポジトリでのビルド・検証・配布物の作成方法です。通常の操作は[操作ガイド](GUIDE.md)、機能ごとの対応条件は[機能と対応範囲](FEATURES.md)を参照してください。

ライブラリの失敗は `raytrace-core`・`raytrace-io`・`raytrace-app` の各 `Error` 列挙型と `Result<T>` で返します。`Error` は `#[non_exhaustive]` のため、呼出側のパターンマッチには残りのケースを扱う分岐が必要です。材料番号・波長・光線ID・ファイルパスなどはフィールドで取得でき、表示文は日本語です。ルートクレートからは `raytrace_rs::error::{CoreError, IoError, AppError}` として参照できます。GUIやレポート内のエラー文字列には `error.to_string()` を使います。光源番号の `source_index` は既存の表示に合わせて1から数えます。

## ソースからビルドして起動

WindowsとRust（Cargo）が必要です。初回ビルド時に依存ライブラリを取得します。

```powershell
cargo build --release --locked --workspace --bins
./target/release/raytrace-gui.exe
```

ビルド後は `Raytraceを起動.cmd` からも起動できます。GUIは光源・レンズ・ロッド・スクリーンの組立サンプルで開きます。「ファイル → 開く…」またはドラッグ＆ドロップで `.zmx` やJSONプロジェクトを読み込み、「計算する」で追跡します。

GUIなしで使う場合は `cargo build --release --locked -p raytrace-cli` でビルドします。

```powershell
./target/release/raytrace-rs.exe examples/sequential_lens.json --threads 4 --output results/lens.json
cargo run --release --locked -p raytrace-cli -- examples/sequential_lens.json --threads 4 --output results/lens.json
cargo run --release --example assembly_demo -- results/assembly_demo.json
./target/release/raytrace-rs.exe results/assembly_demo.json --threads 4 --output results/assembly_trace.json
```

## Windows配布ZIPを作成する

配布用ZIPは `python scripts/package_windows.py` で作成できます。ビルド済みの場合は `--no-build` を指定します。実行ファイル・サンプル・操作説明・ライセンスを同梱し、`package.json` に版とSHA-256を記録します。

README、機能一覧、操作ガイド、開発と検証の文書もZIPに同梱します。

```powershell
python scripts/package_windows.py
python scripts/check_portable.py results/packages/raytrace-rs-0.11.0-windows-x64.zip
```

## 自動テストと配布確認

GitHubへのpushとPull Requestで、Windows上の整形・Clippy・全テスト・解析解／ZMX往復保存・配布ZIPの作成と確認を自動実行します。配布確認はWindows標準ディレクトリだけをPATHに残し、4サンプル、ZMX読込、配置編集、保存と元ファイル削除後の再計算を実行します。STEPは変換キャッシュを空にし、外部Pythonの参照先も使用できない状態で、ZMXから初回読込・配置・追跡・保存・再生を確認します。別PCでのGUIの対話操作は別途確認します。

STEPの回帰テストは、自作6形状について従来の変換結果から固定した900本の光線で、交差有無・交点・法線を照合します。曲面の比較許容差は分割精度・入射角・曲率から求め、平面は丸め誤差の範囲で比較します。NURBS半球は独立した球の解析解でも検証します。穴、周期の継ぎ目、極、単位換算、配置、閉じた形状、分割精度の収束と再現性もテスト対象です。

```powershell
cargo test --locked --test native_step
cargo run --release --locked --example step_check -- --generate-owned results/step-owned
```

`step_check INPUT.step OUTPUT.json [REFERENCE_MESH.json] [DEFLECTION_MM] [--optical]` で、キャッシュを使わない変換時間と固定光線の交点・法線を比較できます。`--optical` は分散材料と6面の受光器を使い、光量収支・画素分布・出射方向・光学距離を比較します。光量の差は固定した全入力光線の重みを分母とします。従来版の曲面分割は精度設定で結果が変わるため、精度を変えた従来版同士の差も一緒に確認してください。

## Rust内の検証

波長・角度・表裏別のコーティング収支、13式のAGFと内部透過率、スペクトル、座標変換、有限物体と視野、入射瞳の近軸計算、波長別のRay Aiming、視野の瞳補正、Source Diodeの超Gaussian・非点収差、ミラー、ZMXの参照ファイル復元、2軸・連動の中止と再開、ROIと任意断面を自作データで検査します。

`cargo run --release --example performance_check` は単色／200波長の各131,072光線を1スレッドで測り、準備・光線生成・追跡を分けた時間と、光線生成から追跡までをまとめた時間を、それぞれ5回の中央値としてJSONへ出力します。一括生成と小分け生成のレポートが一致することも確認し、光線の署名・全画素・光量収支を保存します。性能の比較には同じPC・入力・精度を使用してください。

```powershell
cargo test --locked --workspace --all-features
cargo clippy --locked --workspace --all-features --all-targets -- -D warnings
./target/release/raytrace-gui.exe --smoke results/gui_check
./target/release/raytrace-gui.exe --screen --screen-profiles --smoke results/gui_screen
```

テストではSnell則、Fresnel反射、全反射、多重反射、吸収、光学距離、分散、ZMX往復保存を確認します。サンプリングは等面積・等立体角・位置と方向の同時分布、Gaussian分布、seedと光線数変更時の再現性を確認します。

組立はZMXの別読込、材料番号の統合、側面との交差、剛体変換による位置・方向・光学距離の対応、保存・復元、GUIのドラッグ操作を検証します。画面切替・部品選択時のデータ保持、全光源設定時の局所座標の保持も確認します。

スクリーン解析はGaussianの既知の半値幅、複数ピーク、画面端で欠けた分布、受光ゼロ、光量を変えた場合の幅の不変性を検証します。

## OpticStudioとの実機照合

OpticStudioとの光線比較には `examples/exchange_check.rs` と `scripts/zemax_exchange_probe.py` を使えます。入力はダミー平面・レンズ2面・像面からなるZMXです。中心厚を0.25 mm増やして書き出し、同じ7本の光線を比較します。Python側にはpythonnetと有効なOpticStudio APIライセンスが必要で、インストール先は `RAYTRACE_ZEMAX_ROOT` で指定できます。

```powershell
cargo run --release --example exchange_check -- lens.zmx results/exchange
python scripts/zemax_exchange_probe.py --input results/exchange/lens_edited.zmx --rays results/exchange/reference.json --output results/zemax_exchange
```

自作の検証ケースは次のコマンドで生成できます。逐次レンズ、非球面、平行平板、内部多重反射、分散を持つロッド、回転したロッド、逐次／非逐次の折返しミラーの8ケースで、解析解とZMX往復保存を確認します。

```powershell
cargo run --release --example validation_suite -- results/validation
python scripts/zemax_exchange_probe.py --input results/validation/even_asphere/even_asphere.zmx --rays results/validation/even_asphere/reference.json --output results/validation/even_asphere/opticstudio
```

解析解との比較・Rust内のZMX往復保存と、OpticStudio実機との照合は別の検証です。生成したケースの実機照合状態は `not_run` から始まります。Pythonプローブは軸上Standard／Even Asphereの波長別・面別の交点と方向を比較し、許容差を超えると失敗します。Fresnel光量・光学距離・NSC分岐の実機照合は別途必要です。

## リポジトリ構成

Cargo workspaceとして、`crates/` 以下の5クレートに分けています。

- `raytrace-core`: 数学・形状・光源生成・光線追跡・コーティング・分散・スクリーン解析の計算核です。ファイル形式やGUIには依存しません。
- `raytrace-io`: ZMX／AGF／SPCD・STEP・保存・出力を扱います。ZMX入出力と相互に参照する `project`、`assembly`、`record`、`compatibility` もここに置いています。
- `raytrace-app`: パラメーター走査・収束確認・検証ケースのワークフローを扱います。
- `raytrace-cli`: コマンドライン用の `raytrace-rs` 実行ファイルです。
- `raytrace-gui`: デスクトップ用の `raytrace-gui` 実行ファイルです。PNG出力も有効にします。

依存方向は `core ← io ← app ← {cli, gui}` で、各層は必要な下位層を参照します。リポジトリ直下の `raytrace-rs` はcore・io・appのモジュールを再公開する窓口で、既存の `raytrace_rs::model` などのパスを使えます。`tests/` と `examples/` も直下に残しています。

ライブラリのPNG出力には `--features png` を指定します。SPCDの文字列解析は `Spectrum::parse_spcd`、ファイルの読込・文字コード判定は `raytrace_rs::zemax::read_spcd` を使います。

## GUIのスタイル

`crates/raytrace-gui/src/gui/theme.rs` がGUIの色・文字・余白・角丸・影を定義し、
`App::new` から `theme::install` を一度呼びます。DADSのデザイントークン
`@digital-go-jp/design-tokens` 2.1.0に基づくローカルのスタイルガイドです。
GUIではプリミティブ色や数値の文字サイズを直接指定せず、`TEXT`、`BG_PANEL`、
`PRIMARY`、`BORDER_CONTROL` など用途を示すトークンを使います。
データ描画の色とヒートマップは `theme::viz` に分けています。

文字は最小14 px（eguiのpoint、100%表示時）、密な操作部も14 px、説明文は16 px、
見出しは16/18/20 pxです。Noto Sans JPのRegular/BoldとNoto Sans Monoを埋め込み、
egui既定フォントを記号・絵文字のフォールバックとして残しています。
同梱済みフォントの再生成時のみ、未加工の元フォントがあるディレクトリを指定します。

```sh
uv run --no-project --with fonttools python scripts/subset_fonts.py SOURCE_DIR
```

通常の文字は使用背景に対し4.5:1以上、操作境界・アイコン・データの線とマーカーは
3:1以上を `theme.rs` のテストで検証します。無効状態の文字と装飾用グリッド、連続値の
ヒートマップ、重みに応じて半透明に重ねる光路はこの対象から除きます。DADSのred-800とorange-800の文字は白背景でのみ
使います（gray-50や淡い通知背景では4.5:1未満）。orange-600のアイコンも白背景専用で、
灰色パネルではorange-800を使います。淡い通知背景に色付き文字を追加する際は、
使用する組合せをテストに追加してください。

フォーカスは黄色の外周と黒の内周の二重リングです。egui 0.31の描画終了時に
フォーカス中のウィジェットへ重ねて描きます。マウス操作で得たフォーカスにも表示し、
スクロールで一部が隠れた項目では見えている操作領域を囲みます。
OS標準ダイアログのフォーカス表示は対象外です。

デジタル庁デザインシステムウェブサイト https://design.digital.go.jp/dads/ のコンテンツを加工して作成

raytrace-rsはデジタル庁と提携しておらず、デジタル庁の公式製品ではありません。
トークンのMITライセンスとNotoのSIL OFL 1.1はWindows配布物の
`THIRD_PARTY_LICENSES.txt` に収録します。
