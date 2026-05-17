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

- [UPDATE] `EncoderState` の CUDA コンテキスト管理を `with_context` パターンに統一する
  - `encode_frame`, `lock_and_copy_bitstream`, `send_eos`, `unmap_resource` でコンテキスト管理を自己完結させる
  - `run_worker` と `drain_one_with_ctx` から手動の `cuCtxPushCurrent` / `cuCtxPopCurrent` を除去する
  - `drain_one_with_ctx` を `drain_one` にリネームする
  - これにより `.expect()` パニックの温床が消滅し、issue 0016 も自然解決する
  - @melpon

- [FIX] `encode_frame_inner` で mapped resource に RAII ガード（ReleaseGuard）を導入する
  - エンコード失敗時に ReleaseGuard の drop で自動的に unmap されるようにする
  - 成功時は cancel() でガードを解除し、後続の drain が unmap を担当する
  - @melpon
- [ADD] `EncodedFrame<T>` 構造体を追加する
  - `user_data: T` フィールドを持ち、エンコード完了時に任意のユーザーデータを callback 経由で受け取れるようにする
  - `into_parts()` メソッドでデータとユーザーデータに分解できる
  - `user_data()` メソッドでユーザーデータの参照を取得できる
  - @melpon

  - `into_parts()` メソッドでデータとユーザーデータに分解できる
  - `user_data()` メソッドでユーザーデータの参照を取得できる
  - @melpon

  - `Encoder::new()` に完了用のコールバックを渡すようにする
  - `Encoder::next_frame()` は廃止
  - @melpon

  - `Decoder::new()` に完了用のコールバックを渡すようにする
  - `Decoder::next_frame()` は廃止
  - @melpon

  - `query_encoder_caps()` 及び `query_decoder_caps()` にリネーム
  - @melpon

  - `reconfigure_inner` で `self.pitch` とバッファプール（`device_inputs` / `registered_resources` / `bitstream_buffers`）を解像度変更時に更新する
  - `run_worker` の `Job::Reconfigure` 分岐で in-flight フレームを全て drain してから reconfigure を実行する
  - `cleanup_buffer_pool` を idempotent 化（再呼び出し時にパニックしない）
  - `Encoder::reconfigure` の doc コメントに解像度変更後の IDR フレーム送出要件を追記する
  - @melpon

  - Ok パスから `.expect()` を除去し、`pending_user_data` が空の場合はエラーコールバックで通知する
  - Err パスで `pending_user_data` の状態にかかわらず必ずエラーを通知し、残存 user_data を破棄する
  - @melpon


- [UPDATE] `Encoder<T>::reconfigure` と `Encoder<T>::get_sequence_params` のレシーバを `&mut self` から `&self` に変更する
  - 内部で `job_tx.send()` のみを使用するため可変借用は不要
  - @melpon

- [UPDATE] `EncoderState::expected_frame_size` フィールドを削除する
  - フィールドは一度も読み取られることなく、毎回再計算されていた
  - @melpon

### misc

- `EncoderState` と `DecoderState` の内部メソッドから不要な `pub` を削除する
  - @melpon
## 2026.1.0

**リリース日**: 2026-03-31

- [CHANGE] `hisui/crates/shiguredo_nvcodec/` から `shiguredo/nvcodec-rs` に変更する
  - crates.io はそのまま
  - @voluntas
- [CHANGE] `libloading` クレートへの依存を廃止し、独自の動的ライブラリローダー `dl::DynLib` モジュールに置き換える
  - @voluntas
- [CHANGE] ビルド依存の `toml` クレートを `shiguredo_toml` に置き換える
  - @voluntas
- [CHANGE] `Encoder::new_h264` / `Encoder::new_h265` / `Encoder::new_av1` を廃止し `Encoder::new` に統合する
  - `EncoderConfig` に `codec: CodecConfig` フィールドを追加し、コーデック種別とコーデック固有設定を一体化する
  - @voluntas
- [CHANGE] `Profile` 構造体を廃止し、コーデック固有のプロファイル enum に置き換える
  - `H264Profile` / `HevcProfile` / `Av1Profile` を追加する
  - @voluntas
- [CHANGE] `EncoderConfig` から `profile` と `idr_period` フィールドを削除する
  - コーデック固有設定構造体 (`H264EncoderConfig` / `HevcEncoderConfig` / `Av1EncoderConfig`) に移動する
  - @voluntas
- [CHANGE] `EncoderConfig` / `H264EncoderConfig` / `HevcEncoderConfig` / `Av1EncoderConfig` / `DecoderConfig` から `Default` 実装を削除する
  - NVENC / NVDEC SDK にデフォルト値の概念がないため、全フィールドを明示的に指定する設計にする
  - @voluntas
- [CHANGE] `Decoder::new_h264` / `Decoder::new_h265` / `Decoder::new_av1` を廃止し `Decoder::new` に統合する
  - `DecoderConfig` に `codec: DecoderCodec` フィールドを追加する
  - @voluntas
- [CHANGE] `Encoder::encode()` に `EncodeOptions` 引数を追加する
  - `force_intra` / `force_idr` / `output_spspps` フラグでフレーム単位のエンコード制御が可能になる
  - @voluntas
- [CHANGE] `EncoderConfig` に `buffer_format: BufferFormat` フィールドを追加する
  - NV12 ハードコードを廃止し、NVENC SDK がサポートする入力バッファフォーマットを選択可能にする
  - @voluntas
- [CHANGE] `DecoderConfig` に `surface_format: SurfaceFormat` フィールドを追加する
  - NV12 ハードコードを廃止し、NVDEC SDK がサポートする出力サーフェスフォーマットを選択可能にする
  - @voluntas
- [ADD] エンコーダーの動的解像度変更に対応する
  - `ReconfigureParams` に `width` / `height` フィールドを追加する
  - `maxEncodeWidth` / `maxEncodeHeight` を超える場合はエラーを返す
  - @voluntas
- [ADD] エンコーダのケーパビリティクエリ機能を追加する
  - `EncoderCaps` 構造体と `Encoder::query_caps` メソッドを追加
  - @voluntas
- [ADD] デコーダのケーパビリティクエリ機能を追加する
  - `DecoderCaps` 構造体と `Decoder::query_caps` メソッドを追加
  - @voluntas
- [ADD] エンコーダの動的再設定機能を追加する
  - `ReconfigureParams` 構造体と `reconfigure` メソッドを追加
  - @voluntas
- [ADD] VP8 / VP9 / JPEG デコーダを追加する
  - `DecoderCodec::Vp8` / `DecoderCodec::Vp9` / `DecoderCodec::Jpeg` を追加
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
- [FIX] デコーダーがストリーム中の解像度変更に対応できない問題を修正する
  - `handle_video_sequence_inner` で既存デコーダーを破棄して再作成するようにする
  - @voluntas
- [FIX] `DecodedFrame::uv_plane()` が奇数高さの場合に UV プレーンのサイズを 1 行分少なく返す問題を修正する
  - `height / 2` を `height.div_ceil(2)` に変更し、奇数高さでも正しい行数の UV データを返すようにする
  - @sile
- [UPDATE] CUDA インクルードパスの解決をフォールバック付きの 3 段階方式に改善する
  - 環境変数 → デフォルトパス → スタブヘッダの順で探索する
  - @voluntas
- [UPDATE] `EncoderCaps` に `support_yuv422_encode` / `width_min` / `height_min` / `num_max_bframes` / `support_lookahead` / `support_temporal_aq` フィールドを追加する
  - @voluntas
- [ADD] `supported_codecs()` 関数を追加する
  - 指定 GPU デバイスで利用可能なコーデックのエンコード/デコード対応状況を一括で取得する
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
