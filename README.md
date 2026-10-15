# raytrace-rs

Rust製の幾何光学シミュレーターです。シーケンシャル／非シーケンシャル光線追跡、波長依存の屈折・反射・吸収を扱います。日本語GUIとコマンドラインから、レンズ・ミラー・インテグレータロッドの光路と検出器分布を確認できます。

## ビルドと起動

WindowsとRust（Cargo）が必要です。初回ビルド時に依存ライブラリを取得します。

```powershell
cargo build --release --locked --features gui --bins
./target/release/raytrace-gui.exe
```

ビルド後は `Raytraceを起動.cmd` からも起動できます。GUIは光源・レンズ・ロッド・検出器の組立サンプルで開きます。「開く…」またはドラッグ＆ドロップで `.zmx` やJSONプロジェクトを読み込み、「計算する」で追跡します。

光源の本数・波長・サンプリング、面の位置・傾き・曲率・材料を編集できます。光路図はZ–X／Z–Y／X–Y投影、検出器は画素ごとの積算光量を表示します。設定を変えると再計算が必要です。

## 光源・レンズ・ロッドを組み合わせる

1. 「新規」で空の光学系を作ります。
2. 「部品を追加 → 光源ZMX…」で光源を読み込みます。光源内のレンズやガラス形状も部品に含まれます。
3. 「部品を追加 → レンズZMX…」でレンズを読み込みます。レンズの試験光源・像面を除き、光学面と材料を追加します。
4. 「部品を追加 → ロッド」でロッドを追加し、幅X・幅Y・長さと材料を設定します。
5. 「部品を追加 → 検出器」で観測面を追加し、配置して「計算する」を押します。

図の丸印をドラッグして、選んだ投影の2軸方向へ部品を移動できます。「部品と配置」のX/Y/Z・θx/θy/θzでは位置と傾きを数値指定します。配置の基準は、光源が発光配列の中心、レンズが最初の光学面の頂点、ロッドが入口中心です。部品一覧のチェックで有効／無効を切り替え、「削除」で取り除けます。

組立光学系は非シーケンシャルで追跡します。各部品の内部形状・波長依存の材料を維持し、全体で材料番号を割り当てます。円形口径の軸上逐次レンズには、各面の口径端を結ぶ円筒／円錐側面を追加します。側面形状はこの幾何近似に基づくため、実部品の有効径と形状に合わせて確認してください。

「保存…」は部品の元定義・配置・材料・ロッド寸法を含むJSONを保存します。「開く…」で、元のZMXやSTEPを開き直さずに復元できます。組立光学系のZMX書出しは未対応です。

組立サンプルは「サンプルを開く → 光源＋レンズ＋ロッド」からも開けます。CLI用のプロジェクトは次のように生成できます。

```powershell
cargo run --release --example assembly_demo -- results/assembly_demo.json
./target/release/raytrace-rs.exe results/assembly_demo.json --threads 4 --output results/assembly_trace.json
```

## スクリーンの表示

検出器マップは「ピーク = 1」と「基準光量比 [%]」を切り替えられます。基準光量比は発射基準面の光量に対する各画素の積算光量です。色目盛は線形／平方根から選びます。表示の変更は再追跡を必要とせず、元の光量と光量収支を保持します。

「半値幅」で、最大画素を通るX・Y断面の半値交点を赤い破線で示し、FWHMをmmで表示します。「断面…」で両方向のプロファイルを開けます。座標はスクリーンの局所座標です。

半値交点は画素中心の値を線形補間して求めます。断面に複数の山がある場合は、最大ピークの両側で最初に半値へ達する位置を採ります。同じ最大値が複数ある場合は、画素配列の先頭（Yの小さい行、Xの小さい列）を選びます。交点が両側に揃わない方向は「半値交点不足」、受光ゼロは「受光なし」と表示します。幅の解釈には表示された画素寸法を併せて確認してください。

## コマンドライン

GUIなしで使う場合は `cargo build --release --locked` でビルドします。

```powershell
./target/release/raytrace-rs.exe examples/sequential_lens.json --threads 4 --output results/lens.json
./target/release/raytrace-rs.exe examples/integrator_rod.json --threads 4 --output results/rod.json
./target/release/raytrace-rs.exe lens.zmx --inspect --output results/lens.raytrace.json
./target/release/raytrace-rs.exe source.zmx --wavelength-um 0.465 --rays-per-source 4096 --sampling sobol --output results/source.json
```

| オプション | 用途 |
|---|---|
| `--mode sequential` / `non_sequential` | 追跡方式を上書き |
| `--sampling sobol` / `pseudorandom` | 全光源のサンプリング方式を上書き |
| `--rays-per-source N` | 各光源の本数を上書き |
| `--rays-csv INPUT.csv` | 発射位置・方向・重み・波長を指定した光線を入力 |
| `--terminals` / `--paths N` | 終端光線／発射IDがN未満の光路を保存 |
| `--repeat N` | 同じ光線をN回追跡し、処理時間の中央値を出力 |
| `--inspect` | ZMXを読み込み、追跡せずにプロジェクトへ保存 |

処理時間は追跡と集計を含みます。光源生成、ファイル入出力、描画、ウォームアップは計測から除きます。詳細は `--help` で確認できます。

## サンプル

| ファイル | 内容 |
|---|---|
| [sequential_lens.json](examples/sequential_lens.json) | 450 nmと635 nmの光を両凸レンズへ入射 |
| [nonsequential_slab.json](examples/nonsequential_slab.json) | 平行平板の透過と多重反射 |
| [integrator_rod.json](examples/integrator_rod.json) | 1 mm角・長さ30 mmのロッドと全反射 |
| [folded_mirror.json](examples/folded_mirror.json) | ミラーで光路を90°曲げる系 |

## 光源サンプリング

既定は、領域内の偏りを抑える低偏差列の **Sobol列** です。seedで決まる線形行列スクランブルとデジタルシフトを加え、発光位置・出射方向・ダイオード配列の選択に別々の次元を使います。同じ設定とseedなら、同じ光線を生成します。

一様円板は面積、一様円錐は立体角に対して一様になるよう変換します。GaussianのSource Diodeには逆累積分布関数を使い、発光領域の端で切ったGaussian分布を保ちます。

光源ごとの本数を **256、1024、4096など2のべき乗** にすると、Sobol列の均等性を保てます。任意の本数も指定できますが、同じ均等性の保証はありません。GUIのプレビューは各光源256本が上限です。光路表示は全光源で合計64本を上限とし、各光源の列の先頭から順番に選びます。[Sobol列の性質](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats.qmc.Sobol.html)

各 `sources` 要素で方式とseedを指定できます。

```json
{
  "count": 4096,
  "sampling": "sobol",
  "seed": 42,
  "origin_mm": [0, 0, 0],
  "direction": [0, 0, 1],
  "disk_radius_mm": 1,
  "wavelength_um": 0.55,
  "medium": 0,
  "total_weight": 1
}
```

`"sampling": "pseudorandom"` で従来の疑似乱数方式に切り替えられます。v0.3以降は省略時にもSobolを使うため、以前の生成光線を再現する場合は `pseudorandom` を指定します。CSV入力の光線にはサンプリング設定を適用しません。

## 対応する光学モデル

| 項目 | 対応範囲 |
|---|---|
| シーケンシャル | 指定順の面を追跡。屈折面は主透過光、ミラーと全反射は反射光を次の面へ進める |
| 非シーケンシャル | 最も近い面との交差を探し、Fresnel反射光・透過光を重みで分岐 |
| 面 | 平面、球面キャップ、円錐面、A4～A18の偶数次非球面、レンズ側面（円筒・円錐）、三角形メッシュ |
| 配置・口径 | 3次元の平行移動・回転（Rx→Ry→Rz）、円形・矩形口径 |
| 材料 | 一定屈折率、Sellmeier式、Schott式、波長別表の線形補間、内部吸収 |
| 光源・検出器 | 複数光源・複数波長、円板・円錐・Source Diode、複数の平面検出器 |
| 出力 | 光路、画素別積算光量、光量収支、光学距離、終端光線 |

長さはmm、真空波長はµm、演算はf64です。材料の有効波長範囲を実行前に確認し、範囲外をエラーにします。光学距離は各媒質の `n(λ) × 幾何学的距離` の合計です。

屈折面の `minus_medium`／`plus_medium` は局所法線の負側／正側の媒質番号で、`materials` 配列の添字を指定します。非シーケンシャルのレンズやロッドは、側面を含めて媒質境界を定義します。

## Zemaxファイルの読込と保存

| 定義 | 読込範囲 |
|---|---|
| Sequential Standard / Even Asphere | 面形状・間隔・材料・口径。Even AsphereのPARM 2をA4として解釈 |
| NSC Source Diode | 位置・回転・光量・波長番号・X/Y発散・発光領域・Gaussian幅・配列 |
| NSC Detector Rectangle | 位置・回転・寸法・ピクセル数 |
| NSC CAD Import | STEPで参照された単一の閉じたソリッド |
| AGF | GCAT指定カタログのSchott式1・Sellmeier 1式2、LD波長範囲、IT内部透過率 |

Source DiodeはGaussian係数Gx/Gy/Hx/Hyが1、非点収差0の範囲に対応します。Sequential ZMXの初期照射は主波長の軸上平行光です。視野・瞳・有限距離物体は照射条件の追加が必要です。

材料カタログはGUIの「AGFフォルダーを追加…」から指定できます。

読込レポートに未対応の定義や不足ファイルを表示します。コーティングは名前を保持し、未コートFresnelで計算するには明示設定が必要です。SPCDスペクトルを使う光源は、単色の計算波長を指定して追跡します。

STEPの初回読込にはFreeCADのPythonが必要です。`RAYTRACE_FREECAD_PYTHON` で実行ファイルを指定できます。Windowsの既定値は `%LOCALAPPDATA%/Programs/FreeCAD 1.1/bin/python.exe` です。0.02 mm設定で分割したメッシュをビルド元の `results/cad_cache` に保存し、追跡はRustで行います。CADの精度はメッシュ分割を変えた収束確認が必要です。

「保存…」は元のZMX全文と編集状態をJSONプロジェクトへ保存します。「ZMX書出し…」は次の編集をZemaxへ返します。

- Sequential：軸上の面形状・間隔・口径・材料。
- NSC：既存オブジェクトの位置・回転、Source Diodeの本数・光量・発散・配列・発光領域。

変更のないZMXは元の文字コードとレコードを保って書き出します。参照ファイルも保存先へコピーし、同名で内容が異なるファイルがあれば上書きを止めます。NSCの新規部品、形状・材料・波長表の変更、座標ブレークを伴う編集はJSONプロジェクトで保存します。

## 光量の読み方

`reference_plane` で指定した発射基準面の光線重み合計を100%とします。複数光源・波長の合計は各光源の `total_weight` の和です。検出器マップは画素内の積算光量で、照度密度を求めるには画素面積で割ります。

光量収支は、検出器到達、追跡領域外、幾何学的な外れ、界面・内部吸収、逐次追跡で追わない界面反射、分岐・深さの打切り、数値的な未解決分に分けて記録します。これらの和を発射光量と照合します。

非シーケンシャルの面別値は、同じ面への複数回の入射を積算します。`numerical_failure` は未解決の面交差、`medium_mismatch` は媒質境界の不整合です。非球面の斜入射は区間を分割して最も近い交点を探し、探索上限に達した光量を未解決分として記録します。

## OpticStudioと比べた主な未対応機能

[OpticStudioの公式機能一覧](https://www.ansys.com/products/optics/ansys-zemax-opticstudio)、[面型](https://ansyshelp.ansys.com/public/Views/Secured/Zemax/v252/en/OpticStudio_User_Guide/OpticStudio_Help/topics/Summary_Table_of_Sequential_Surface_Types.html)、[光源モデル](https://optics.ansys.com/hc/en-us/articles/42661687233939-Light-sources-used-in-illumination-design)と比較した、v0.5の主な制約です。

| 分野 | 未対応の機能 |
|---|---|
| ファイル・配置 | ZMX全定義の計算再現、任意の編集の再書出し、ZOS/ZBB/ZRD、Coordinate Break連鎖、NSC参照階層 |
| 面・照射 | 円筒レンズ・トロイド・自由曲面・回折面、視野・瞳・有限距離物体・Ray Aimingの自動再現 |
| 光源・材料 | IES/SDF/SPCD、Source Diodeの非点収差・超Gaussian、追加AGF分散式、温度依存、GRIN、複屈折 |
| 界面・散乱 | 偏光追跡、多層薄膜、実コーティング、BRDF/BSDF表面散乱、体積散乱、迷光経路の分類 |
| 評価・設計 | 収差図・MTF/PSF、自動最適化、ソルブ／ピックアップ、複数構成、公差・歩留まり解析 |
| GUI | 3Dビューでの操作、部品カタログ。部品の追加・配置・ロッド編集は2D投影と数値入力に対応 |

POPによる波動伝搬、干渉、回折によるビーム拡がり、ファイバーのモード結合は対象外です。

## 検証

```powershell
cargo test --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
./target/release/raytrace-gui.exe --smoke results/gui_check
```

テストではSnell則、Fresnel反射、全反射、多重反射、吸収、光学距離、分散、ZMX往復保存を確認します。サンプリングは等面積・等立体角・位置と方向の同時分布、Gaussian分布、seedと光線数変更時の再現性を確認します。

組立はZMXの別読込、材料番号の統合、側面との交差、剛体変換による位置・方向・光学距離の対応、保存・復元、GUIのドラッグ操作を検証します。

スクリーン解析はGaussianの既知の半値幅、複数ピーク、画面端で欠けた分布、受光ゼロ、光量を変えた場合の幅の不変性を検証します。

OpticStudioとの光線比較には `examples/exchange_check.rs` と `scripts/zemax_exchange_probe.py` を使えます。入力はダミー平面・レンズ2面・像面からなるZMXです。中心厚を0.25 mm増やして書き出し、同じ7本の光線を比較します。Python側にはpythonnetと有効なOpticStudio APIライセンスが必要で、インストール先は `RAYTRACE_ZEMAX_ROOT` で指定できます。

```powershell
cargo run --release --example exchange_check -- lens.zmx results/exchange
python scripts/zemax_exchange_probe.py --input results/exchange/lens_edited.zmx --rays results/exchange/reference.json --output results/zemax_exchange
```

計算核は `src/geometry.rs` と `src/trace.rs`、光源生成は `src/model.rs` と `src/sampling.rs`、ZMX交換は `src/zemax.rs`、部品の組立は `src/assembly.rs`、追跡後のスクリーン解析は `src/screen.rs` にあります。Rustライブラリとしても利用できます。
