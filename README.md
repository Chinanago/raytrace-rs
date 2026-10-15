# Rust 光線追跡アプリ

シーケンシャルと非シーケンシャルを同じ面交差・屈折・反射処理で計算する、Rust製のGUIとライブラリです。レンズ、ミラー、インテグレータロッド、複数の検出器を扱います。v0.2ではZMX/AGFの直接読込と、元のZMX定義を保持する保存を追加しました。2026年10月15日時点の試作です。

## 公開するファイルとローカルデータ

このリポジトリにはソースコード、手書きの一般形状サンプル、合成データを使ったテスト、文書を収録します。Zemaxの実ファイル、AGFなどの材料カタログ、参照CAD・スペクトル、検証画像・光線データ・ログはローカルで管理します。ZMX全文を含む保存プロジェクトやCAD由来のメッシュJSONも公開対象から除外します。

手元の入力データは `local_data/`、出力は `results/` に置いてください。`.gitignore` でこれらのフォルダーと関連形式を除外し、JSONは同梱の4つの手書きサンプルだけを登録対象にしています。テスト内の最小ZMX/AGFレコードは、入出力処理を確認するために作成したものです。

コミットする前に `python scripts/check_publish.py` でステージ済みのファイルを確認できます。この検査は公開ファイルの許可リストと、サンプルJSONへの元プロジェクト混入を確認します。機密情報を含む実データはGit管理外に保ってください。

## GUIを起動する

Windowsで次のようにビルドし、`Raytraceを起動.cmd` または `target/release/raytrace-gui.exe` をダブルクリックします。初回ビルドは依存ライブラリの取得にネット接続が必要です。

```powershell
cargo build --release --locked --features gui --bins
```

日本語のネイティブ画面で、光源・面の配置・波長・材料・追跡方式を編集し、光路と検出器分布を確認できます。ウィンドウへのファイルのドラッグ＆ドロップにも対応します。

1. 「開く…」から `.zmx` または保存済みのプロジェクトを選ぶ。
2. 読込レポートで材料や参照ファイルを確認する。
3. 「計算する」で追跡する。ZMXのプレビューは各光源200本を上限にする。チェックを外すと定義の光線数を使う。
4. 「保存…」で元のZMX全文を含むJSONプロジェクトを保存する。「ZMX書出し…」は対応範囲の編集をZemaxへ戻す際に使う。

左側の「定義の波長」は元の光源設定です。「単色の波長を明示する」が有効な場合は、その波長を計算に使います。材料欄には計算波長での屈折率を表示します。設定変更後は「再計算で更新」と表示し、光路図には最後に計算したモデルを使います。

画面表示と追跡は別のスレッドで実行します。起動用EXEは約6 MBで、GUIは必要なときに描画します。レンズだけの追跡ではPython・Zemaxの起動は不要です。

## ZMXを直接扱う範囲

| ZMX内の定義 | v0.2の扱い |
|---|---|
| シーケンシャル Standard | 平面・球面・円錐面、面間隔、材料、円形/矩形口径を読込 |
| Even Asphere | 曲率、円錐定数、A4～A18を読込。A4はPARM 2に対応 |
| NSC Source Diode | 位置・回転、光量、波長番号、X/Y発散、発光領域、Gaussian幅、ダイオード配列を読込 |
| NSC Detector Rectangle | 位置・回転、寸法、ピクセル数を読込 |
| NSC CAD Import | 参照STEPの閉じたソリッドを読込。三角形BVHで交差を探索 |
| AGF材料 | GCATで指定したカタログのSchott式1・Sellmeier 1式2、LD波長範囲、IT内部透過率を読込 |
| 未対応定義 | 元のレコードを保持し、影響がある項目は追跡前に通知 |

Source DiodeはGaussian係数Gx/Gy/Hx/Hyが1、非点収差0の範囲を実装しています。通常の配列配置を扱い、参照オブジェクトやNSOOの追加オフセットは未対応です。光源の乱数列はRust側で生成し、Zemaxと光線ごとの乱数一致は仮定しません。

シーケンシャルZMXは形状と材料を直接読み、初期照射には主波長の軸上平行光を設定します。視野・瞳・有限距離物体の再現には照射条件の追加が必要です。ZMXの全レコードを物理モデルに対応させた段階ではないため、読込レポートに対応範囲を表示します。

STEPの初回読込にはFreeCADのPythonを使います。`RAYTRACE_FREECAD_PYTHON` で実行ファイルを指定できます。Windowsの既定値は `%LOCALAPPDATA%/Programs/FreeCAD 1.1/bin/python.exe` です。変換処理はEXEに埋め込み、ビルド元フォルダーの `results/cad_cache` に0.02 mm設定のメッシュを保存します。追跡はRustで実行し、2回目以降はキャッシュを使います。メッシュ上の交点と頂点法線補間を使うため、分割を細かくしたときの収束が精度検証の対象です。

コーティングは名前を保持します。未コートFresnelでの計算はGUIの明示設定が必要です。SPCDスペクトルが欠けている場合も通知し、単色の波長を明示するまで追跡を開始しません。ブラックボックスZBB、未対応の面型・光源型・分散式は追跡対象として利用できません。

## 編集とZemaxへの返却

元の光学定義を変更していないZMXは、元の文字コード・レコードを保って書き出します。JSONプロジェクトにも元のZMX全文、材料レコード、読込レポートを保存します。

- 逐次ZMX：軸上の面形状・間隔・口径・材料の対応する編集を反映。元のCOAT、コメント、メリット関数などの追加レコードを保持。
- NSC ZMX：既存オブジェクトの位置・回転と、Source Diodeの光線数・光量・発散・配列・発光領域を反映。
- STEP・ローカルAGFなどの依存ファイル：保存先へ同梱。同名で内容の異なる既存ファイルがある場合は上書きを止める。

NSCの新しいオブジェクト追加、形状・材料・波長表の変更、逐次の座標ブレークを伴う編集は、現在はJSONプロジェクトで保存します。新規の軸上レンズ系はZMXとAGFを書き出せます。独自AGFをOpticStudioで使う際は、[材料カタログのフォルダーへ登録する手順](https://optics.ansys.com/hc/ja/articles/42661706968211-OpticStudio-%E3%81%A7%E6%96%B0%E3%81%97%E3%81%84%E6%9D%90%E6%96%99%E3%81%A8%E3%82%AC%E3%83%A9%E3%82%B9%E3%82%92%E8%BF%BD%E5%8A%A0%E3%81%99%E3%82%8B%E6%96%B9%E6%B3%95)が必要です。Zemaxの標準カタログを書き換える処理は行いません。

## 実装した範囲

| 項目 | 動作 |
|---|---|
| シーケンシャル | 指定順の面を追跡。通常の屈折面では主透過光、ミラーと全反射では反射光を次の指定面へ進める |
| 非シーケンシャル | 前方で最も近い面を選び、Fresnel反射光と透過光を重みで分岐して追跡 |
| 面 | 有限口径の平面、球面キャップ、円錐定数とA4～A18で記述する偶数次非球面、閉じたCADメッシュ |
| 配置 | 各面の3次元平行移動と回転。回転の適用順はRx→Ry→Rz |
| 材料 | 一定屈折率、Sellmeier式、Schott式、波長別表データの線形補間、AGFのITによる波長別内部吸収 |
| 波長 | 光線ごとに真空波長を保持。波長別の屈折率を屈折、全反射、Fresnel反射率、光学距離に適用 |
| 光源 | 一様円板・一様立体角の円錐・GaussianのSource Diode。複数光源・複数波長、保存光線CSV入力 |
| 検出器 | 複数の平面検出器、矩形/円形口径、ピクセル別積算重み |
| 光量 | 指定した発射基準面を100%とするバジェット。幾何学的な外れ、反射、吸収、打切りを別項目に記録 |
| 実行 | CPU並列化、1スレッド実行、同じseedによる光源生成、スレッド数に依存しない集計順序 |

演算はf64です。長さはmm、波長はµm、光線の重みは任意の共通光量単位を使います。光学距離は各媒質内の `n(λ) × 幾何学的距離` の積算です。波動伝搬と干渉は対象外です。

## すぐ試す

このフォルダーで実行します。初回は `cargo fetch --locked` で依存ライブラリを取得し、その後は以下の手順をオフラインで実行できます。検証環境はWindows、Rust 1.98.1です。

```powershell
cargo build --release --offline
./target/release/raytrace-rs.exe examples/sequential_lens.json --threads 4 --repeat 21 --output results/sequential_lens.json
./target/release/raytrace-rs.exe examples/nonsequential_slab.json --threads 4 --output results/nonsequential_slab.json
./target/release/raytrace-rs.exe examples/integrator_rod.json --threads 4 --output results/integrator_rod.json
./target/release/raytrace-rs.exe examples/folded_mirror.json --threads 4 --output results/folded_mirror.json
```

`cargo fetch` は依存ライブラリの初回取得に使います。GUIもビルドする場合は次を実行します。

```powershell
cargo build --release --offline --features gui --bins
```

ZMXを直接読み込むコマンドラインの例です。`--inspect` は定義を読んでプロジェクトに保存し、追跡を実行しません。

```powershell
./target/release/raytrace-rs.exe local_data/lens.zmx --inspect --output results/lens.raytrace.json
./target/release/raytrace-rs.exe local_data/source.zmx --wavelength-um 0.465 --rays-per-source 200 --threads 4 --output results/source.json
```

`--mode sequential` / `--mode non_sequential` で方式を上書きできます。シーケンシャルの面順は物理的な光路順に指定します。非シーケンシャルの例は面を意図的に順不同で登録しています。

`--terminals` は終端光線の位置・方向・重み・波長・光学距離を保存します。`--paths 8` は発射IDが8未満の光線について、分岐を含む光路の線分を保存します。光路表示用の記録は1発射につき256線分までです。

JSON出力には元のscene、基準面の重み、基準面に対する光量割合、面ごとの集計、検出器マップ、処理時間が含まれます。マップはy行/x列の順で、値はピクセル内の積算光量です。照度密度が必要な場合は各ピクセル面積で割ります。

## 材料と波長

各光線の `wavelength_um` を材料の分散式に渡します。レンズとロッドにはそれぞれの材料を割り当てます。同じ光路を450 nmと635 nmで計算する例は `examples/sequential_lens.json` と `examples/integrator_rod.json` にあります。

Sellmeier式は `n² = 1 + Σ Bi λ²/(λ²−Ci)`、Schott式は `n² = A0 + A1 λ² + A2/λ² + A3/λ⁴ + A4/λ⁶ + A5/λ⁸` です。SellmeierのCはµm²、Schottの係数はλをµmで入力したときの値を指定します。表データは `[wavelength_um, n]` を波長の昇順に並べます。

分散式には `valid_range_um` を必須とし、表データは両端の波長を有効範囲にします。範囲外の波長は実行前のエラーになります。現状はscene中の全材料について、発射光線の波長を評価できることを検査します。複数波長の合計光量は各光源の `total_weight` の和で定義し、波長ごとの重みを指定できます。

シリカの例には固定したSellmeier係数をJSONへ記述し、受け付ける範囲は0.4～0.7 µmに絞っています。実際の設計では、ガラス銘柄・係数・適用波長・基準温度をメーカー資料で確認して登録します。AGFのITは厚みd、内部透過率Tから吸収係数 `α = −ln(T)/d` を求め、波長の間をαで線形補間します。吸収データ範囲外も実行前の確認対象です。温度依存は未実装です。

## ロッドと面の向き

`integrator_rod.json` は1 mm角、長さ30 mmのシリカロッドを6面で定義します。側面の法線と材料の組合せから、入射角と波長ごとに全反射を判定します。出口へ進む光と入口へ戻る光をそれぞれ別の検出器で集計します。

屈折面の `minus_medium` は局所法線の負側、`plus_medium` は正側の媒質番号です。法線は局所面の `(-∂sag/∂x, -∂sag/∂y, 1)` の向きです。媒質番号は `materials` 配列の添字に対応します。光線の現在媒質が境界条件と一致しない場合、その光量を `medium_mismatch` に記録します。閉じたレンズやロッドを非シーケンシャルで扱う際は、側面を含む境界を用意します。

## 光量バジェット

基準面はsceneの `reference_plane` に明記し、その面で発射した光線重みの合計を100%とします。各サンプルJSONにも照射を開始する基準面を記述しています。

| 終端・減少項目 | 意味 |
|---|---|
| detected | 全検出器に届いた合計光量。各検出器の割合は `detector_percent_of_reference` に別記 |
| escaped | 追跡対象の面を抜けた光。レンズ出口でsceneを終えた場合は、この項目がレンズの出力になる |
| geometric_miss | シーケンシャルで次の指定面・口径を通過できなかった光 |
| surface_absorbed / bulk_absorbed | 吸収面・ミラーでの吸収と、媒質内部での吸収 |
| sequential_reflection_not_followed | シーケンシャルの主透過光路から分かれた通常の界面反射光 |
| branch_cutoff | 発射時の1光線重みを分母に、指定した閾値より小さくなった分岐光 |
| interaction_limit | 深さまたは1発射あたりの相互作用数の上限に達した光 |
| numerical_failure / medium_mismatch | 面交差の未解決光と、媒質境界の不整合があった光 |

これらの和を発射光量と照合します。打切りと数値計算の未解決分を吸収と区別するため、光量収支が閉じていても、これらの値を確認します。非シーケンシャルの面別値は、同じ面に何度も入射した光を積算するため、100%を超える場合があります。例のロッドには前後両方の検出器があり、`detected` 合計100%には戻り光も含まれます。

## 検証

```powershell
cargo test --release --offline
cargo clippy --all-targets --all-features --offline -- -D warnings
```

物理計算15項目とZMX交換8項目で、Snell則、Fresnel反射、臨界角、平行平板、多重反射、吸収、ロッド、非球面、並列実行の再現性、AGF選択、文字コード、元定義の保持、編集の反映、依存ファイルの競合を確認しています。

GUIの起動・計算・画面保存は `--smoke 出力先` で検証できます。例えば `./target/release/raytrace-gui.exe --smoke results/gui_lens` でサンプルレンズを確認できます。実光源ZMXの検証出力はローカルで管理しています。

実機OpticStudioとの往復検証は `examples/exchange_check.rs` と `scripts/zemax_exchange_probe.py` を使います。ダミー平面・レンズ2面・像面からなるローカルZMXを入力し、レンズの中心厚を0.25 mm増やして書き出します。Python側にはpythonnetと有効なOpticStudio APIライセンスが必要です。インストール先は `RAYTRACE_ZEMAX_ROOT` で指定できます。

```powershell
cargo run --release --example exchange_check -- local_data/lens.zmx results/exchange
python scripts/zemax_exchange_probe.py --input results/exchange/lens_edited.zmx --rays results/exchange/reference.json --output results/zemax_exchange
```

ローカル検証ではACL5040U-Aを使い、633 nm、軸上平行光の代表7本で、像面のX/Y位置と進行方向をOpticStudioと許容値内で照合できました。この試験はレンズの幾何光路を検証する範囲で、コーティングや全NSC機能の一致は追加の検証対象です。入力・編集後のZMX、比較光線、読込結果を含むJSONは公開リポジトリに含めません。

既存の独立NumPy実装とも、ローカルの保存光線を使って位置・方向・重み・通過本数を照合しました。15,955本から両実装とも15,921本がレンズを通過し、座標差は最大1e-9 mm未満でした。この値は両実装の数値的一致を示し、部品寸法や材料データの物理的精度は実物とカタログに基づいて別途評価します。この15,955本を使う条件のOpticStudio直接照合は今後の検証です。専用の検証ヘルパーと元の研究データはローカルで管理しています。

v0.1で同じ15,955本・465 nmを使い、21回の中央値で測定した結果は次のとおりです。時間はレンズ2面の計算で、ファイル読込、JSON/CSV出力、描画、ウォームアップを除きます。Rust側には集計処理を含めています。比較対象は既存NumPy実装で、OpticStudioの実行時間は測定していません。

| 実装 | レンズ2面の計算時間 | NumPy実装に対する速度比 |
|---|---:|---:|
| 既存NumPy | 90.11 ms | 1.00 |
| Rust・1スレッド | 9.66 ms | 9.33 |
| Rust・4スレッド | 3.15 ms | 28.60 |

## 次に拡張する点

1. 部品数が増えた場合の探索を加速する。CAD内部は三角形BVHを使用し、部品間は境界ボックスで候補を除外して全登録面を調べる方式。
2. ZMXの追加面型・光源型・参照オブジェクトと、偏心・傾き・複合系の書出しを拡張する。ブラックボックス部品は面データの提供か参照計算が必要。
3. 円筒面・円錐側面・自由曲面、部品単位の組立編集を追加する。
4. 偏光状態、波長・角度に依存するコーティング、散乱モデルを追加する。現状は各界面で非偏光平均のFresnel係数を使用する。`reflectance` を明示した界面は一定反射率の近似。
5. 光路ビューアーのズーム、3D表示、材料と部品のライブラリを追加する。

現状の偶数次非球面は、口径内の勾配上限から光線との交差が一意と確認できる場合を処理します。一意性の条件を満たさない斜入射や複雑な高次面は `numerical_failure` に集計し、全根の分離に対応する解法を追加する対象としています。位置の自己交差除外は1e-8 mmで、重なった面・同一位置の境界には適切な面間隔が必要です。

計算核は `src/geometry.rs` と `src/trace.rs`、配置・材料・光源は `src/model.rs`、実行と保存は `src/main.rs` に分けています。CPU並列化には[Rayon](https://docs.rs/rayon/latest/rayon/)、sceneと結果の保存には[Serde](https://serde.rs/)を使います。既存Pythonから呼ぶ際は[PyO3](https://pyo3.rs/main/parallelism)の接続層を追加でき、計算核を共有できます。
