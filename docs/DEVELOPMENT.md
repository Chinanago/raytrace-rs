# 開発と検証

ソースリポジトリでのビルド・検証・配布物の作成方法です。通常の操作は[操作ガイド](GUIDE.md)、機能ごとの対応条件は[機能と対応範囲](FEATURES.md)を参照してください。

## ソースからビルドして起動

WindowsとRust（Cargo）が必要です。初回ビルド時に依存ライブラリを取得します。

```powershell
cargo build --release --locked --features gui --bins
./target/release/raytrace-gui.exe
```

ビルド後は `Raytraceを起動.cmd` からも起動できます。GUIは光源・レンズ・ロッド・スクリーンの組立サンプルで開きます。「ファイル → 開く…」またはドラッグ＆ドロップで `.zmx` やJSONプロジェクトを読み込み、「計算する」で追跡します。

GUIなしで使う場合は `cargo build --release --locked` でビルドします。

```powershell
./target/release/raytrace-rs.exe examples/sequential_lens.json --threads 4 --output results/lens.json
cargo run --release --example assembly_demo -- results/assembly_demo.json
./target/release/raytrace-rs.exe results/assembly_demo.json --threads 4 --output results/assembly_trace.json
```

## Windows配布ZIPを作成する

配布用ZIPは `python scripts/package_windows.py` で作成できます。ビルド済みの場合は `--no-build` を指定します。実行ファイル・サンプル・操作説明・ライセンスを同梱し、`package.json` に版とSHA-256を記録します。

README、機能一覧、操作ガイド、開発と検証の文書もZIPに同梱します。

```powershell
python scripts/package_windows.py
python scripts/check_portable.py results/packages/raytrace-rs-0.10.0-windows-x64.zip
```

## 自動テストと配布確認

GitHubへのpushとPull Requestで、Windows上の整形・Clippy・全テスト・解析解／ZMX往復保存・配布ZIPの作成と確認を自動実行します。配布確認はWindows標準ディレクトリだけをPATHに残し、4サンプル、ZMX読込、配置編集、保存と元ファイル削除後の再計算を実行します。GUIの対話操作とFreeCAD初回変換は別途確認します。

## Rust内の検証

波長・角度・表裏別のコーティング収支、13式のAGFと内部透過率、スペクトル、座標変換、有限物体と視野、入射瞳の近軸計算、波長別のRay Aiming、視野の瞳補正、Source Diodeの超Gaussian・非点収差、ミラー、ZMXの参照ファイル復元、2軸・連動の中止と再開、ROIと任意断面を自作データで検査します。

`cargo run --release --example performance_check` は単色／200波長の各131,072光線を1スレッドで測り、準備・光線生成・追跡を分けて5回の中央値、光線の署名、全画素と光量収支をJSONへ出力します。性能の比較には同じPC・入力・精度を使用してください。

```powershell
cargo test --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
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

## コードの構成

計算核は `src/geometry.rs` と `src/trace.rs`、光源生成は `src/model.rs` と `src/sampling.rs`、ZMX交換は `src/zemax.rs`、部品の組立は `src/assembly.rs`、追跡後のスクリーン解析は `src/screen.rs` にあります。Rustライブラリとしても利用できます。
