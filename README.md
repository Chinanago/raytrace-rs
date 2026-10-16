# raytrace-rs

Rust製の幾何光学シミュレーターです。シーケンシャル／非シーケンシャル光線追跡、波長依存の屈折・反射・吸収を扱います。日本語GUIとコマンドラインから、レンズ・ミラー・インテグレータロッドの光路と検出器分布を確認できます。

## Windows版をダウンロードして起動

[Windows版ZIP（v0.10.0）](https://github.com/Chinanago/raytrace-rs/releases/download/v0.10.0/raytrace-rs-0.10.0-windows-x64.zip)を展開し、「Raytraceを起動.cmd」を開きます。基本の光線追跡にはRustやOpticStudioは不要です。版ごとの変更点とチェックサムは[Releases](https://github.com/Chinanago/raytrace-rs/releases)で確認できます。

起動すると光源・レンズ・ロッド・スクリーンの組立サンプルが開きます。部品を選んで配置し、「計算する」→「スクリーン」で結果を確認します。提供されたZMXは「追加」から光源・光学部品として読み込み、「保存…」で配置と条件をJSONへ保存できます。

Visual C++ v14ランタイムを使います。DLL不足の表示が出る環境では、[Microsoft公式のx64ランタイム](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)を導入してください。

## ソースからビルドして起動

WindowsとRust（Cargo）が必要です。初回ビルド時に依存ライブラリを取得します。

```powershell
cargo build --release --locked --features gui --bins
./target/release/raytrace-gui.exe
```

ビルド後は `Raytraceを起動.cmd` からも起動できます。GUIは光源・レンズ・ロッド・スクリーンの組立サンプルで開きます。「ファイル → 開く…」またはドラッグ＆ドロップで `.zmx` やJSONプロジェクトを読み込み、「計算する」で追跡します。

配布用ZIPは `python scripts/package_windows.py` で作成できます。ビルド済みの場合は `--no-build` を指定します。実行ファイル・サンプル・操作説明・ライセンスを同梱し、`package.json` に版とSHA-256を記録します。

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

## コーティング・スペクトル・材料

逐次ZMXの `COAT` は、ZMXと同じフォルダーのDAT、追加カタログ、標準のZemax CoatingsフォルダーからIDEAL／TABLE定義を読み込みます。ファイルを指定する `COFN` がある場合はその名前を使います。「読込レポート → カタログフォルダーを追加…」で検索先を追加できます。

IDEALは透過・反射・全反射時の反射率、TABLEは波長・入射角ごとの光量係数を扱います。TABLEは波長方向に線形補間、角度方向にcos補間し、高屈折率側からの入射では低屈折率側の角度を使います。各界面のS/P強度を平均する幾何光学モデルで、偏光状態・位相は追跡しません。透過・反射の残りを界面吸収として記録します。多層膜の処方やNSCの面別コーティングは読込レポートで計算を止めます。「未コートFresnelで計算」を選ぶと、保存されたコーティングを保持して未コート条件を採用します。[IDEAL形式](https://ansyshelp.ansys.com/public/Views/Secured/Zemax/v252/en/OpticStudio_User_Guide/OpticStudio_Help/topics/The_IDEAL_Data_Section.html)、[TABLE形式](https://ansyshelp.ansys.com/public/Views/Secured/Zemax/v25101/en/OpticStudio_User_Guide/OpticStudio_Help/topics/The_TABLE_Data_Section.html)

NSC光源のSPCDファイルは、昇順の波長[μm]と相対光量の2列（3〜200点）を読み込みます。System Wavelengths、Uniform Power Spectrum、Black Body Spectrumにも対応します。各光線の波長を相対光量で選び、発射光量の合計を保ちます。波長選択は位置・方向・ダイオード配列とは独立したSobolの次元を使います。スペクトルをプロジェクト・計算記録へ保存するため、保存後の追跡には元のSPCDを必要としません。CIE色座標からのスペクトル推定は未対応です。[SPCD形式](https://ansyshelp.ansys.com/public/Views/Secured/Zemax/v252/en/OpticStudio_User_Guide/OpticStudio_Help/topics/Defining_a_spectrum_file.html)

AGFの分散式1〜13と内部透過率ITを扱います。ITの指定厚さから吸収係数[1/mm]へ変換し、吸収係数を波長方向に線形補間します。材料・内部透過率・コーティングの有効波長範囲を外れる光源は、計算前に理由を示して停止します。「全光源の計算波長を単色で指定」でスペクトルと比較できます。[AGF形式と分散式番号](https://ansyshelp.ansys.com/public/Views/Secured/Zemax/v252/en/OpticStudio_User_Guide/OpticStudio_Help/topics/The_AGF_BGF_File_Formats.html)

Coordinate Breakは、変換順序と変換後の軸に沿った厚さを順番に適用します。NSCの正・負の参照番号、参照の連鎖、Inside Ofによる初期媒質と外側媒質も保持します。Zemax既定の局所X→Y→Z回転を、配置画面の座標へ変換して読み込みます。座標リターン・ソルブ・NSOOの追加設定は読込レポートで対応範囲を示します。NSCの参照関係は、編集後の局所配置へ逆変換して書き戻します。Coordinate Breakを含む編集はJSONへ保存でき、ZMXは無変更保存に対応します。[座標系と参照関係](https://ansyshelp.ansys.com/public/Views/Secured/Zemax/v25101/en/OpticStudio_User_Guide/OpticStudio_Help/topics/Object_Placement.html)

## スクリーンの表示

「スクリーン」タブで観測面を選び、「ピーク = 1」と「基準光量比 [%]」を切り替えられます。基準光量比は発射基準面の光量に対する各画素の積算光量です。色目盛は線形／平方根から選びます。表示の変更は再追跡を必要とせず、元の光量と光量収支を保持します。配置・条件を変更した後は前回の結果であることを表示します。

「半値幅」で、X・Y断面の半値交点を赤い破線で示し、FWHMをmmで表示します。初期の断面は最大画素を通ります。「断面位置・ROI」で任意の局所座標を指定すると、最も近い画素列へ移ります。半値は各断面の最大値の50%です。「断面」で両方向のプロファイルを同じ画面内に表示します。座標はスクリーンの局所座標です。

「表示範囲」で基準光量比の上端を固定できます。「ファイル → スクリーンを書き出す…」は、元の画素光量・基準光量比・mm²当たりの光量比をCSVへ、X/Y断面を別CSVへ、分布をPNGへ保存します。基準面・座標・正規化・表示範囲・計算条件も同じフォルダーに記録します。

半値交点は画素中心の値を線形補間して求めます。断面に複数の山がある場合は、最大ピークの両側で最初に半値へ達する位置を採ります。同じ最大値が複数ある場合は、画素配列の先頭（Yの小さい行、Xの小さい列）を選びます。交点が両側に揃わない方向は「半値交点不足」、受光ゼロは「受光なし」と表示します。幅の解釈には表示された画素寸法を併せて確認してください。

ROIは指定基準面の光量に対する割合を先に表示し、面積[mm²]と平均光量比[%/mm²]を併記します。指定矩形をスクリーン内へ切り詰め、端の画素は重なる面積で按分します。画素内で密度が一定とする後処理です。出力の `postprocessing.json` に範囲・計算法・実際の断面位置を、`selected_x_profile.csv`／`selected_y_profile.csv` に選択断面を保存します。元の画素CSVも保持します。

## パラメーター走査

選択部品の「操作 → この部品の値を走査…」、または「ファイル → パラメーター走査…」で専用画面を開きます。部品の位置・角度、ロッドの幅・長さ、単体モデルの面の位置・角度、全光源の計算波長から1項目を選び、範囲と点数または値の一覧を指定します。GUIの波長単位はnm、計画JSONではμmです。

「2軸の走査・値の連動」を開くと、第2軸の値一覧と、`連動先 = 連動元 × 係数 + 差分` を設定できます。差分は計画JSONの単位（mm・deg・μm）で指定します。「距離・差分を保つ連動を追加」は現在の差分を初期値に使います。連動は同じ単位の項目間で設定でき、循環・連動先の重複・走査軸との競合を計算前に検査します。2軸は全組合せを計算します。

各ケースは同じ光源方式とseedを使い、受光量・半値幅・追跡時間・失敗理由を記録します。完了ケースの「表示」で、保存した結果と配置を開けます。「2ケースを比較」では分布とX/Y断面を並べ、共通の基準光量比と表示上端で比較します。比較するスクリーンはA/Bそれぞれで選べます。半値幅や表示設定の変更は追跡後の処理です。

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
  "secondary": { "parameter": { "type": "wavelength" }, "values": [0.45, 0.635] },
  "links": [],
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

一様円板は面積、一様円錐は立体角に対して一様になるよう変換します。Source Diodeには逆累積分布関数を使い、発光領域の端で切ったGaussian／超Gaussian分布を保ちます。角度は前方の±90度内に制限し、光線数を増やしても同じseedのサンプル列の先頭を維持します。

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
| NSC Source Diode | 位置・回転・光量・波長番号・X/Y発散・発光領域・Gaussian幅・角度と発光面の超Gaussian係数・非点収差・配列 |
| NSC Detector Rectangle | 位置・回転・寸法・ピクセル数 |
| NSC CAD Import | STEPで参照された単一の閉じたソリッド |
| AGF | GCAT指定カタログの分散式1〜13、LD波長範囲、IT内部透過率 |

Source Diodeの角度分布は `exp(-2(|θx/αx|^(2Gx)+|θy/αy|^(2Gy)))`、発光面の分布も同じ形で幅sx/syと係数Hx/Hyを使います。G/Hが1で通常のGaussianになります。αは強度が1/e²になる半角です。非点収差Aを正にすると、XZ方向の仮想発光点を局所Z=-Aへ移し、Z=0のX方向の線上から発射します。このとき発光面の幅とHの設定は原文に保持します。[Source Diodeの定義](https://ansyshelp.ansys.com/public/Views/Secured/Zemax/v252/en/OpticStudio_User_Guide/OpticStudio_Help/topics/Source_Diode.html)

Sequential ZMXはWAVMの相対重み、Angle／Object Heightの視野とFWGN、ENPDによる瞳径を読み込みます。STOPより前の屈折面から、主波長の近軸入射瞳の位置と倍率を計算します。Ray AimingがOffならその入射瞳へ照射し、Paraxial／Realでは各波長の実光線を反復してSTOPの指定点へ合わせます。STOPの照準半径は、Paraxialでは近軸計算、Realでは主波長の軸上周辺光線から決めます。[視野型](https://ansyshelp.ansys.com/public/Views/Secured/Zemax/v251/en/OpticStudio_User_Guide/OpticStudio_Help/topics/Field_Type.html)、[Ray Aimingの定義](https://ansyshelp.ansys.com/public/Views/Secured/Zemax/v251/en/OpticStudio_User_Guide/OpticStudio_Help/topics/Ray_Aiming.html)

入射瞳の計算は、最初の面からSTOPまでが同軸で正の面間隔を持つ平面・球面・偶数次非球面の屈折系に対応します。光学系全体を部品として移動・回転した場合も追従します。偏心・傾きを含む内部配置、STOPより前のミラー、無限遠の入射瞳は拡張予定です。照準が収束しない場合は、光源・光線ID・波長と理由を表示して計算を止めます。

視野ごとのVDX/VDY・VCX/VCY・TANは、正規化した瞳座標を移動・圧縮してから回転します。Offでは入射瞳、Ray Aiming時はSTOPで適用します。**光量の基準は、補正後の照射領域から発射した光線の重み合計を100%**とします。補正前の瞳全体や物点の全放射光に対する光量は別途評価が必要です。実開口で外れた光と界面の減衰は、追跡結果の光量収支へ分けて記録します。[ビネッティング係数の定義](https://ansyshelp.ansys.com/public/Views/Secured/Zemax/v251/en/OpticStudio_User_Guide/OpticStudio_Help/topics/Vignetting_Factors.html)

光源の「分布と非点収差」「視野と瞳」に詳細設定をまとめています。計算した入射瞳の中心・半径・照準方式・主波長は、計算条件の画面と保存結果から確認できます。新しい照射機能を使うプロジェクトは形式v3、生成光線の記録はサンプリング版3で保存します。従来のプロジェクトと対応する版1/2の計算記録も読み込めます。過去の記録は保存時の照射定義を維持し、新しい入射瞳計算を使う場合は元ZMXを読み直します。

材料カタログはGUIの「読込レポート → カタログフォルダーを追加…」から指定できます。追加後に部品を読み込むか、「現在のファイルを読み直す」を使います。

読込レポートは部品単位で、計算対応・原文のみ保持・条件指定待ち・条件指定済み・未対応を表示します。計算波長ごとの屈折率と吸収係数も確認できます。対応するコーティングとスペクトルを計算に使用し、未コート・単色の明示設定も記録します。未定義の波長番号、未知の面・オブジェクト属性、未対応の環境条件は読込時に明示します。

STEPの初回読込にはFreeCADのPythonが必要です。`RAYTRACE_FREECAD_PYTHON` で実行ファイルを指定できます。Windowsの既定値は `%LOCALAPPDATA%/Programs/FreeCAD 1.1/bin/python.exe` です。0.02 mm設定で分割したメッシュをデータ保存先の `cad_cache` に置き、追跡はRustで行います。CADの精度はメッシュ分割を変えた収束確認が必要です。

「保存…」は元のZMX全文と編集状態をJSONプロジェクトへ保存します。「ZMX書出し…」は次の編集をZemaxへ返します。

- Sequential：軸上の面形状・正負の間隔・口径・ミラー・材料と内部透過率、波長表、IDEAL／TABLEコーティング。対応する物体・視野条件は、照射配置の変更を伴わない編集で保持します。
- NSC：参照関係を含む既存オブジェクトの位置・回転、Source Diodeの本数・光量・発散・超Gaussian係数・非点収差・配列・発光領域、単色波長・SPCDスペクトル。

変更のないZMXは元の文字コードとレコードを保って書き出します。参照ファイルも保存先へコピーし、保存済みの材料・コーティング・スペクトルからテキスト参照ファイルを復元できます。同名で内容が異なるファイルがあれば上書きを止めます。独自のAGF／DAT／SPCDには出力名を付けます。NSCの新規部品・形状・材料・面別コーティングの変更、座標ブレークを伴う編集、視野ごとに異なるスペクトル・光量比の編集はJSONプロジェクトで保存します。

光線数・seed・発射基準面・単色／未コートの上書き条件まで含む再計算には、JSONプロジェクトと計算記録を使います。

## 光量の読み方

`reference_plane` で指定した発射基準面の光線重み合計を100%とします。複数光源・波長の合計は各光源の `total_weight` の和です。検出器マップは画素内の積算光量で、照度密度を求めるには画素面積で割ります。

非シーケンシャル追跡は光量の大きい未処理分岐から進め、発射光線ごとの計算回数上限で強い反射を取り残す影響を抑えます。上限到達時は、残った光量を未解決分として保持します。非球面は区間ごとの単調性と曲率の上界を使って交点を絞り込み、かすめる入射でも直前の面の拾い直しを抑えます。

CADの媒質判定には三角形の幾何法線を使います。補間した光学法線が境界の表裏と食い違う反射・屈折を作る場合は幾何法線へ切り替え、面ごとの `normal_fallbacks` とGUIの診断に回数を記録します。切替が多い面はCAD分割精度を変えて結果を確認します。

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

[OpticStudioの公式機能一覧](https://www.ansys.com/products/optics/ansys-zemax-opticstudio)、[面型](https://ansyshelp.ansys.com/public/Views/Secured/Zemax/v252/en/OpticStudio_User_Guide/OpticStudio_Help/topics/Summary_Table_of_Sequential_Surface_Types.html)、[光源モデル](https://optics.ansys.com/hc/en-us/articles/42661687233939-Light-sources-used-in-illumination-design)と比較した、v0.10の主な制約です。

| 分野 | 未対応の機能 |
|---|---|
| ファイル・配置 | ZMX全定義の計算再現、任意の編集の再書出し、ZOS/ZBB/ZRD、座標リターン、NSOOの追加設定 |
| 面・照射 | 円筒レンズ・トロイド・自由曲面・回折面、偏心・傾き・反射を含む入射瞳、無限遠の瞳、強い瞳収差や焦線での頑健な照準探索、像高指定の視野 |
| 光源・材料 | IES/SDF、CIE色座標からのスペクトル推定、温度依存、GRIN、複屈折 |
| 界面・散乱 | 偏光・位相の追跡、多層薄膜の処方、NSC面別コーティング、BRDF/BSDF表面散乱、体積散乱、迷光経路の分類 |
| 評価・設計 | 収差図・MTF/PSF、自動最適化、ソルブ／ピックアップ、複数構成、公差・歩留まり解析 |
| GUI | 3Dビューでの操作、部品カタログ。部品の追加・配置・ロッド編集は2D投影と数値入力に対応 |

POPによる波動伝搬、干渉、回折によるビーム拡がり、ファイバーのモード結合は対象外です。

## 自動テストと配布確認

GitHubへのpushとPull Requestで、Windows上の整形・Clippy・全テスト・解析解／ZMX往復保存・配布ZIPの作成と確認を自動実行します。配布確認はWindows標準ディレクトリだけをPATHに残し、4サンプル、ZMX読込、配置編集、保存と元ファイル削除後の再計算を実行します。GUIの対話操作とFreeCAD初回変換は別途確認します。

## 検証

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

## ライセンス

[MIT License](LICENSE)。依存ライブラリとフォントには、それぞれのライセンスが適用されます。Windows版ZIPには、その著作権表示と利用条件を `THIRD_PARTY_LICENSES.txt` に同梱しています。
