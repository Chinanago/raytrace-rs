# 開発と検証

ソースリポジトリでのビルド・検証・配布物の作成方法です。目指す構成と設計指針は[グランドデザイン](DESIGN.md)にあります。通常の操作は[操作ガイド](GUIDE.md)、機能ごとの対応条件は[機能と対応範囲](FEATURES.md)を参照してください。

ライブラリの失敗は `raytrace-core`・`raytrace-formats`・`raytrace-io`・`raytrace-engine` の各 `Error` 列挙型と `Result<T>` で返します。`Error` は `#[non_exhaustive]` のため、呼出側のパターンマッチには残りのケースを扱う分岐が必要です。材料番号・波長・光線ID・ファイルパスなどはフィールドで取得でき、表示文は日本語です。ルートクレートからは `raytrace_rs::error::{CoreError, FormatError, IoError, EngineError}` として参照できます。GUIやレポート内のエラー文字列には `error.to_string()` を使います。光源番号の `source_index` は既存の表示に合わせて1から数えます。

## ソースからビルドして起動

WindowsとRust 1.95以降（Cargo）が必要です。初回ビルド時に依存ライブラリを取得します。

```powershell
cargo build --release --locked --workspace --bins
./target/release/raytrace-gui.exe
```

ビルド後は `Raytraceを起動.cmd` からも起動できます。GUIは光源・レンズ・ロッド・スクリーンの組立サンプルで開きます。「ファイル → 開く…」またはドラッグ＆ドロップで `.zmx` や設計JSONを読み込み、「計算する」で追跡します。

GUIなしで使う場合は `cargo build --release --locked -p raytrace-cli` でビルドします。

```powershell
./target/release/raytrace-rs.exe analysis.trace model.path=examples/sequential_lens.json threads:=4 output=results/lens.json
cargo run --release --locked -p raytrace-cli -- analysis.trace model.path=examples/sequential_lens.json threads:=4 output=results/lens.json
cargo run --release --example assembly_demo -- results/assembly_demo.json
./target/release/raytrace-rs.exe analysis.trace model.path=results/assembly_demo.json threads:=4 output=results/assembly_trace.json
```

## GUI・Rustの基盤

GUIは egui / eframe 0.37.0、ファイル選択は rfd 0.17.2 を使います。既定の描画は wgpu です。WindowsではDX12とVulkanを有効にし、ソフトウェアアダプター（WARP）も選択対象に含めます。wgpuの初期化に失敗した場合は、標準エラーに1行出力してglowで起動し直します。`--renderer wgpu` または `--renderer glow` で描画方式を固定できます。固定した場合は自動切替を行いません。`--smoke 出力先` はどちらでも画面と状態を保存します。

全クレートはedition 2024です。`rust-version` は依存するegui / eframeの最低要件に合わせて1.95に設定し、CIも1.95.0で検証します。wgpu 30.0.1の要件は1.87、rfd 0.17.2には最低Rust版の指定がありません。

workspaceのlintで `unsafe_code` をdeny、`unused_qualifications`・Clippyの `dbg_macro`・`todo` をwarnにします。`unwrap_used`・`expect_used`・`panic` はdenyで、製品コードでは使えません（テストは `clippy.toml`、`tests/`・`examples/` は先頭の `#![allow]` で許可）。全クレートがこの設定を継承します。unsafeを許可するのは `raytrace-core/src/packet.rs` のAVX2カーネルだけです。各unsafeブロックには、実行時のCPU機能検出・配列範囲・非整列アクセスなどの成立条件を `// SAFETY:` に記載します。

依存関係は `cargo deny check` で検査できます。GUIを含む全workspaceの検査には次を使います。RustSecの脆弱性はエラー、保守停止・yank・重複バージョンは警告にします。許可ライセンスとフォントに限る例外は `deny.toml` に記載します。

```powershell
cargo install cargo-deny --version 0.20.2 --locked
cargo deny --workspace check
```

## 配布物の作成

配布物は3つのOSと、Claude Desktop用の拡張（MCPB）です。版は `Cargo.toml` の `[workspace.package] version` から取り、ファイル名に入ります。リリースごとに `CHANGELOG.md` を更新します。ベータの間は後方互換性を保ちません。

| 配布物 | スクリプト | ファイル名（版付き） |
|---|---|---|
| Windows ZIP | `scripts/package_windows.py`、確認は `scripts/check_portable.py` | `raytrace-rs-<版>-windows-x64.zip` |
| macOS | `scripts/package_macos.py` | `raytrace-rs-<版>-macos.tar.gz` |
| Linux（CLIのみ） | `scripts/package_linux.py` | `raytrace-rs-<版>-linux-x64.tar.gz` |
| Claude Desktop拡張 | `scripts/package_mcpb.py` | `raytrace-rs-<版>-windows.mcpb`・`raytrace-rs-<版>-macos.mcpb` |

スクリプトは標準ライブラリだけで動き、出力先は既定で `results/packages` です。ビルド済みの場合は `--no-build` を指定します。展開後のスモークテスト（バージョン、4サンプルの追跡、再計算、JSONリクエスト、PNG出力、MCPセッション）は `scripts/smoke.py` にまとめ、LinuxとmacOSの作成スクリプトとWindowsの `check_portable.py` が共有します。

### Windows配布ZIP

`python scripts/package_windows.py` で作成します。実行ファイル・サンプル・操作説明・変更履歴・ライセンスを同梱し、`package.json` に版とSHA-256を記録します。

```powershell
python scripts/package_windows.py
python scripts/check_portable.py results/packages/raytrace-rs-0.12.0-windows-x64.zip
```

### macOS版

`python3 scripts/package_macos.py` は、macOS上で `raytrace-rs`（CLI）と `raytrace-gui` を `aarch64-apple-darwin` と `x86_64-apple-darwin` の両方で `--release --locked` ビルドし、`lipo -create` でユニバーサルバイナリにまとめます（`lipo -info` で両方のアーキテクチャーを確認し、`codesign` でアドホック署名します）。最低のmacOSは11.0です（`MACOSX_DEPLOYMENT_TARGET`）。実行ファイル2つ・文書・変更履歴・サンプル・スキーマ・`THIRD_PARTY_LICENSES.txt`（GUIのフォントを含む）・`package.json` を `raytrace-rs-<版>-macos/` に入れて固めます。

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
python3 scripts/package_macos.py
```

作成後に空のフォルダーへ展開し、4サンプルの追跡・再計算・JSONリクエスト・PNG出力に加えて、`raytrace-rs mcp` のセッション（`initialize`、`doc.open`、`analysis.trace`、`analysis.budget`）を確認します。アプリケーションバンドル（`.app`）は作らず、署名と公証もしていないため、ブラウザーで取得したファイルには `xattr -dr com.apple.quarantine` が必要になることがあります。`objc2` 系のクレートは公開パッケージにライセンスファイルを含まないため、上流リポジトリの固定したコミットのMIT本文を `scripts/third_party_notices.toml` から入れます。上流にもライセンスファイルがない `dispatch 0.2.0` だけは、パッケージ情報（MIT、作者名）から作った表示を入れ、その旨を節に書きます。

### Claude Desktop拡張（MCPB）

`python3 scripts/package_mcpb.py PLATFORM BINARY OUTPUT_DIR` は、`PLATFORM`（`macos` または `windows`）用の `raytrace-rs` を [MCPB](https://github.com/modelcontextprotocol/mcpb)（`manifest_version` 0.3）にまとめます。ZIPの直下に `manifest.json`、`server/raytrace-rs`（Windowsは `server/raytrace-rs.exe`）、`LICENSE` を置き、サーバーは `raytrace-rs mcp` で起動します。`compatibility.platforms` は `darwin` または `win32` です。`--check-server` を付けると、実行ファイルを起動して `tools/list` がマニフェストの `tools` と一致するか確認します（そのOSで実行できる場合のみ）。Claude Desktopで `.mcpb` を開くと導入できます。マニフェストの検証は `npx -y @anthropic-ai/mcpb validate manifest.json` でできます。

### インストールスクリプトとチェックサム

利用者は配布物を手で展開する代わりに、リポジトリ直下のスクリプトで入れられます。管理者権限は要りません。

| スクリプト | 対象 | 動作 |
|---|---|---|
| `install.sh` | macOS・Linux x86_64（POSIX sh） | `uname` で資産を選び（`raytrace-rs-macos.tar.gz`・`raytrace-rs-linux-x64.tar.gz`）、`releases/latest/download/` から取得する。`RAYTRACE_VERSION=v0.12.0` なら `releases/download/<タグ>/` の版付きファイルを取得する。`.sha256` が取得できれば照合し、不一致なら何も入れずに失敗する。実行ファイルを `RAYTRACE_INSTALL_DIR`（既定 `~/.local/bin`）へ（macOSはGUIも）、サンプル・スキーマ・文書を `RAYTRACE_SHARE_DIR`（既定 `~/.local/share/raytrace-rs`）へ置き換えて入れる。macOSでは隔離属性を外す。`RAYTRACE_INSTALL_FROM=アーカイブ` はダウンロードを省く。シェルの設定ファイルは書き換えず、PATHに足す1行を表示する。`--uninstall`・`--help` に対応 |
| `install.ps1` | Windows PowerShell 5.1・PowerShell 7 | `raytrace-rs-windows-x64.zip`（`-Version v0.12.0` なら版付き）を取得して `.sha256` を照合し、`%LOCALAPPDATA%\Programs\raytrace-rs` へ置き換えて入れ、ユーザーのPATH（HKCU）に足す。`-From zip` はダウンロードを省き、`-Uninstall` でフォルダーとPATHの項目を消す。`Invoke-WebRequest`・`Expand-Archive`・`Get-FileHash` だけを使う |

どちらも最後に `raytrace-rs --version` を実行し、Claude Code・Claude Desktop・Codexの設定を絶対パスで表示します。メッセージは、PowerShell 5.1がBOMなしのファイルをANSIとして読むため、ASCIIの英語にしています。`install.sh` は `RAYTRACE_RELEASES_URL`、`install.ps1` は `-BaseUrl` で取得元を差し替えられます（テストとミラー用）。

`scripts/check_install.py PACKAGE` は、パッケージを一時フォルダーにインストールして使い、削除まで確認します。LinuxとmacOSは `install.sh`（`sh`）で次を行います。ヘルプと不正な引数、自分で作っていないフォルダーを置き換えない動作、ローカルのアーカイブからの導入、設定の出力（絶対パス）、導入したバイナリの `--version` と `smoke.py` のMCPセッション、再導入での置き換え、空白を含む導入先、ローカルのHTTPサーバーを使った固定名・タグ指定の取得とチェックサム（不一致の拒否、欠落時の警告）、削除後にファイルが残らないこと。Windowsは `pwsh` と `powershell`（5.1）の両方で `install.ps1 -From` を実行し、チェックサムの不一致、導入、PATH、MCPセッション、再導入、`-Uninstall` を確認します。ユーザーのPATH（レジストリ）は一時的に変わるため、終了時に元へ戻します。CIでは、Linuxは `package_linux.py` の後（`shellcheck` も実行）、macOSは `package_macos.py` の後、Windowsは `check_portable.py` の後に実行します。手元では次のように実行します。

```bash
python3 scripts/package_linux.py
python3 scripts/check_install.py results/packages/raytrace-rs-0.12.0-linux-x64.tar.gz
```

リリースの資産には、版付き・固定名の両方に、同名の `.sha256`（`<16進数>  <ファイル名>`）を添付します（`.github/workflows/release.yml`）。

### Linux版CLI

Linux版は静的リンクしたCLIだけを配布します。muslターゲットで依存ライブラリなしにビルドし、サンプル・スキーマ・文書・CLIの依存ライブラリのライセンスと一緒に `raytrace-rs-<版>-linux-x64.tar.gz` へまとめます。作成後の確認はmacOS版と同じです。

```bash
rustup target add x86_64-unknown-linux-musl
python3 scripts/package_linux.py
```

muslの静的リンク版は、2026-11-08の計測でglibc版と同じ結果を出し、追跡時間の差は4%以内でした。

### CIとリリース

`.github/workflows/windows.yml`（表示名 Checks）は、pushごとにWindowsの `test-and-package`（整形・Clippy・全テスト・Windows ZIPとMCPBの作成と確認）、macOSの `macos`（全テスト、macOS版とMCPBの作成と確認）、Linuxの `linux-core`（層の検査、WASM、Linux版の作成と確認）を実行します。Windows・macOS・Linuxのジョブはそれぞれ、パッケージの作成と確認の後にインストールスクリプトの確認（`check_install.py`）も行い、成果物を14日間保存します。GUIのテストは `egui::Context` をウィンドウなしで作るため、ディスプレイのないmacOSランナーでも動きます。

リリースを公開すると `.github/workflows/release.yml` が動きます（タグ名を指定した手動実行もできます。リリースがなければ、最初のジョブが `CHANGELOG.md` の該当版の節を本文にしてリリースを公開します。タグがなければ、実行したブランチの先頭に作ります）。WindowsとmacOSとLinuxのジョブが、タグと `Cargo.toml` の版の一致を確かめてからビルドと確認を行い、次のファイルと、それぞれの `.sha256` を `gh release upload --clobber` で添付します。版なしの固定名は `releases/latest/download/<固定名>` で最新版を取得するためのものです。リリースノートは `CHANGELOG.md` から作るか、手作業で用意します。ファイルの追加は、書き込み権限（`contents: write`）を持つこれらのジョブだけが行います。

| 版付き | 固定名 |
|---|---|
| `raytrace-rs-<版>-windows-x64.zip` | `raytrace-rs-windows-x64.zip` |
| `raytrace-rs-<版>-windows.mcpb` | `raytrace-rs-windows.mcpb` |
| `raytrace-rs-<版>-macos.tar.gz` | `raytrace-rs-macos.tar.gz` |
| `raytrace-rs-<版>-macos.mcpb` | `raytrace-rs-macos.mcpb` |
| `raytrace-rs-<版>-linux-x64.tar.gz` | `raytrace-rs-linux-x64.tar.gz` |

版を上げるときは、`Cargo.toml` の `[workspace.package]` と `[workspace.dependencies]` の内部クレートの版を揃え、`cargo update -w` で `Cargo.lock` を更新し、`CHANGELOG.md` を書いてから、`v<版>` のタグでリリースを公開します（`Release assets` をmainでタグ名を指定して手動実行すると、タグとリリースを作って公開します）。

## 自動テストと配布確認

GitHubへのpushとPull Requestで、Windows上の整形・Clippy・全テスト・解析解／ZMX往復保存・配布ZIPの作成と確認を自動実行します（macOSとLinuxは上の「CIとリリース」を参照）。配布確認はWindows標準ディレクトリだけをPATHに残し、4サンプル、ZMX読込、配置編集、保存と元ファイル削除後の再計算を実行し、MCPサーバーのセッションも確認します。STEPは変換キャッシュを空にし、外部Pythonの参照先も使用できない状態で、ZMXから初回読込・配置・追跡・保存・再生を確認します。別PCでのGUIの対話操作は別途確認します。

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

## 大量光線の速度比較

`throughput_check` は自作の単色レンズ、200波長のレンズ、非球面レンズ、256面の配置、分散を持つロッドを測ります。引数は `光線数 スレッド数 反復回数 ケース 計測対象` です。ケースは `lens`、`lens_200`、`asphere`、`array_256`、`rod`、`all`、計測対象は `explicit`、`streaming`、`both` を指定します。

`explicit` は準備済みの同一光線の追跡とレポート集計、`streaming` は光線生成から追跡・集計までを測ります。入力と出力、結果の照合は計測時間から外します。各対象を一度ウォームアップし、反復の中央値を保存します。両経路の全レポートが一致することも検査します。

```powershell
cargo build --release --locked --example throughput_check
./target/release/examples/throughput_check.exe 1048576 4 3 all both > results/throughput.json
```

改修前後で同一のベンチマークをビルドし、実行ファイルを別名で保存します。次のスクリプトは実行順を交互に変えて3組を測り、全画素、光量収支、面ごとの集計、反射回数、診断を含む全レポートと光線の署名が一致することを確認します。差があれば比較を失敗させます。

```powershell
python scripts/benchmark_compare.py --baseline results/before.exe --candidate results/after.exe --output results/comparison --rays 1048576 --threads 4 --repeats 3 --pairs 3
```

PコアとEコアが混在するPCなどで配置による差が大きい場合は、Windowsの `--affinity マスク` で両方の子プロセスを同じ論理CPU群に固定できます。マスクはそのPCのコア構成に合わせて選びます。例えば `--affinity 0x35` は論理CPU 0・2・4・5を指定します。スレッド数と、選んだ物理コア・SMTの対応も計測条件に記録してください。

Windowsではプロセスのピークワーキングセットと、定期取得したプライベートメモリの最大値も保存します。このメモリ値には、比較用の全入力光線と、時間外に行う光線の署名作成・結果照合も含みます。小分け生成だけを使うアプリケーションのメモリ量を評価するときは、比較用の一括生成を別に扱ってください。

計算核は光線の集計順を固定したまま、単色の屈折率検索、分岐管理用の領域、平面法線、吸収ゼロの区間、固定反射率・コーティングで上書きされるFresnel計算を効率化しています。逐次・非逐次の処理は追跡開始時に分け、逐次の次光線の状態をローカルに保持します。材料データは波長のビット列が一致したときだけ再利用し、キャッシュ上限を超えた波長も元の分散式・吸収式で評価します。

16面以上の非シーケンシャル系では、光学面全体のBVHで交点候補を絞ります。探索構造は各追跡呼出しの冒頭に一度作り、全ワーカーで共有します。その構築時間も上記の追跡時間に含まれます。各候補の交点計算には従来の形状ソルバーを使い、同距離の面の優先順と、手前にある数値計算失敗の診断を保持します。回帰テストは平面・球面・非球面・円すい側面・メッシュ、回転配置、開口端、遠方・浅い角度の入射、全終端光線・光路・画素・分岐順、キャッシュ上限を超える分散と吸収を確認します。

### シーケンシャルのSoA・SIMD経路

`raytrace-core/src/packet.rs` は4光線の位置・方向を成分別の配列（SoA）へまとめ、AVX2の倍精度命令で平面・球面・偶数次非球面の交点、法線、Snell則、Fresnel係数を計算します。非球面は探索区間を保持するNewton法と二分法を使い、各光線の収束を個別のマスクで管理します。分散式と波長別の吸収係数は既存の材料キャッシュから取得します。同じパケット内で波長が異なる場合も、それぞれの材料値を使います。

成功したパケットの交点と界面結果は、従来の追跡処理へ入力順に渡します。光量、画素、面別の集計、光学距離、終端、光路、診断と打切り条件は同じ処理を共有します。固定チャンクと合算順序も維持します。パケット用の作業領域はチャンク内で再利用し、光線数に比例する追加配列を作りません。

AVX2は実行時に検出します。対応しないCPU、非シーケンシャル、円すい側面・メッシュを含む逐次系は従来のループを使います。同一位置の界面、全反射、単調な交点を保証できない斜入射、交点探索に失敗したパケットも従来のソルバーで追跡します。失敗が4パケット続く区間はSoAへの変換を挟まず、まず256光線を1本ずつ処理してから再試行します。失敗が続くと再試行の間隔を1024、4096光線へ広げ、成功時に戻します。

SIMD命令は実行時の検出を通過した関数内で使用します。近似逆平方根や32ビット化を使わず、積和を融合しない加算・乗算で既存の演算順序を保ちます。内部テストはSIMDを無効にした参照経路と、回転・多波長・吸収・コーティング・逆向きの通過・ミラー・開口外・全反射・打切りを含む全レポートを照合します。

SoAとパケット追跡の背景は[Embreeの論文](https://embree.github.io/papers/2014-Siggraph-Embree.pdf)と[PBRTのデータ配置](https://www.pbr-book.org/4ed/Wavefront_Rendering_on_GPUs/Implementation_Foundations)を参照してください。

### 2026-10-25の計測

各ケース1,048,576光線、4スレッドで、変更前のRust計算核（`a7d0e11`）と最適化後（`efd1199`）を比較しました。Core i7-1255U / Windows 11 / rustc 1.98.1 MSVC、releaseのthin LTO・codegen-units=1を使用しています。Pコア2個とEコア2個から論理CPUを1個ずつ選び、マスク`0x35`へ固定しました。

両版の計算核を同一プロセスにリンクし、共通のRayonプール内で呼出し順を交互に変えた5回の中央値です。準備済み光線の追跡と、光線生成を含む追跡をそれぞれウォームアップしています。入力光線の全フィールドと、全画素・収支・面別集計・反射回数・診断を含むレポートは、両版でビット単位に一致しました。モデル準備、ファイル入出力、結果照合の時間は除外しています。

| 自作ケース | 追跡・集計の秒数：変更前 → 変更後 | 追跡の高速化 | 光線生成込みの高速化 |
|---|---:|---:|---:|
| 単色の球面レンズ | 0.504 → 0.382 | 1.32倍 | 1.30倍 |
| 200波長の球面レンズ | 0.386 → 0.326 | 1.18倍 | 1.15倍 |
| 非球面レンズ | 0.432 → 0.311 | 1.39倍 | 1.31倍 |
| 256面の格子配置 | 2.512 → 0.150 | 16.70倍 | 10.77倍 |
| ロッド | 10.252 → 5.349 | 1.92倍 | 1.87倍 |

256面のケースは吸収平面を格子状に置いた、境界箱の重なりが少ない配置です。境界箱が重なる配置では交点候補が増えます。実際のモデルの効果は、同じ入力・精度・スレッド数で上記の比較を行って確認してください。

### SoA・SIMDの計測（2026-10-25）

シーケンシャルの変更直前（`3e831b4`）と4光線のSIMD経路（`2089847`）を、各1,048,576光線・4スレッドで比較しました。Core i7-1255U / Windows 11 / rustc 1.98.1 MSVC、releaseのthin LTO・codegen-units=1を使用し、両版を同一プロセスとRayonプールで実行しています。論理CPUは`0x35`へ固定しました。各経路をウォームアップし、呼出し順を交互に変えた7回の中央値です。全入力光線と全レポートはビット単位に一致しました。

| 自作ケース | 追跡・集計の秒数：変更直前 → SIMD | 追跡の高速化 | 光線生成込みの高速化 |
|---|---:|---:|---:|
| 単色の球面レンズ | 0.384 → 0.342 | 1.12倍 | 1.16倍 |
| 200波長の球面レンズ | 0.393 → 0.348 | 1.13倍 | 1.07倍 |
| 非球面レンズ | 0.340 → 0.281 | 1.21倍 | 1.13倍 |

開口外が多い条件も、単色レンズの先頭面の開口半径を0.1 mmにした派生ケースで確認しました。この条件は11回の中央値で、追跡・集計が0.0365 → 0.0360秒、光線生成込みが0.1177 → 0.1163秒でした。失敗したパケットを繰り返し準備するコストは、上記の切替処理で抑えています。

時間はモデル準備・入出力・結果照合を除いた値です。SIMDの効果には、反復回数、波長分布、開口外へ向かう光線、集計処理と光線生成の割合が影響します。比較対象は今回同時に測り直した変更直前版です。

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

解析解との比較・Rust内のZMX往復保存と、OpticStudio実機との照合は別の検証です。生成したケースの実機照合状態は `not_run` から始まります。Pythonプローブは軸上Standard／Even Asphereの波長別・面別の交点と方向を比較し、許容差を超えると失敗します。光学距離の実機照合は別途必要です。Fresnel光量・NSC分岐・検出器マップの照合は次節のNSC比較器を使います。

### NSCの検出器照度・光量収支

`examples/nsc_reference.rs` と `scripts/opticstudio_nsc_probe.py` は、共通の `fixture.json` から両側のモデルを個別に構築します。NSCオブジェクトを相互にインポートする検証ではありません。SCHOTTのN-BK7は実行時にZemaxのGlasscatから読み、分散と内部透過率による吸収を使用します。カタログを別の場所に置く場合は `RAYTRACE_GLASSCAT` を指定してください。

```powershell
# 最初に少数光線で構築・画素方向を確認
cargo run --release --locked --example nsc_reference -- prepare results/nsc_smoke --rays 20000 --budget-rays 5000
uv run --no-project --python 3.12 --with pythonnet python scripts/opticstudio_nsc_probe.py --input results/nsc_smoke --rays 20000 --budget-rays 5000
cargo run --release --locked --example nsc_reference -- compare results/nsc_smoke

# 既定値：マップ100万光線、保存光線による収支10万光線
cargo run --release --locked --example nsc_reference -- prepare results/nsc_reference
uv run --no-project --python 3.12 --with pythonnet python scripts/opticstudio_nsc_probe.py --input results/nsc_reference
cargo run --release --locked --example nsc_reference -- compare results/nsc_reference
```

Python側の `--fixture NAME` で対象を限定できます。光線数を変える場合はRust側のprepareとPython側を同じ数で実行します。OpticStudioの接続・ライセンス障害は `status.json` の `failure_kind` に記録します。トレースには時間上限を設け、例外時もツールとアプリケーションを閉じます。

| fixture | 条件 | モード |
| --- | --- | --- |
| diode_direct | 550 nm、X/Y発散8°/20°、X偏心+0.5 mm、50 mm先の検出器 | geometric |
| lens_focus | N-BK7両凸レンズ、R=±50 mm、厚さ6 mm、半径10 mm | geometric 450/550/650 nm、fresnel 550 nm |
| rod_homogenizer | 4×4×40 mmのN-BK7ロッド、X/Y発散10°/25° | geometric/fresnel、出口と20 mm後方 |
| tilted_window | X軸まわり30°傾斜、厚さ3 mmのN-BK7平行平板 | fresnel |

検出器は64×64画素の吸収型です。ロッド出口と後方の検出器は別トレースで測定します。各画素の値は積分光量で、照度は画素面積で割った値です。配列はXを先に進める行優先で、最初の画素は負のX/Y側です。OpticStudioの既定Pixel Interpolationは隣接画素へ光量を配分するため、比較では明示的にOFFにします。Standard Lensのclearとedgeの半径は同一とし、Rust側は球面2面とリムを結ぶ側面で構築します。Rectangular Volumeはassemblyの6平面ロッドで構築します。オブジェクトの回転はZemaxのintrinsic XYZに合わせます。

geometricではRustの反射率を0とし、OpticStudioの偏光・分岐を無効にします。屈折と全反射は残ります。ただし実機2026 R1.02では偏光OFFにするとバルク吸収も無効になるため、RustもgeometricではAGFの内部吸収を0にします。geometricは形状・分散・全反射・画素集計の照合に限り、内部吸収はfresnelで照合します。fresnelでは偏光・分岐を有効にし、ランダム偏光、散乱なし、IgnoreErrors=True、相対強度閾値1e-7、絶対強度閾値0、交点数128、セグメント数4096を設定します。IgnoreErrors=Falseでは微小な交点誤差で全トレースが途中終了してもツール成功となる場合があるため、Trueで続行し、DeadRayErrorsとZRDのエラーを光量収支に記録します。温度20℃・圧力1 atmで環境による屈折率補正を無効にし、カタログの空気に対する相対屈折率を使用します。Rustも媒質0をn=1とし、同じ相対屈折率を使用します。

Rustは各界面でS/P反射率を独立に平均します。OpticStudioは偏光状態を追跡するため、複数界面を通過した光量に差が生じます。傾斜平板については独立平均とS/P保存の両方の解析値を `material_info.json` に残します。550 nm・30°平板で両モデルの透過率差は約0.000508であり、光量許容差の絶対項0.001以内です。このモデル差を統計誤差として隠さず、比較結果と併記します。

比較器は独立な疑似乱数seed A/Bを使用します。正規化マップL1の許容差は `1.5 * L1(A,B) + 0.005`、重心・RMS幅はseed差から求めた3σと半画素の大きい方、光量比は二項近似とseed差の大きい方の3σに絶対差0.001を加えます。重心とRMSは画素中心から算出します。ピーク画素も記録します。`opticstudio.json` がないfixtureは `not_run`、比較した指標が許容差を超えた場合は終了コード1です。

各fixtureに `rust_seed_A.json`、`rust_seed_B.json`、`opticstudio.json`、`comparison.json` を保存し、全体は `summary.json` に集計します。Rustの全Budgetと投入光量、画素寸法、API設定の読み戻し、実機版、所要時間を残します。保存ZRDはZOS-API readerで読み、分類根拠と未対応理由を `opticstudio.json` に記録します。detectedは吸収検出器の総光量とZRD終端和、escapedはRayErrorなし・SegTerminatedかつHitObject=0のセグメント、branch_cutoffはDeadRayThresholdに対応します。無コーティング・散乱なし・エラーなし・未知終端なしの場合に限り、投入光量から検出・脱出・閾値損失を引いた残差をbulk_absorbedとします。surface_absorbedは無コーティング誘電体で0です。RayErrorとDeadRayErrorsがともに0の場合はerrorsとinteraction_limitを0として比較できます。非0の場合は両者を区別できないため未比較とします。傾斜板では透過・反射脱出・バルク吸収を多重反射込みの解析解とも比較します。界面で両子枝が閾値以下になると入射セグメントが終端となる場合があるため、2枝分の閾値以内の微小終端はDeadRayThresholdの内訳として扱います。それを超える未知終端があればbulk_absorbedは未比較とします。対応できない収支項目は `not_compared` であり、合格を意味しません。生成ZMX、ZRD、AGFを含む成果物はgit管理対象外の `results/` に置きます。

## リポジトリ構成

Cargo workspaceとして、`crates/` 以下の8クレートに分けています。

- `raytrace-core`: 数学・形状・光源生成・光線追跡・コーティング・分散・スクリーン解析の計算核です。ファイル形式やGUIには依存しません。
- `raytrace-formats`: ZMX／AGF／SPCD・STEPの入出力とCADキャッシュの保存先を扱います。
- `raytrace-doc`: 設計文書（L2）です。部品（`Part`）と配置（`Occurrence`）の木、安定したID、展開（`Design::compile`）、組立の編集補助、互換性レポートを持ちます。ファイル・時刻・環境に触れず、外部形式を知りません（元のZMXなどは `Provenance` に不透明なJSONとして保管します）。
- `raytrace-io`: 設計文書のファイル形式（`design`：開く・保存）、`record`、ZMX書出し、設定の保存・スクリーン出力を扱います。従来のZMX・CAD入出力のパスもここから使えます。
- `raytrace-engine`: セッション（`session`）、コマンド登録（`command`・`commands`）、計算パイプライン（`pipeline`）、パラメーター走査・収束確認・検証ケースのワークフローを扱います。旧 `raytrace-app` です。
- `raytrace-mcp`: MCPサーバー（L5）です。標準入出力のJSON-RPC 2.0（1行1メッセージ）を外部ライブラリなしで処理し、`list_commands`・`describe_command`・`execute`・`batch` の4つのツールでセッションのコマンドを呼びます。呼び出しは作業スレッドが順に実行し、`notifications/cancelled` で `TraceControl` を止め、`progressToken` があれば進捗を通知します。`raytrace-rs mcp` から起動します。
- `raytrace-cli`: コマンドライン用の `raytrace-rs` 実行ファイルです。引数をコマンドのパラメーターへ変換し、応答をJSONで出力します。PNG出力も有効にします。
- `raytrace-gui`: デスクトップ用の `raytrace-gui` 実行ファイルです。PNG出力も有効にします。

依存方向は `core ← {formats, doc} ← io ← engine ← {mcp, cli, gui}` で、各層は必要な下位層を参照します。リポジトリ直下の `raytrace-rs` はcore・doc・io・engineのモジュールを再公開する窓口で、既存の `raytrace_rs::model` などのパスを使えます。`tests/` と `examples/` も直下に残しています。

`raytrace-formats` と `raytrace-doc` は互いを知りません。formatsはZMXの読込結果と書出し用の参照を形式側の型で受け渡し、docは元のZMXを不透明な `Provenance` として持ちます。`raytrace-io` がこの2つを接続し（ZMXを1部品の設計に変換し、書出しで型に戻す）、ファイル形式と設計文書の相互参照を避けています。公開順もcore・formats・doc・io・engineの順です。

### セッションとコマンド

計算とファイル操作の入口は `raytrace_engine` の `Session` とコマンド登録（`command::registry()`）です。コマンドは `fn(&mut Session, In, &mut Context) -> Result<Out, EngineError>` の関数で、`In` は `Deserialize + JsonSchema`（`deny_unknown_fields`）、`Out` は `Serialize + JsonSchema` です。IDは `<領域>.<動作>`（`analysis.trace`、`doc.set` など）で、`commands.rs` の `all()` に `command(ハンドル, 要約, 種類, 関数)` を足すと、CLI・`command.list`・`command.describe`・スキーマ（`request`・`response`）に現れます。種類は `Query`（文書を変えない）、`Mutation`（文書を変える）、`Job`（長い計算）です。

`Session` は開いている文書（`Arc<Design>`）、ファイルの場所、版番号、取り消し・やり直しの履歴、AGFカタログを持ちます。文書は書き換えず、変更のたびに新しい `Design` を作って検証（`design::check` ＝ 展開）してから確定し、古い文書を履歴に積み、版番号を1増やします。失敗した変更は文書も履歴も版番号も変えません。変更コマンドは `expected_revision` を受け付け、一致しなければ `revision` エラーにします。呼び出しは次の2通りで、同じ関数を実行します。

- `Session::execute(id, params, &mut Context)`: パラメーターをJSONで渡し、応答の封筒 `Response`（`ok`・`command`・`revision`・`result`・`error`）を返します。CLIと `run` が使います。`execute_request` は `{"command": ID, ...}` を、`execute_text` はそのテキストを受けます。
- `Session::call(ハンドル, 入力, &mut Context)` / `Session::run(ハンドル, 入力)`: 型付きの入力を渡し、型付きの結果を返します。JSONを経由しないため、GUIが使います。ハンドルは `commands::ANALYSIS_TRACE` のような `Cmd<In, Out>` で、入出力の型の取り違えをコンパイル時に防ぎます。

文書を使うコマンドは `model`（`path`・`design`・`scene`）を省略でき、省略するとセッションの文書、指定するとそのモデルだけを扱います（セッションは変わりません）。`Context` は中止と光線進捗の `TraceControl`、走査の進捗の `Progress` のコールバックを運びます。コマンドはドット区切りのIDだけで呼べ、前の短い名前（別名）は持ちません。

CLIの `--connect 接続ファイル` は共有するGUIセッションの選択です。操作ごとの独自のオプションはありません。`raytrace-rs COMMAND a.b=文字列 a.b:=JSON` の引数をコマンドのパラメーターとして組み立て、`raytrace-rs run` はリクエストJSONを1つのセッションで続けて実行します。どちらも同じ検証を通り、`--help` と `COMMAND --help` は `command.list`・`command.describe` のスキーマから生成します。そのため、CLIで使える操作とフィールドはコマンドの入力型だけで決まり、引数の処理を個別に保守する必要がありません。GUIはUIスレッドの `Session` で編集します。外部接続の輸送は `raytrace_engine::remote` のネイティブ専用部分にあり、`Pending::prepare` を作業スレッドで実行し、`Prepared::commit` をUIスレッドで確定します。`Session::generation` は文書の版だけでなく保存・計算結果の変更も検出し、競合・中止では準備した状態を受け取りません。GUIから行った計算も同じセッションの直近の計算記録になります。

モデル・計算記録・計画・応答の型は `schemars` でJSON Schemaを導出します。下位クレートでは `schema` 機能で有効にし、`raytrace-engine` は常に有効です。スキーマは `schemas/` にコミットし、型を変えたら次のコマンドで更新します。`tests/engine.rs` が古いスキーマを検出します。各クレートの `Error::code()` は応答の `error.code` に使う固定の識別子です。

```powershell
cargo run -q --locked -p raytrace-cli -- schema.get output=schemas
```

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

## panicの防止とパーサのファジング

`cargo xtask no-panic` は製品のモジュールを構文解析し、assert系マクロ（debug版を含む）を拒否します。テスト専用モジュールと `#[test]` 関数は対象外です。`unwrap`・`expect`・`panic!`・`unreachable!` はworkspaceのClippy lintでもdenyします。これらは添字アクセスやメモリ枯渇を含むすべてのpanicを静的に保証するものではないため、不正入力の回帰テストとファジングも併用します。

[パーサのファジング](../fuzz/README.md)は独立workspaceで、ZMX・AGF・STEPの実際の読込APIを検査します。専用CIは正常に読める自作の種を確認し、AddressSanitizer付きで各ターゲットを30秒実行します。失敗入力はCIのartifactに保存します。長時間実行と再現の手順も同文書にあります。
