# testdata/10bit のテストデータ

10bit ストリーム (bit_depth_luma_minus8 != 0) の入力をデコーダーが拒否することを
検証するためのテストデータ。

## データの性質

| ファイル | コーデック | 寸法 | ビット深度 | 内容 |
|---|---|---|---|---|
| `black10.h265` | H.265 | 320x240 | 10bit (yuv420p10le) | 黒フレーム 1 枚 |

- 出力サーフェスは 8bit NV12 のみ対応のため、10bit 入力 (bit_depth_luma_minus8 = 2) は
  デコード不可として拒否される (P010 相当の出力サーフェスになると
  コピー処理 (8bit 前提) と整合しないため)
- `handle_video_sequence` の検証で明示的エラーを返し、Decoder が終端状態に遷移する

## 生成方法

- ffmpeg で生成: `ffmpeg -f lavfi -i color=c=black:s=320x240:d=0.04 -c:v libx265 -pix_fmt yuv420p10le -x265-params bit-depth=10:internal-bitdepth=10 -frames:v 1 black10.h265`

## テストでの使われ方

- `src/decode.rs` の `mod tests` が `include_bytes!` で読み込む
- 10bit 入力を拒否するテスト (`test_decode_h265_10bit_rejected`) に使う
- デコードの実機検証は NVIDIA GPU を要するため、CI の `test-nvidia-video-codec` で実行する
