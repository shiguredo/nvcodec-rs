# testdata/resolution-change のテストデータ

デコーダーの動的解像度変更テスト用の解像度変化ストリーム。

## データの性質

h264.h264 / h265.h265 / vp8.ivf / vp9.ivf / av1.ivf の 5 ファイルは以下の共通仕様を持つ。

- フレーム数は 45 で、320x240 x15 フレーム + 256x160 x15 フレーム + 320x240 x15 フレームの 3 セグメント構成
  - 小さい方の解像度が 256x160 なのは、HEVC のハードウェア最小デコード解像度 (144x144) と
    VP9 / AV1 の最小デコード解像度 (128x128) の両方を上回るサイズが必要なため
- 各セグメントの先頭はキーフレームで、パラメータセット (SPS/PPS/VPS 等) を含む
- コンテンツは合成テストパターン (人物・実映像を含まない)
- テスト (`src/decode.rs` の `mod tests`) は 1 フレームずつ `Decoder::decode` に渡すため、
  フレーム境界が分かる形式 (Annex-B のアクセスユニット / IVF フレーム) になっている

h264_upscale.h264 は上記 5 ファイルとはセグメント順が逆で、
256x160 x15 フレーム + 320x240 x15 フレーム + 256x160 x15 フレームの 3 セグメント構成を持つ。
初回 create の session 上限を 256x160 に設定してから上限超過の拡大を起こすことで、
上限超過後の reconfigure 経路 (再作成 → reconfigure) を検証できる。

## ファイル一覧

| ファイル | コーデック | 形式 | 備考 |
|---|---|---|---|
| `h264.h264` | H.264 | Annex-B (生ビットストリーム) | |
| `h265.h265` | H.265 | Annex-B (生ビットストリーム) | |
| `h264_upscale.h264` | H.264 | Annex-B (生ビットストリーム) | 256x160 → 320x240 → 256x160 |
| `vp8.ivf` | VP8 | IVF | |
| `vp9.ivf` | VP9 | IVF | 1 フレーム = 1 ピクチャ (alt-ref なし) |
| `av1.ivf` | AV1 | IVF | 1 フレーム = 1 ピクチャ |

VP9 / AV1 は alt-ref (VP9 のスーパーフレーム等) を含まないため、
IVF の 1 フレームが 1 ピクチャに対応する。

## テストでの使われ方

- `src/decode.rs` の `mod tests` が `include_bytes!` で読み込む
- 次の 3 系統の検証に使う
  - reconfigure 経路 (`reconfigure_enabled = true`) での解像度変化の検証
  - destroy + create 経路 (`reconfigure_enabled = false`) での解像度変化の検証
  - h264_upscale.h264 による上限超過後の reconfigure 経路の検証
