FROM nvidia/cuda:13.1.1-devel-ubuntu24.04

# ビルドに必要なパッケージ
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    build-essential \
    pkg-config \
    libclang-dev \
    && rm -rf /var/lib/apt/lists/*

# Rust ツールチェインのインストール
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"

# clippy / rustfmt コンポーネント（rust-toolchain.toml と同期）
RUN rustup component add clippy rustfmt
