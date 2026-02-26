# 変更履歴

- UPDATE
  - 後方互換がある変更
- ADD
  - 後方互換がある追加
- CHANGE
  - 後方互換のない変更
- FIX
  - バグ修正

## develop

- [CHANGE] `hisui/crates/shiguredo_nvcodec/` から `shiguredo/nvcodec-rs` に変更する
  - crates.io はそのまま
  - @voluntas
- [CHANGE] `libloading` クレートへの依存を廃止し、独自の動的ライブラリローダー `dl::DynLib` モジュールに置き換える
  - @voluntas
- [CHANGE] ビルド依存の `toml` クレートを `shiguredo_toml` に置き換える
  - @voluntas
- [ADD] エンコーダのケーパビリティクエリ機能を追加する
  - `EncoderCaps` 構造体と `query_caps_h264` / `query_caps_h265` / `query_caps_av1` メソッドを追加
  - @voluntas
- [ADD] デコーダのケーパビリティクエリ機能を追加する
  - `DecoderCaps` 構造体と `query_caps_h264` / `query_caps_h265` / `query_caps_av1` / `query_caps_vp8` / `query_caps_vp9` / `query_caps_jpeg` メソッドを追加
  - @voluntas
- [ADD] エンコーダの動的再設定機能を追加する
  - `ReconfigureParams` 構造体と `reconfigure` メソッドを追加
  - @voluntas
- [ADD] JPEG デコーダを追加する
  - `Decoder::new_jpeg` メソッドを追加
  - @voluntas
- [ADD] CUDA デバイス列挙関数を追加する
  - `device_count()` および `device_name()` 関数を追加
  - @voluntas
- [ADD] CUDA ストリーム管理機能を追加する
  - `CudaStream` 構造体を追加
  - @voluntas
- [ADD] 2D メモリコピー機能を追加する
  - `memcpy_2d()` および `mem_alloc_pitch()` 関数を追加
  - @voluntas
- [UPDATE] CUDA インクルードパスの解決をフォールバック付きの 3 段階方式に改善する
  - 環境変数 → デフォルトパス → スタブヘッダの順で探索する
  - @voluntas
- [UPDATE] docs.rs 向けスタブ生成を包括的な型定義に書き直す
  - @voluntas

## 2025.2.2

- [UPDATE] エラーメッセージを改善する
  - CUDA および NVENC のエラーコードに対応する詳細情報を表示するようにする
  - @sile

## 2025.2.1

**リリース日**: 2025-10-21

- [FIX] ビルドに必要なヘッダファイルを含んだ third_party/ ディレクトリを crate 内に移動する
  - 今までは hisui リポジトリのルートに配置していたが、これだと shiguredo_nvcodec の crates.io への publish 時に third_party/ がパッケージに含まれない
  - そのため cargo 経由でビルドする際に必要なファイルが見つからずに失敗してしまっていた
  - third_party/ ディレクトリを hisui/crates/shiguredo_nvcodec/ 以下に移動することで、crates.io に登録したパッケージにもこのディレクトリが含まれるようにした
  - @sile
