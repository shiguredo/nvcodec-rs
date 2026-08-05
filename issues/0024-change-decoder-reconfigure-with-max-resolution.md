# 0024-add-decoder-reconfigure-with-max-resolution

Created: 2026-08-05

## 背景

issue 0006 で `handle_video_sequence_inner` にストリーム中の解像度変更対応が実装された。実装は「方法 1: デコーダーの再作成」で、`pfnSequenceCallback` で解像度変更を検出した際に既存のデコーダーを `cuvid_destroy_decoder` で破棄してから `cuvid_create_decoder` で新規作成する。

0006 の設計方針にはもう一つ「方法 2: cuvidReconfigureDecoder を使用」が挙げられていたが、`ulMaxWidth` / `ulMaxHeight` を事前に知る必要があるという理由で見送られた。

## 問題

現在の破棄→再作成方式は正しく動作するものの、以下の課題がある。

1. **キーフレーム毎の再作成コスト**
   - WebRTC のシミュキャスト / 適応ビットレート録画のように、per-frame に近い頻度で符号化解像度が変わるストリームでは、キーフレーム到来のたびに `cuvidCreateDecoder` が呼ばれる
   - GPU デコーダーの生成コストはフレーム毎の処理として看過できないオーバーヘッドが乗る想定
2. **create 失敗時の復旧不能**（issue 0017 pending で言及済み）
   - destroy-then-create の順序のため、新規 create が失敗すると既存デコーダーは既に破棄済みで復旧不能になる
   - `cuvidReconfigureDecoder` は既存デコーダーを in-place で再構成するため、失敗しても既存デコーダーは温存される

なお `CUVIDDECODECREATEINFO.ulMaxWidth` / `ulMaxHeight` は現在も `format.coded_width` / `coded_height`（初回フレームのサイズ）に固定されているため、方法 2 に切り替えるにはここも見直す必要がある。破棄→再作成方式ではフレーム毎に上限が更新されるため実害は出ていない。

## 提案

呼び出し側から最大解像度が渡された場合に限り、`cuvidReconfigureDecoder` による in-place 再構成に切り替える。渡されなかった場合は現状どおり破棄→再作成にフォールバックする（後方互換を保つ）。

### 変更内容

- `DecoderConfig` に `max_coded_width: Option<u32>` / `max_coded_height: Option<u32>` を追加する
  - `None` の場合は現状の破棄→再作成方式で動作する
  - `Some` の場合は `CUVIDDECODECREATEINFO.ulMaxWidth` / `ulMaxHeight` にその値を設定して初回作成し、2 回目以降の `pfnSequenceCallback` では `cuvidReconfigureDecoder` を呼ぶ
- `handle_video_sequence_inner` を初回作成パスと再構成パスに分岐する
- `cuvidReconfigureDecoder` に対応する Rust ラッパー (`cuvid_reconfigure_decoder`) を `CudaLibrary` に追加する
- 再構成時にサイズが `ulMaxWidth` / `ulMaxHeight` を超えたらエラーを返す

### 変更対象ファイル

- `src/decode.rs`: `handle_video_sequence_inner` の分岐、`DecoderConfig` 拡張
- `src/lib.rs`: `cuvid_reconfigure_decoder` ラッパーとローダー登録の追加
- `CHANGES.md`: 変更履歴を追加

## 関連 issue

- 0006（closed）方法 1 で実装されたデコーダーの動的解像度変更
- 0017（pending）破棄→再作成の順序による復旧不能問題
