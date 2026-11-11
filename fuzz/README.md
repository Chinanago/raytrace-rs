# パーサのファジング

ZMX、AGF、STEPを、製品と同じファイル読込APIで検査します。入力は一時ディレクトリに置き、STEP変換キャッシュを使いません。パーサが返す通常のエラーは許容し、panic・AddressSanitizerの検出・時間超過・メモリ上限超過は失敗です。panicを捕まえて成功に変換しません。

開発用の独立したCargo workspaceです。製品の依存やRust 1.95の要件にはcargo-fuzzを追加しません。

```sh
rustup toolchain install 1.95.0 --profile minimal
rustup toolchain install nightly-2026-11-11 --profile minimal
cargo +1.95.0 install cargo-fuzz --version 0.13.1 --locked
cargo +1.95.0 test --manifest-path fuzz/Cargo.toml --locked --test seeds
cargo +nightly-2026-11-11 fuzz build
for target in zmx agf step; do
  cargo +nightly-2026-11-11 fuzz run "$target" -- \
    -max_total_time=30 -timeout=10 -rss_limit_mb=2048 -max_len=16384
done
```

リポジトリ直下で実行します。CIも同じ3ターゲットを各30秒実行します。長時間調べる場合は `-max_total_time` を増やしてください。有限時間の実行は、すべての入力で安全である証明にはなりません。

`corpus/` のコミット対象は手書きの平面ZMX、一定屈折率AGF、`tests/support/step.rs` の `box_step(false)` が作る直方体STEPだけです。正常に読み込めることを `tests/seeds.rs` で検査します。AGFは `FUZZ` カタログ内の `FUZZ` 材料を読み込み、係数から材料への変換も実行します。

生成されたコーパスと失敗入力（`artifacts/`）はGit管理外です。失敗入力を再現するときは、例えば次のように実行します。

```sh
cargo +nightly-2026-11-11 fuzz run step fuzz/artifacts/step/crash-INPUT
```

原因を修正したら、自作の最小入力を通常の回帰テストにも追加します。利用者の光学データをコーパスやコミットへ追加しないでください。
