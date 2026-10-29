# raytrace-rs

光源・レンズ・インテグレータロッドを配置し、光路とスクリーン上の光量分布を調べるRust製の幾何光学シミュレーターです。日本語GUIとコマンドラインで、シーケンシャル／非シーケンシャルの両方を扱います。

[Windows版ZIP（v0.11.0）](https://github.com/Chinanago/raytrace-rs/releases/download/v0.11.0/raytrace-rs-0.11.0-windows-x64.zip) · [機能と対応範囲](docs/FEATURES.md) · [操作ガイド](docs/GUIDE.md) · [変更履歴](https://github.com/Chinanago/raytrace-rs/releases)

## はじめる

Windows x64版のZIPを展開し、「Raytraceを起動.cmd」を開きます。基本の光線追跡にはRustやOpticStudioのインストールは不要です。

1. 光源・レンズ・ロッド・スクリーンの組立サンプルで、部品を選んで配置します。
2. 「計算する」を押して、光路を確認します。
3. 「スクリーン」で分布と半値幅、光量収支を確認します。

自分のデータを使うときは「ファイル → 新規の光学系」から始め、「＋ 追加」で光源とレンズのZMXを別々に読み込みます。ロッドとスクリーンを加え、一つの光学系として追跡できます。配置と条件はJSONプロジェクトへ保存します。

Visual C++ v14ランタイムを使います。DLL不足の表示が出る環境では、[Microsoft公式のx64ランタイム](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)を導入してください。STEPもRustで直接読み込みます。

## できること

| 目的 | 機能 |
|---|---|
| 部品を配置する | ZMXの光源・レンズ、ロッド、スクリーンを組み合わせ、2D投影上のドラッグと数値で位置・回転を編集。複製、無効化、元に戻す・やり直し |
| 光の進み方を追う | 指定順の面を追うシーケンシャル、最も近い面へ進む非シーケンシャル。屈折、反射、全反射、多重反射、吸収をCPUで並列計算 |
| 光源と照射を設定する | 複数光源・スペクトル、Sobol列、Source DiodeのGaussian／超Gaussian分布と非点収差。逐次系の入射瞳・Ray Aiming・視野の瞳補正 |
| 波長による変化を調べる | レンズ・ロッドの屈折率と内部吸収、AGFの分散式1〜13、波長・入射角別のIDEAL／TABLEコーティング |
| スクリーンで評価する | 画素別光量、ピーク正規化、X/Y断面と半値幅、矩形領域（ROI）の光量。CSV・PNG出力 |
| 条件を比較する | 1軸／2軸のパラメーター走査、値の線形連動、中止・再開、2ケースの分布比較。光線数・画素数・CAD分割精度を変える収束確認 |
| 計算を保存・再現する | 配置・材料を含むプロジェクト保存、条件と結果を含む計算記録、保存記録からの再計算。CLIとRustライブラリからも利用可能 |

通常の画面は部品一覧と広い光路図を中心にしています。光源の詳細設定は折りたたみ、パラメーター走査は専用画面で開きます。

## Zemaxデータを使う

`.zmx` を直接読み込み、対応する形状・材料・光源・配置を計算に使います。読込レポートには、**計算に使用する定義・原文として保持する定義・条件の指定が必要な定義・未対応の定義**を表示します。

シーケンシャルのStandard／Even Asphere、非シーケンシャル（NSC）のSource Diode／Detector Rectangle／STEP参照のCAD Importなどに対応します。ZMX書出しでは、対応する面・光源・配置・参照ファイルの編集を反映できます。組立光学系や対応範囲を超える編集はJSONプロジェクトで保存します。[読込・保存の詳細](docs/FEATURES.md#zemaxファイルとcad)

## 計算の範囲と確認状況

光線追跡による光路と光量を扱います。偏光・位相、回折・干渉・波動伝搬、収差図・MTF/PSF、自動最適化、公差解析は対象に含めていません。入射瞳・Ray Aimingは、最初の面から絞りまで同軸で正の面間隔を持つ屈折系に対応します。[機能ごとの条件と未対応項目](docs/FEATURES.md)

v0.11.0では132テストが通過しています。STEPの従来版比較、解析解とZMX往復保存、Windows配布ZIPの動作も確認しています。OpticStudioとの実機照合は継続課題です。[検証方法と開発情報](docs/DEVELOPMENT.md)

## AIエージェントから使う

[raytrace-rsのAgent Skill](https://github.com/Chinanago/raytrace-rs/tree/main/skills/raytrace-rs)を公開しています。raytrace-rsのCLI・GUIを操作するための手順で、ZMXの読込、材料、追跡、出力の読み方、走査・再計算を扱います。

Codexでは、次のように依頼してインストールできます。

```text
$skill-installer で Chinanago/raytrace-rs の skills/raytrace-rs をインストールして。
```

インストール後は `$raytrace-rs` で呼び出し、モデルと計算条件を指定します。ほかのAIエージェントでも、フォルダー内の `SKILL.md` と、そこから参照される文書を読み込ませて利用できます。スキルはMITライセンスです。

## ドキュメント

| 読みたいこと | 参照先 |
|---|---|
| 光学モデル、ZMX互換性、機能ごとの対応条件 | [機能と対応範囲](docs/FEATURES.md) |
| 部品の配置、スクリーン、走査、保存、CLI | [操作ガイド](docs/GUIDE.md) |
| ソースからのビルド、自動テスト、検証、配布物の作成 | [開発と検証](docs/DEVELOPMENT.md) |

ソースは `raytrace-core`、`raytrace-formats`、`raytrace-io`、`raytrace-app`、`raytrace-cli`、`raytrace-gui` の6クレートに分かれています。依存方向は `core ← formats ← io ← app ← {cli, gui}` です。ZMX／AGF／SPCD・STEPを扱う `raytrace-formats` はプロジェクトモデルに依存せず、`raytrace-io` が読込結果を `Project` に接続します。[構成の詳細](docs/DEVELOPMENT.md#リポジトリ構成)

## ライセンス

[MIT License](LICENSE)。依存ライブラリとフォントには、それぞれのライセンスが適用されます。Windows版ZIPには、その著作権表示と利用条件を `THIRD_PARTY_LICENSES.txt` に同梱しています。
