# AIから使う

Claude Code・Claude Desktop・Codexに、raytrace-rsで光学計算をしてもらうための手順です。日本語で頼むだけで、AIが光学系を開き、編集し、光線を追跡して、結果を説明します。操作の詳細は[操作ガイド](GUIDE.md)、できること・できないことは[機能と対応範囲](FEATURES.md)にあります。

> ベータ版（0.12.0）です。コマンドやファイル形式は予告なく変わることがあります。

## できること

`raytrace-rs mcp` はMCPサーバーです。AIはこのサーバー経由で、GUIやCLIと同じコマンドを使えます。起動中のGUIの光学系を操作する場合は、GUIを `--automation-session 接続ファイル` 付きで起動し、MCPの起動引数を `mcp --connect 接続ファイルの絶対パス` にします。[GUIへの接続](GUIDE.md#開いているguiへ接続する)に設定例と競合時の扱いがあります。

- ZMXや設計JSONを開き、計算できるか（互換性）を確認する
- 光源・レンズ・ロッド・スクリーンを追加し、位置や寸法を変える（取り消しできる）
- 光線追跡を実行し、スクリーンの光量分布・半値幅・光量収支を調べる
- 位置・寸法・波長を変えた走査で、結果を比べる

幾何光学（光線追跡）だけを扱います。偏光・回折・MTF・自動最適化はできません。長さはmm、波長はμm（マイクロメートル）です。

## インストール

macOS（Apple Silicon・Intel）とLinux：

```bash
curl -fsSL https://raw.githubusercontent.com/Chinanago/raytrace-rs/main/install.sh | sh
```

Windows（PowerShell）：

```powershell
irm https://raw.githubusercontent.com/Chinanago/raytrace-rs/main/install.ps1 | iex
```

管理者権限もsudoも要りません。インストーラーは最新のリリースを取得し、SHA-256を確かめて、次の場所に入れます。

| OS | 実行ファイル | サンプル・スキーマ・文書 |
|---|---|---|
| macOS・Linux | `~/.local/bin/raytrace-rs`（macOSは `raytrace-gui` も） | `~/.local/share/raytrace-rs` |
| Windows | `%LOCALAPPDATA%\Programs\raytrace-rs`（`raytrace-gui.exe` も同じ場所） | 同じフォルダー |

終わるとインストール先の絶対パスを使った設定をそのまま表示します。上のURLはリポジトリの `main` を指すため、取得できないときは次の方法を使ってください。

**Releasesからダウンロードする場合**：[最新のリリース](https://github.com/Chinanago/raytrace-rs/releases/latest)から、使うOSのファイルを取得します。

| ファイル | 内容 |
|---|---|
| `raytrace-rs-windows-x64.zip` | Windows：GUI・CLI・MCPサーバー・サンプル |
| `raytrace-rs-macos.tar.gz` | macOS：同上（ユニバーサルバイナリ） |
| `raytrace-rs-linux-x64.tar.gz` | Linux：CLIとMCPサーバー |
| `raytrace-rs-windows.mcpb` / `raytrace-rs-macos.mcpb` | Claude Desktop用の拡張 |

各ファイルには `.sha256` が付いています。展開後の `raytrace-rs`（Windowsは `raytrace-rs.exe`）が実行ファイルです。

**macOSで開けないとき**：署名・公証をしていないため、ブラウザーで取得したファイルは隔離属性が付き、開けないことがあります。`xattr -dr com.apple.quarantine <展開したフォルダー>` で外します。インストールスクリプトは自動で外します。

**更新**：同じコマンドをもう一度実行すると、最新版に置き換わります。版を指定するときは、macOSは `curl -fsSL <URL> | RAYTRACE_VERSION=v0.12.0 sh`、Windowsは `$env:RAYTRACE_VERSION='v0.12.0'` を先に実行します。Windowsでは、Claude Desktop・Claude Code・Codexなどを終了してから更新してください（使用中のファイルは置き換えられません）。

**削除**：macOSは `curl -fsSL <URL> | sh -s -- --uninstall`、Windowsは `& ([scriptblock]::Create((irm <URL>))) -Uninstall` を実行します。AI側の登録は、Claude Codeなら `claude mcp remove raytrace-rs`、ほかは設定を消します。

## Claude Code

どちらか1つを選びます。

**プラグイン**（スキルとMCPサーバーの両方が入ります。`raytrace-rs` がPATHにある必要があります）：

```text
/plugin install raytrace-rs --marketplace Chinanago/raytrace-rs
```

古いClaude Codeでは `/plugin marketplace add Chinanago/raytrace-rs` の後に `/plugin install raytrace-rs@raytrace-rs` を実行します。

**`claude mcp add`**（絶対パスで登録するので、PATHは要りません）：

```bash
claude mcp add --scope user raytrace-rs -- ~/.local/bin/raytrace-rs mcp
```

Windowsは `claude mcp add --scope user raytrace-rs -- "%LOCALAPPDATA%\Programs\raytrace-rs\raytrace-rs.exe" mcp`（PowerShellでは `"$env:LOCALAPPDATA\Programs\raytrace-rs\raytrace-rs.exe"`）です。`--scope user` を付けると、どのプロジェクトでも使えます。

確認：Claude Code内で `/mcp` を実行し、`raytrace-rs` が connected になっていれば使えます。

## Claude Desktop

**拡張（`.mcpb`）がおすすめです。** パスや設定ファイルを触る必要がありません。

1. [Releases](https://github.com/Chinanago/raytrace-rs/releases/latest)から `raytrace-rs-windows.mcpb`（Windows）または `raytrace-rs-macos.mcpb`（macOS）を取得します。
2. ファイルを開く（ダブルクリック）と、Claude Desktopにインストールの確認が出ます。または Settings > Extensions にドラッグします。
3. インストールして有効にします。

拡張には実行ファイルが入っていますが、サンプルは入っていません。サンプルが必要なら上の方法でインストールするか、zip・tar.gzを展開してください。

**設定ファイルで登録する場合**：Claude Desktopはシェルの `PATH` を引き継がないため、絶対パスが必要です。Settings > Developer > Edit Config で次のファイルを開き、`mcpServers` に足します。

- macOS：`~/Library/Application Support/Claude/claude_desktop_config.json`
- Windows：`%APPDATA%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "raytrace-rs": {
      "command": "/Users/あなた/.local/bin/raytrace-rs",
      "args": ["mcp"]
    }
  }
}
```

Windowsではパスの `\` を `\\` と2つ書きます：`"C:\\Users\\あなた\\AppData\\Local\\Programs\\raytrace-rs\\raytrace-rs.exe"`。インストールスクリプトが正しい形で表示するので、それをコピーするのが確実です。

**Claude Desktopを完全に終了して起動し直します**（ウィンドウを閉じるだけでは足りません）。チャットの入力欄の設定アイコンやコネクターの一覧に `raytrace-rs` のツール（`list_commands`・`describe_command`・`execute`・`batch`）が出れば成功です。

## Codex

`~/.codex/config.toml`（Windowsは `%USERPROFILE%\.codex\config.toml`）に書きます。

```toml
[mcp_servers.raytrace-rs]
command = "/Users/あなた/.local/bin/raytrace-rs"
args = ["mcp"]
```

Windowsは `command = 'C:\Users\あなた\AppData\Local\Programs\raytrace-rs\raytrace-rs.exe'` のように、シングルクォートで囲むとバックスラッシュをそのまま書けます。Codexを起動し直し、`/mcp` で `raytrace-rs` が見えれば使えます。

スキル（使い方の手引き）も入れるとより確実です。Codexに次のように頼みます。使うときは `$raytrace-rs` で呼び出します。

```text
$skill-installer で Chinanago/raytrace-rs の skills/raytrace-rs をインストールして。
```

## 最初に試す頼み方

AIにそのまま日本語で頼めます。サンプルを使うときは、インストールで表示された場所（`~/.local/share/raytrace-rs/examples/`、Windowsはインストール先の `examples\`）の絶対パスを添えてください。

- 「`examples/integrator_rod.json` を開いて追跡し、スクリーンの半値幅と光量収支を教えて」
- 「このZMXを読み込んで計算できるか確認して」（ファイルの場所を伝えます）
- 「ロッドの長さを20〜40 mmで走査して受光率を比べて」
- 「レンズを1 mm光軸方向にずらして、スクリーンの分布がどう変わるか見て。気に入らなければ元に戻して」
- 「光量収支の誤差と、未解決の光量を教えて」

結果は、基準面（発射した光量）を100%とした割合で返ります。光量収支の誤差が小さいこと、未解決分が小さいことを、結果を信じる前に確かめるようAIに頼むとよいです。編集すると結果は古くなる（`stale`）ので、編集後は追跡し直します。

## 保存先・単位・メッセージ

- 長さはmm、波長はμmです（GUIの波長入力だけnm）。
- 保存先を相対パスで書くと、MCPサーバーを起動した場所が基準になります。Claude Codeは起動したフォルダー、Claude Desktopは決まっていません。**保存先は `~/Documents/raytrace/` のように絶対パスで頼む**のが確実です。
- 計算記録・走査・スクリーン画像（PNG・CSV）は、頼んだ場所に書き込まれます。AIが使うファイルの読み書きは、あなたの権限で行われます。
- エラー文は日本語です。そのままAIに見せれば、原因を説明して直してくれます。
- 設定やキャッシュは、macOSは `~/Library/Application Support/Raytrace`、Windowsは `%LOCALAPPDATA%\Raytrace` にあります。

## うまくいかないとき

| 症状 | 対処 |
|---|---|
| サーバーが見つからない・`ENOENT`・connectedにならない | `raytrace-rs` がPATHにありません。設定のコマンドを**絶対パス**にします。ターミナルで `/絶対パス/raytrace-rs --version` が動くか確認します。 |
| Claude Desktopに出てこない | 設定ファイルのJSON（カンマ・`\\`）を確認し、Claude Desktopを完全に終了して再起動します。 |
| macOSで「開けません」「破損している」 | `xattr -dr com.apple.quarantine <フォルダーまたはファイル>`。インストールスクリプトを使えば自動で外れます。 |
| Windowsで「WindowsによってPCが保護されました」 | 「詳細情報」→「実行」。実行ファイルは署名していません。 |
| `.ps1` が実行できない | `irm ... \| iex` を使うか、`powershell -ExecutionPolicy Bypass -File install.ps1` を実行します。 |
| 更新・削除で置き換えられない | Claude Desktop・Claude Code・Codexを終了してからやり直します。 |
| ZMXが追跡されない | `doc.inspect` の互換性レポートを見ます。`requires_override`・`unsupported` は[機能と対応範囲](FEATURES.md)の条件が必要です。 |

ログ：Claude Desktopは、macOSが `~/Library/Logs/Claude/`、Windowsが `%APPDATA%\Claude\logs\` の `mcp.log` と `mcp-server-*.log` です。Claude Codeは `claude --debug` で起動します。単体の動作確認は、ターミナルで `raytrace-rs --version` と、`echo '{"command":"app.version"}' | raytrace-rs run -` を実行します。

不具合や要望は [Issues](https://github.com/Chinanago/raytrace-rs/issues) へお寄せください。MCPサーバーの仕様（ツール、進捗通知、中止）は[操作ガイド](GUIDE.md#aiエージェントmcp)にあります。
