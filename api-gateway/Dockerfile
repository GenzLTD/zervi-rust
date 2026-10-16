# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# api-gateway 生产环境 Dockerfile
#
# 用途：ADR-0012 单 1 网关的部署载体（Vultr-Genzltd#39 验收①）
#
# 构建上下文是**仓库根目录**（workspace 根），不是本文件所在目录：
#   docker build -f api-gateway/Dockerfile -t api-gateway:latest .
#
# 部署侧职责（compose 落实，非本文件）：
#   - 端口**只映射宿主回环**，禁止 `0.0.0.0:` 公网直曝
#   - 运行时 env 由 compose 注入（见 .env.example），镜像不固化任何密钥
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# ─── Stage 1: 构建 ───
# 版本下限由依赖链决定，不是随便挑的：依赖 `home v0.5.12` 的 manifest 声明
# `edition2024`，该特性 Rust 1.85 才稳定 —— 1.83 会在解析阶段直接失败
# （error: feature `edition2024` is required）。
# 取 1.98 与本机工具链（rustc/cargo 1.98.1）对齐，且是确定版本而非 :latest，
# 避免 tag 漂移导致构建不可复现。
FROM rust:1.98 AS builder

WORKDIR /app

COPY . .

RUN cargo build --release -p api-gateway --locked

# ─── Stage 2: 运行时 ───
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

# 非 root：沿用 upload-service/Dockerfile.production 的 uid 1000
RUN useradd -m -u 1000 -s /bin/bash appuser

WORKDIR /app

COPY --from=builder /app/target/release/api-gateway /usr/local/bin/api-gateway
RUN chmod +x /usr/local/bin/api-gateway && chown -R appuser:appuser /app

USER appuser

# main.rs 监听 3333；是否对公网可达由 compose 的端口映射决定，不由本文件决定
EXPOSE 3333

HEALTHCHECK --interval=30s --timeout=10s --start-period=20s --retries=3 \
    CMD curl -fsS http://localhost:3333/health || exit 1

# 运行时 env 全部由 compose 注入：
#   GATEWAY_JWKS_URL / GATEWAY_JWT_SECRET / TATHA_BASE_URL
#   TATHA_SERVICE_TOKEN / GATEWAY_DEFAULT_TIER
ENTRYPOINT ["/usr/local/bin/api-gateway"]
