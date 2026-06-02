# Stage 1: builder
# Multi-stage build: ~1.5GB builder → <100MB runtime.
# Stage 4 简化: 不引入 alpine/musl (避免 OpenSSL 编译), 用 rust:1.85-slim + debian:bookworm-slim.
FROM rust:1.85-slim AS builder

WORKDIR /app

# 依赖层 (COPY Cargo.toml 先, 利用 Docker 缓存)
COPY Cargo.lock Cargo.toml ./
COPY qianxun-core/ ./qianxun-core/
COPY qianxun-memory/ ./qianxun-memory/
COPY qianxun/ ./qianxun/

# pkg-config + libssl-dev 是 reqwest (rustls 也可不需要, 但保守起见保留)
# 实际上 rustls 后端不依赖 OpenSSL, 但其他传递依赖可能需要. 留作兜底.
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        pkg-config libssl-dev ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Release build (~3-5min on first build, 增量 ~30s)
RUN cargo build --release --bin qx

# Stage 2: runtime
# debian:bookworm-slim ~50MB, 加上 ca-certificates ~10MB, 总计 <80MB.
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# 非 root 用户 (UID 1000 与 host docker 普通用户对齐)
RUN useradd -m -u 1000 qxvps

USER qxvps
WORKDIR /home/qxvps

# 从 builder 拷贝编译产物
COPY --from=builder /app/target/release/qx /usr/local/bin/qx

# 端口: 02-vps-server.md §1.4 默认 23901
EXPOSE 23901

# 数据卷: 持久化 SQLite + 上传文件
VOLUME ["/home/qxvps/.qxvps"]

# 默认启动命令: VPS Server 模式, 监听 23901
# 环境变量 QXVPS_LOG_LEVEL 在 docker-compose 中设置
ENTRYPOINT ["/usr/local/bin/qx", "--server", "--port", "23901"]
