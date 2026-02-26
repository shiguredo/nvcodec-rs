# third_party ディレクトリについて

third_party ディレクトリは、外部から取得したコードを置いておくためのディレクトリです。

## コードフォーマッタについて

third_party ディレクトリ以下のコードは可能な限り外部から取得した状態を維持し、コードフォーマッタの利用はしません。
これは、ライブラリのアップデート時に不要な差分が出るのを避けることを目的とします。

## third_party/nvcodec について

`third_party/nvcodec` は [NVIDIA Video Codec SDK](https://developer.nvidia.com/video-codec-sdk) から取得したものを使用しています。

## third_party/cuda について

`third_party/cuda/include/cuda.h` は、CUDA Toolkit がインストールされていない環境 (macOS など) で
バインディング生成を可能にするためのスタブヘッダです。

NVIDIA Video Codec SDK のヘッダが必要とする最小限の CUDA Driver API 型定義のみを提供します。
CUDA Toolkit がインストールされている環境では、スタブではなく実際の `cuda.h` が優先的に使用されます。
