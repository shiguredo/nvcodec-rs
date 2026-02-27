# 0006 デコーダーの動的解像度変更に対応する

## 概要

ストリーム中に解像度が変わった場合、デコーダーを再作成して正しくデコードできるようにする。

## 背景

現在の `handle_video_sequence_inner` はデコーダーが既に作成済みの場合、早期リターンしている:

```rust
if !state.decoder.is_null() {
    return Ok(format.min_num_decode_surfaces as i32);
}
```

NVDEC SDK の `pfnSequenceCallback` はストリーム中で解像度やコーデックパラメータが変わるたびに呼ばれる。現在の実装では解像度変更を無視するため、以下のケースで問題が発生する:

- WebRTC で送信側が解像度を動的に変更する（例: 1080p -> 720p -> 480p）
- アダプティブビットレートストリーミングで解像度が切り替わる
- SVC (Scalable Video Coding) で解像度が変わる

## 現在の問題

1. デコーダーが最初の解像度で作成された後、解像度変更時に再作成されない
2. `ulMaxWidth` / `ulMaxHeight` が `coded_width` / `coded_height` と同じ値のため、SDK 内部の再構成も効かない
3. `handle_picture_display_inner` のフレームサイズ計算が最初の解像度のまま

## NVDEC SDK の解像度変更の仕組み

`pfnSequenceCallback` で解像度変更を検出する方法は 2 つある:

### 方法 1: デコーダーの再作成

1. `pfnSequenceCallback` で新しい `CUVIDEOFORMAT` を受け取る
2. 既存のデコーダーを `cuvidDestroyDecoder` で破棄する
3. 新しい解像度で `cuvidCreateDecoder` を呼ぶ

利点: シンプルで確実
欠点: 再作成のオーバーヘッドがある

### 方法 2: cuvidReconfigureDecoder を使用

1. `CUVIDDECODECREATEINFO` の `ulMaxWidth` / `ulMaxHeight` に最大解像度を設定して作成
2. `pfnSequenceCallback` で `cuvidReconfigureDecoder` を呼んで解像度を変更

利点: 再作成よりオーバーヘッドが小さい
欠点: `ulMaxWidth` / `ulMaxHeight` を事前に知る必要がある

## 設計方針

方法 1（デコーダーの再作成）を採用する。

理由:
- `ulMaxWidth` / `ulMaxHeight` を事前に知ることが難しい
- WebRTC のように解像度変更が頻繁でないユースケースでは再作成のオーバーヘッドは許容範囲
- 実装がシンプルで確実

## 実装

### `handle_video_sequence_inner` の変更

```rust
fn handle_video_sequence_inner(
    state: &mut DecoderState,
    format: &sys::CUVIDEOFORMAT,
) -> Result<i32, Error> {
    // デコーダーが既に作成されている場合は破棄する
    if !state.decoder.is_null() {
        state.lib.with_context(state.ctx, || {
            state.lib.cuvid_destroy_decoder(state.decoder)
        })?;
        state.decoder = ptr::null_mut();
    }

    // デコーダーの作成情報を設定（以下は既存コードと同様）
    // ...
}
```

### `DecodedFrame` への解像度情報の反映

フレームごとに `width` / `height` が異なる可能性があるため、`DecodedFrame` は既にフレームごとの解像度を保持している。`handle_picture_display_inner` で `state.width` / `state.height` を使用しているため、`handle_video_sequence_inner` で更新されていれば正しく動作する。

## 変更対象ファイル

- `src/decode.rs`: `handle_video_sequence_inner` の修正
- `CHANGES.md`: 変更履歴を追加

## 完了内容

- `handle_video_sequence_inner` の早期リターンを削除し、既存デコーダーを `cuvid_destroy_decoder` で破棄してから再作成するように修正
- `CHANGES.md` に FIX エントリを追加
