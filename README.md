# raytrace-rs

Rust製の幾何光学シミュレーターです。シーケンシャル／非シーケンシャル光線追跡、波長依存の屈折・反射・吸収を扱います。日本語GUIとコマンドラインから、レンズ・ミラー・インテグレータロッドの光路と検出器分布を確認できます。

## ビルドと起動

WindowsとRust（Cargo）が必要です。初回ビルド時に依存ライブラリを取得します。

```powershell
cargo build --release --locked --features gui --bins
./target/release/raytrace-gui.exe
```

ビルド後は `Raytraceを起動.cmd` からも起動できます。GUIは光源・レンズ・ロッド・スクリーンの組立サンプルで開きます。「ファイル → 開く…」またはドラッグ＆ドロップで `.zmx` やJSONプロジェクトを読み込み、「計算する」で追跡します。

Windows x64用ZIPは `python scripts/package_windows.py` で作成できます。ビルド済みの場合は `--no-build` を指定します。ZIPにはGUI・CLI・サンプル・操作説明が入り、展開先で起動できます。基本の追跡にRustやOpticStudioは不要です。MSVC版はVisual C++ v14ランタイムを使います。`VCRUNTIME140.dll` が不足する環境では、[Microsoft公式のx64ランタイム](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)を導入してください。

「光学系」では、左に部品一覧と選択部品の設定、中央に広い光路図を表示します。「スクリーン」に切り替えると画素ごとの積算光量を確認できます。光線数・追跡方式は「計算設定」、ZMXの対応条件は「読込レポート」、光量収支はスクリーンの「光量の内訳」から開けます。

## 光源・レンズ・ロッドを組み合わせる

1. 「ファイル → 新規の光学系」で空の光学系を作ります。
2. 部品一覧の「＋ 追加 → 光源のZMX…」で光源を読み込みます。光源内のレンズやガラス形状も部品に含まれます。
3. 「＋ 追加 → レンズのZMX…」でレンズを読み込みます。レンズの試験光源・像面を除き、光学面と材料を追加します。
4. 「＋ 追加 → ロッド」でロッドを追加し、幅X・幅Y・長さと材料を設定します。
5. 「＋ 追加 → スクリーン」で観測面を追加し、配置して「計算する」を押します。

図の丸印をドラッグして、選んだ投影の2軸方向へ部品を移動できます。左側のX/Y/Z・「傾き」では位置と回転を数値指定します。光路図はZ–X／Z–Y／X–Y投影に対応し、縦横を同じ縮尺で表示します。「表示」メニューから縦横を独立した縮尺に切り替えられます。配置を変えると古い光線を隠し、再計算後に更新します。

ホイールでカーソル位置を中心に拡大し、背景のドラッグ、右ボタンまたは中ボタンのドラッグで表示範囲を移動できます。「選択へ」で選択部品を拡大し、「全体」で光学系全体の表示へ戻ります。表示範囲は投影ごとに保ち、表示操作は部品の座標を変更しません。

上部の戻す／やり直すボタン、`Ctrl+Z`、`Ctrl+Shift+Z`／`Ctrl+Y`で配置編集を戻せます。一連のドラッグや数値入力は1操作として扱います。テキスト入力中はその欄の編集を優先します。履歴は最大64操作、CAD形状を含む保存量は約128 MiBを目安に古い操作から整理します。部品の「操作 → 部品を複製」でコピーを作れます。計算時の配置へ戻ると、その結果の光路を再表示します。

配置の基準は、光源が発光配列の中心、レンズが最初の光学面の頂点、ロッドが入口中心です。部品一覧のチェックで有効／無効を切り替え、選択部品の「操作 → 部品を削除」で取り除けます。光源を選ぶと本数・波長・分布・サンプリング、ロッドを選ぶと寸法と波長依存の材料、スクリーンを選ぶと寸法と画素数を編集できます。単体のZMXやJSONでは光源と各光学面を選んで編集します。

組立光学系は非シーケンシャルで追跡します。各部品の内部形状・波長依存の材料を維持し、全体で材料番号を割り当てます。円形口径の軸上逐次レンズには、各面の口径端を結ぶ円筒／円錐側面を追加します。側面形状はこの幾何近似に基づくため、実部品の有効径と形状に合わせて確認してください。

「保存…」は部品の元定義・配置・材料・ロッド寸法を含むJSONを保存します。「開く…」で、元のZMXやSTEPを開き直さずに復元できます。組立光学系のZMX書出しは未対応です。

組立サンプルは「ファイル → サンプルを開く → 光源＋レンズ＋ロッド」からも開けます。CLI用のプロジェクトは次のように生成できます。

```powershell
cargo run --release --example assembly_demo -- results/assembly_demo.json
./target/release/raytrace-rs.exe results/assembly_demo.json --threads 4 --output results/assembly_trace.json
```

## スクリーンの表示

「スクリーン」タブで観測面を選び、「ピーク = 1」と「基準光量比 [%]」を切り替えられます。基準光量比は発射基準面の光量に対する各画素の積算光量です。色目盛は線形／平方根から選びます。表示の変更は再追跡を必要とせず、元の光量と光量収支を保持します。配置・条件を変更した後は前回の結果であることを表示します。

「半値幅」で、最大画素を通るX・Y断面の半値交点を赤い破線で示し、FWHMをmmで表示します。「断面」で両方向のプロファイルを同じ画面内に表示します。座標はスクリーンの局所座標です。

「表示範囲」で基準光量比の上端を固定できます。「ファイル → スクリーンを書き出す…」は、元の画素光量・基準光量比・mm²当たりの光量比をCSVへ、X/Y断面を別CSVへ、分布をPNGへ保存します。基準面・座標・正規化・表示範囲・計算条件も同じフォルダーに記録します。

半値交点は画素中心の値を線形補間して求めます。断面に複数の山がある場合は、最大ピークの両側で最初に半値へ達する位置を採ります。同じ最大値が複数ある場合は、画素配列の先頭（Yの小さい行、Xの小さい列）を選びます。交点が両側に揃わない方向は「半値交点不足」、受光ゼロは「受光なし」と表示します。幅の解釈には表示された画素寸法を併せて確認してください。

## パラメーター走査

選択部品の「操作 → この部品の値を走査…」、または「ファイル → パラメーター走査…」で専用画面を開きます。部品の位置・角度、ロッドの幅・長さ、単体モデルの面の位置・角度、全光源の計算波長から1項目を選び、範囲と点数または値の一覧を指定します。GUIの波長単位はnm、計画JSONではμmです。

各ケースは同じ光源方式とseedを使い、受光量・半値幅・追跡時間・失敗理由を記録します。完了ケースの「表示」で、保存した結果と配置を開けます。「2ケースを比較」では分布とX/Y断面を並べ、共通の基準光量比と表示上端で比較します。半値幅や表示設定の変更は追跡後の処理です。

計算中は完了した発射光線数を表示し、「中止」で停止できます。単発計算では前回の完了結果を保持します。走査は完了したケースを保存し、計算途中だったケースを未完了へ戻します。「走査記録を開く… → 未完了のケースを再開」で続行でき、完了・失敗が確定したケースを維持します。走査記録は同じソフトの版で再開します。各ケースは合計200万光線、計画全体は256ケースが上限です。

```powershell
./target/release/raytrace-rs.exe --scan examples/sequential_lens.json --plan scan_plan.json --output results/focus_scan
./target/release/raytrace-rs.exe --resume-scan results/focus_scan
```

`scan_plan.json` の例です。部品番号・面番号・軸番号は0から始まり、軸0/1/2はX/Y/Zです。`rays_per_source: null` は元の本数を使います。

```json
{
  "parameter": { "type": "surface_position", "surface": 2, "axis": 2 },
  "values": [65.0, 68.5, 72.0],
  "threads": 4,
  "rays_per_source": 4096
}
```

組立モデルには `component_position`・`component_rotation`（`component`・`axis`）、`rod_size`（`component`・`axis`：幅X／幅Y／長さ）を指定します。単体モデルは `surface_position`・`surface_rotation`（`surface`・`axis`）、波長は `{ "type": "wavelength" }` です。

## 保存先

タイトルの点印は未保存の編集を表し、タイトルにカーソルを合わせると現在の保存先を確認できます。「ファイル → 最近開いたファイル」から読み直せます。計算記録JSONも「開く…」で配置と結果を表示できます。

明示入力の光線を含む記録では、GUIは保存済みの結果を表示し、「モデル光源で計算」で光源定義からの計算へ切り替えます。保存した入力光線を固定して再計算する場合はCLIの `--replay` を使います。

Windowsでは設定・CADキャッシュ・GUI走査記録を `%LOCALAPPDATA%/Raytrace` に保存します。`RAYTRACE_DATA_DIR` で保存先を変更できます。プロジェクト・計算結果・スクリーンの出力先は保存時に指定します。

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

材料カタログはGUIの「読込レポート → AGFフォルダーを追加…」から指定できます。追加後に部品を読み込むか、「現在のファイルを読み直す」を使います。

読込レポートは部品単位で、計算対応・原文のみ保持・条件指定待ち・条件指定済み・未対応を表示します。計算波長ごとの屈折率と吸収係数も確認できます。コーティングは名前を保持し、未コートFresnelで計算するには明示設定が必要です。SPCDスペクトルを使う光源は、単色の計算波長を指定して追跡します。未定義の波長番号、未知の面・オブジェクト属性、未対応の環境条件は読込時に明示します。

STEPの初回読込にはFreeCADのPythonが必要です。`RAYTRACE_FREECAD_PYTHON` で実行ファイルを指定できます。Windowsの既定値は `%LOCALAPPDATA%/Programs/FreeCAD 1.1/bin/python.exe` です。0.02 mm設定で分割したメッシュをデータ保存先の `cad_cache` に置き、追跡はRustで行います。CADの精度はメッシュ分割を変えた収束確認が必要です。

「保存…」は元のZMX全文と編集状態をJSONプロジェクトへ保存します。「ZMX書出し…」は次の編集をZemaxへ返します。

- Sequential：軸上の面形状・間隔・口径・材料。
- NSC：既存オブジェクトの位置・回転、Source Diodeの本数・光量・発散・配列・発光領域。

変更のないZMXは元の文字コードとレコードを保って書き出します。参照ファイルも保存先へコピーし、同名で内容が異なるファイルがあれば上書きを止めます。NSCの新規部品、形状・材料・波長表の変更、座標ブレークを伴う編集はJSONプロジェクトで保存します。

## 光量の読み方

`reference_plane` で指定した発射基準面の光線重み合計を100%とします。複数光源・波長の合計は各光源の `total_weight` の和です。検出器マップは画素内の積算光量で、照度密度を求めるには画素面積で割ります。

光量収支は、検出器到達、追跡領域外、幾何学的な外れ、界面・内部吸収、逐次追跡で追わない界面反射、分岐・深さの打切り、数値的な未解決分に分けて記録します。これらの和を発射光量と照合します。

非シーケンシャルの面別値は、同じ面への複数回の入射を積算します。`numerical_failure` は未解決の面交差、`medium_mismatch` は媒質境界の不整合です。非球面の斜入射は区間を分割して最も近い交点を探し、探索上限に達した光量を未解決分として記録します。

GUIの「光量の内訳 → 追跡の診断」から、未解決・媒質不整合・打切りの光量、原因の面、代表光線の位置・方向・波長を確認できます。「配置で確認」で対応する部品を選択します。記録する代表例は最大64グループとし、保存上限を超えた分岐の数と光量も集計します。分岐数は同じ発射IDの重複を含みます。

## 計算記録と再計算

CLIの計算結果とGUIの「ファイル → 計算結果を保存…」は、計算時点のプロジェクト・実際に使った光学系・光源条件・ソフトの版・追跡上限を結果と一緒に保存します。CSV入力では発射光線そのものを含み、元のCSVを開き直さずに再計算できます。

```powershell
./target/release/raytrace-rs.exe examples/sequential_lens.json --rays-per-source 1024 --output results/lens_run.json
./target/release/raytrace-rs.exe --replay results/lens_run.json --threads 4 --output results/lens_replay.json
```

計算記録の形式は `raytrace-run-v1`、CLI出力の `schema_version` は2です。既存の `report`・光量比・ベンチマーク欄に `calculation` を追加しています。再計算では出力先・スレッド数・繰返し回数・保存する光線情報を指定でき、記録した光学条件を維持します。GUIの光路表示用サンプルは計算記録から除き、再計算時に `--paths N` で指定できます。配置の編集にはプロジェクトJSONを使います。

## 収束確認

光線数・seed・画素数・CAD分割精度を変えた計算を一括実行できます。既定は各光源256／1024／4096本、seed 42／43、64角画素、CAD分割精度0.02 mmです。

```powershell
./target/release/raytrace-rs.exe --convergence examples/sequential_lens.json --output results/lens_convergence
./target/release/raytrace-rs.exe --convergence model.zmx --plan plan.json --output results/cad_convergence
```

計画JSONの例です。空配列の `ray_counts`・`seeds`・`pixels` は元の設定を維持します。指定したseedは先頭光源へ割り当て、以後の光源は順に+1します。

```json
{
  "ray_counts": [256, 1024],
  "seeds": [42, 43],
  "pixels": [[32, 32], [64, 64]],
  "cad_deflections_mm": [0.02, 0.01],
  "threads": 4,
  "save_runs": true
}
```

`convergence.json` に受光量、光量収支、半値幅、画素寸法、三角形数、追跡時間、各ケースのエラーを保存します。ほかの条件を揃え、光線数・画素数の最大値、CAD分割精度の最小値、最初のseedをそれぞれ比較先とします。分布の差は、最大64角の共通画素へ面積比で集約した光量のL1差を、発射基準光量に対する割合で示します。細かい画素内の分布は一様と仮定した比較です。差分は感度の確認値で、収束の自動判定は行いません。

CAD分割精度を比較する場合はSTEP参照を持つ元のZMXを指定します。保存済みJSONのメッシュは、その保存時の分割を使います。`save_runs: true` ではケースごとの計算記録も保存し、`--replay` で再計算できます。各ケースは合計200万光線、計画全体は256ケースが上限です。

## OpticStudioと比べた主な未対応機能

[OpticStudioの公式機能一覧](https://www.ansys.com/products/optics/ansys-zemax-opticstudio)、[面型](https://ansyshelp.ansys.com/public/Views/Secured/Zemax/v252/en/OpticStudio_User_Guide/OpticStudio_Help/topics/Summary_Table_of_Sequential_Surface_Types.html)、[光源モデル](https://optics.ansys.com/hc/en-us/articles/42661687233939-Light-sources-used-in-illumination-design)と比較した、v0.8の主な制約です。

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
./target/release/raytrace-gui.exe --screen --screen-profiles --smoke results/gui_screen
```

テストではSnell則、Fresnel反射、全反射、多重反射、吸収、光学距離、分散、ZMX往復保存を確認します。サンプリングは等面積・等立体角・位置と方向の同時分布、Gaussian分布、seedと光線数変更時の再現性を確認します。

組立はZMXの別読込、材料番号の統合、側面との交差、剛体変換による位置・方向・光学距離の対応、保存・復元、GUIのドラッグ操作を検証します。画面切替・部品選択時のデータ保持、全光源設定時の局所座標の保持も確認します。

スクリーン解析はGaussianの既知の半値幅、複数ピーク、画面端で欠けた分布、受光ゼロ、光量を変えた場合の幅の不変性を検証します。

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

計算核は `src/geometry.rs` と `src/trace.rs`、光源生成は `src/model.rs` と `src/sampling.rs`、ZMX交換は `src/zemax.rs`、部品の組立は `src/assembly.rs`、追跡後のスクリーン解析は `src/screen.rs` にあります。Rustライブラリとしても利用できます。
