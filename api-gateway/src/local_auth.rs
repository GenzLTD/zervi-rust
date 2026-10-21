//! 本地开发账号体系（邮箱 + 密码）
//!
//! ## 为什么放在网关而不是 auth-service
//!
//! `auth-service` 是**手机号 + 短信验证码**体系，依赖 MySQL `adirp_auth` 与 Redis，
//! 且自签 token 的 claims 口径与 `shared/common/src/looma/auth.rs` 的校验不一致
//! （后者要求 `iss=looma` 的 HS256 token，并把 `sub` 落到 PostgreSQL `users.id`）。
//!
//! 而 resume / job / application 等服务的 `AuthMode::Required` 全部走 looma 口径。
//! 因此这里直接对 `users` 表做邮箱密码登录，签发 **looma 同源 HS256 token**，
//! 使客户端拿到的 token 能被所有后端服务直接认可。
//!
//! ## 边界（重要）
//!
//! - 仅在 `ZERVI_DEV_AUTH=1` 时启用；未设置时三个端点一律返回 503。
//! - 生产环境的账号体系是 **Supabase Auth**（`SUPABASE_JWKS_URL` / `SUPABASE_JWT_SECRET`
//!   两轨已在 `looma::auth` 里就绪），本模块不参与生产。
//! - 密码用 bcrypt(cost=10) 存储到 `users.password_hash`，与 schema 既有列对齐。

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use tracing::{info, warn};

use crate::AppState;

/// 本地账号体系开关。未设置 ⇒ 端点关闭。
const ENV_DEV_AUTH: &str = "ZERVI_DEV_AUTH";
/// 与 `looma::auth::decode_looma` 的 issuer 断言保持一致 —— 改这里会让 token 全挂。
const JWT_ISSUER: &str = "looma";
/// 开发期 token 有效期：30 天（免去反复登录）
const TOKEN_TTL_SECS: i64 = 30 * 24 * 3600;
/// bcrypt 成本因子（与常见默认一致；过高会让注册在模拟器上明显变慢）
const BCRYPT_COST: u32 = 10;

// ─────────────────────────────────────────────────────────────────────────────
// 数据结构
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct UserInfo {
    pub id: String,
    pub email: String,
    pub name: String,
    pub tier: String,
    pub role: String,
}

#[derive(Debug, Serialize)]
struct AuthResponse {
    token: String,
    user: UserInfo,
}

#[derive(Debug, Serialize, Deserialize)]
struct LoomaClaims {
    sub: String,
    iss: String,
    iat: i64,
    exp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    role: String,
    tier: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// 工具
// ─────────────────────────────────────────────────────────────────────────────

fn dev_auth_enabled() -> bool {
    matches!(
        std::env::var(ENV_DEV_AUTH).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE")
    )
}

fn err(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "error": code, "message": message.into() })),
    )
        .into_response()
}

fn disabled() -> Response {
    err(
        StatusCode::SERVICE_UNAVAILABLE,
        "dev_auth_disabled",
        "本地开发账号体系未启用：请在 .env 设置 ZERVI_DEV_AUTH=1（生产环境请接入 Supabase Auth）",
    )
}

fn jwt_secret() -> Option<String> {
    std::env::var("JWT_SECRET").ok().filter(|s| !s.trim().is_empty())
}

fn normalize_email(raw: &str) -> String {
    raw.trim().to_lowercase()
}

fn email_hash(email: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(email.as_bytes());
    hex::encode(hasher.finalize())
}

/// 与 `looma::auth::resolve_principal` 同一把钥匙、同一个 issuer ⇒ 服务端可直接认。
fn mint_token(user: &UserInfo) -> Result<String, Response> {
    let secret = jwt_secret().ok_or_else(|| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "JWT_SECRET 未配置，无法签发 token",
        )
    })?;
    let now = Utc::now().timestamp();
    let claims = LoomaClaims {
        sub: user.id.clone(),
        iss: JWT_ISSUER.to_string(),
        iat: now,
        exp: now + TOKEN_TTL_SECS,
        email: Some(user.email.clone()),
        name: Some(user.name.clone()),
        role: user.role.clone(),
        tier: user.tier.clone(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            format!("token 签发失败：{e}"),
        )
    })
}

fn db_or_503(state: &AppState) -> Result<PgPool, Response> {
    state.db.clone().ok_or_else(|| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "db_unavailable",
            "DATABASE_URL 未配置，本地账号体系不可用",
        )
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// 数据访问（沿用 schema 既有 users 表，不新增实体文件）
//
// 全部列在 schema 里都可空（除 id），所以统一按 Option<String> 读取再兜底。
// ─────────────────────────────────────────────────────────────────────────────

struct UserRow {
    id: String,
    email: String,
    name: String,
    tier: String,
    role: String,
    password_hash: Option<String>,
}

impl UserRow {
    fn info(&self) -> UserInfo {
        UserInfo {
            id: self.id.clone(),
            email: self.email.clone(),
            name: if self.name.is_empty() {
                self.email.split('@').next().unwrap_or("用户").to_string()
            } else {
                self.name.clone()
            },
            tier: if self.tier.is_empty() {
                "free".into()
            } else {
                self.tier.clone()
            },
            role: if self.role.is_empty() {
                "user".into()
            } else {
                self.role.clone()
            },
        }
    }
}

fn text(row: &sqlx::postgres::PgRow, col: &str) -> String {
    row.try_get::<Option<String>, _>(col)
        .ok()
        .flatten()
        .unwrap_or_default()
}

const SELECT_COLUMNS: &str = "id, email, name, tier, role, password_hash";

async fn find_by_email(db: &PgPool, email: &str) -> Result<Option<UserRow>, sqlx::Error> {
    let sql = format!("SELECT {SELECT_COLUMNS} FROM users WHERE lower(email) = lower($1) LIMIT 1");
    let row = sqlx::query(&sql).bind(email).fetch_optional(db).await?;
    Ok(row.map(|r| UserRow {
        id: text(&r, "id"),
        email: text(&r, "email"),
        name: text(&r, "name"),
        tier: text(&r, "tier"),
        role: text(&r, "role"),
        password_hash: r.try_get::<Option<String>, _>("password_hash").ok().flatten(),
    }))
}

async fn find_by_id(db: &PgPool, id: &str) -> Result<Option<UserRow>, sqlx::Error> {
    let sql = format!("SELECT {SELECT_COLUMNS} FROM users WHERE id = $1 LIMIT 1");
    let row = sqlx::query(&sql).bind(id).fetch_optional(db).await?;
    Ok(row.map(|r| UserRow {
        id: text(&r, "id"),
        email: text(&r, "email"),
        name: text(&r, "name"),
        tier: text(&r, "tier"),
        role: text(&r, "role"),
        password_hash: r.try_get::<Option<String>, _>("password_hash").ok().flatten(),
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers
// ─────────────────────────────────────────────────────────────────────────────

/// POST /v1/auth/register —— 邮箱注册，成功即返回 token（免二次登录）
pub async fn register(State(state): State<AppState>, Json(req): Json<RegisterRequest>) -> Response {
    if !dev_auth_enabled() {
        return disabled();
    }
    let db = match db_or_503(&state) {
        Ok(db) => db,
        Err(resp) => return resp,
    };

    let email = normalize_email(&req.email);
    if !email.contains('@') || email.split('@').nth(1).map_or(true, |d| !d.contains('.')) {
        return err(StatusCode::BAD_REQUEST, "invalid_email", "邮箱格式不正确");
    }
    if req.password.chars().count() < 8 {
        return err(
            StatusCode::BAD_REQUEST,
            "weak_password",
            "密码至少 8 位",
        );
    }
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| email.split('@').next().unwrap_or("用户"))
        .to_string();

    match find_by_email(&db, &email).await {
        Ok(Some(_)) => {
            return err(
                StatusCode::CONFLICT,
                "email_taken",
                "该邮箱已注册，请直接登录",
            )
        }
        Ok(None) => {}
        Err(e) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                format!("查询用户失败：{e}"),
            )
        }
    }

    let password = req.password.clone();
    let hash = match tokio::task::spawn_blocking(move || bcrypt::hash(password, BCRYPT_COST)).await
    {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                format!("密码加密失败：{e}"),
            )
        }
        Err(e) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                format!("密码加密任务失败：{e}"),
            )
        }
    };

    let id = uuid::Uuid::new_v4().to_string();
    let insert = sqlx::query(
        "INSERT INTO users \
         (id, email, email_hash, password_hash, name, tier, role, is_early_adopter, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, 'free', 'user', 1, now(), now())",
    )
    .bind(&id)
    .bind(&email)
    .bind(email_hash(&email))
    .bind(hash)
    .bind(&name)
    .execute(&db)
    .await;
    if let Err(e) = insert {
        // 并发注册时唯一约束兜底
        let msg = e.to_string();
        if msg.contains("users_email_key") || msg.contains("duplicate key") {
            return err(
                StatusCode::CONFLICT,
                "email_taken",
                "该邮箱已注册，请直接登录",
            );
        }
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            format!("创建用户失败：{msg}"),
        );
    }

    let user = UserInfo {
        id,
        email: email.clone(),
        name,
        tier: "free".into(),
        role: "user".into(),
    };
    info!("🆕 [dev-auth] 注册成功：{} ({})", user.email, user.id);
    match mint_token(&user) {
        Ok(token) => (StatusCode::CREATED, Json(AuthResponse { token, user })).into_response(),
        Err(resp) => resp,
    }
}

/// POST /v1/auth/login —— 邮箱 + 密码登录
pub async fn login(State(state): State<AppState>, Json(req): Json<LoginRequest>) -> Response {
    if !dev_auth_enabled() {
        return disabled();
    }
    let db = match db_or_503(&state) {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let email = normalize_email(&req.email);

    let row = match find_by_email(&db, &email).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return err(
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "邮箱或密码不正确",
            )
        }
        Err(e) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                format!("查询用户失败：{e}"),
            )
        }
    };

    let Some(stored) = row.password_hash.clone().filter(|h| !h.is_empty()) else {
        return err(
            StatusCode::UNAUTHORIZED,
            "no_password",
            "该账号未设置密码登录（可能由 Supabase / 微信创建）",
        );
    };
    let password = req.password.clone();
    let ok = match tokio::task::spawn_blocking(move || bcrypt::verify(password, &stored)).await {
        Ok(Ok(ok)) => ok,
        Ok(Err(e)) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                format!("密码校验失败：{e}"),
            )
        }
        Err(e) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                format!("密码校验任务失败：{e}"),
            )
        }
    };
    if !ok {
        warn!("🚫 [dev-auth] 登录失败（密码不匹配）：{email}");
        return err(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "邮箱或密码不正确",
        );
    }

    let user = row.info();
    info!("✅ [dev-auth] 登录成功：{} ({})", user.email, user.id);
    match mint_token(&user) {
        Ok(token) => Json(AuthResponse { token, user }).into_response(),
        Err(resp) => resp,
    }
}

/// GET /v1/auth/me —— 用当前 token 换取账号信息（客户端冷启动校验登录态）
pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !dev_auth_enabled() {
        return disabled();
    }
    let db = match db_or_503(&state) {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let Some(secret) = jwt_secret() else {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "JWT_SECRET 未配置",
        );
    };
    let Some(raw) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) else {
        return err(StatusCode::UNAUTHORIZED, "unauthorized", "缺少 Authorization 头");
    };
    let token = raw.trim().strip_prefix("Bearer ").unwrap_or(raw).trim();
    if token.is_empty() {
        return err(StatusCode::UNAUTHORIZED, "unauthorized", "token 为空");
    }

    // 与 looma 侧同口径：HS256 + 关掉 aud 校验（HS256 轨无 aud 语义）
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_aud = false;
    validation.set_issuer(&[JWT_ISSUER]);
    let claims = match jsonwebtoken::decode::<LoomaClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    ) {
        Ok(data) => data.claims,
        Err(e) => {
            return err(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                format!("token 无效或已过期：{e}"),
            )
        }
    };

    match find_by_id(&db, &claims.sub).await {
        Ok(Some(row)) => Json(row.info()).into_response(),
        Ok(None) => err(
            StatusCode::UNAUTHORIZED,
            "user_not_found",
            "token 指向的用户不存在（可能已被删除）",
        ),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            format!("查询用户失败：{e}"),
        ),
    }
}
