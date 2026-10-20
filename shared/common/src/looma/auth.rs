use super::error::{fail, fail_extra};
use super::x1::Settings;
use super::AppState;
use axum::{
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    response::Response,
};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub struct Principal {
    pub user_id: String,
    pub tier: String,
    pub role: String,
    pub guest: bool,
}

#[derive(Clone, Copy)]
pub enum AuthMode {
    Required,
    Optional,
}

/// 解析登录态时，对"库里没有可对应的 `users` 行"的处理策略。
///
/// 背景（P0-1 身份键断裂）：`load_user` 原来在 `supabase_uid` 未命中、email 也未命中时
/// **直接 INSERT 一行新 users**（新 uuid / tier=free / role=user）。对只读接口这是便利，
/// 对**写钱**的接口（IAP verify / restore）是灾难：
/// Apple「隐藏我的邮箱」会让 email 匹配失效，于是老账号旁边多出一行"孤儿 user"，
/// 订阅记在孤儿行上 —— HTTP 200、日志无异常、用户买完不解锁（3.1.1 拒审经典款）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provision {
    /// 历史行为：uid 未命中 → 按 email 合并 → 仍没有就新建 users 行。
    Auto,
    /// 禁止隐式建档：uid 未命中时仍允许**按 email 合并**（那是"绑定"，不是"建档"），
    /// 但绝不 INSERT。返回 `409 need_link`，由客户端显式建档/绑定后再重试 verify。
    NoAutoCreate,
}

impl Provision {
    fn allows_auto_create(self) -> bool {
        matches!(self, Provision::Auto)
    }
}

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    user_metadata: Option<Value>,
}

pub async fn resolve_principal(
    state: &AppState,
    headers: &HeaderMap,
    mode: AuthMode,
) -> Result<Principal, Response> {
    resolve_principal_with(state, headers, mode, Provision::Auto).await
}

/// 同 [`resolve_principal`]，但由调用方指定建档策略。
///
/// 写钱路径（payment 的 verify / restore）必须传 [`Provision::NoAutoCreate`]：
/// 认不出身份时宁可 409，也不能新开一行。
pub async fn resolve_principal_with(
    state: &AppState,
    headers: &HeaderMap,
    mode: AuthMode,
    provision: Provision,
) -> Result<Principal, Response> {
    let token = bearer(headers);
    match (token, mode) {
        (None, AuthMode::Required) => Err(fail(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or invalid Authorization header",
        )),
        (None, AuthMode::Optional) => Ok(guest()),
        (Some(token), mode) => match verify_token(&state.settings, &token) {
            Ok((source, claims)) => match load_user(state, source, &claims, provision).await {
                Ok(Some(p)) => Ok(p),
                Ok(None) if matches!(mode, AuthMode::Required) => Err(fail(
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                    "user not found",
                )),
                Ok(None) => Ok(guest()),
                Err(resp) => Err(resp),
            },
            Err(_) if matches!(mode, AuthMode::Optional) => Ok(guest()),
            Err(message) => Err(fail(StatusCode::UNAUTHORIZED, "unauthorized", message)),
        },
    }
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let token = raw.strip_prefix("Bearer ")?.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn guest() -> Principal {
    let id = uuid::Uuid::new_v4().simple().to_string();
    Principal {
        user_id: format!("guest-{}", &id[..12]),
        tier: "guest".into(),
        role: "user".into(),
        guest: true,
    }
}

fn verify_token(settings: &Settings, token: &str) -> Result<(&'static str, Claims), String> {
    if let Some(claims) = decode_looma(settings, token) {
        return Ok(("looma", claims));
    }
    if let Some(claims) = decode_supabase_hs(settings, token) {
        return Ok(("supabase", claims));
    }
    Err("Invalid token: token failed looma and supabase verification".into())
}

fn decode_looma(settings: &Settings, token: &str) -> Option<Claims> {
    if settings.jwt_secret.is_empty() {
        return None;
    }
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_issuer(&["looma"]);
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(settings.jwt_secret.as_bytes()),
        &validation,
    )
    .ok()
    .map(|d| d.claims)
}

fn decode_supabase_hs(settings: &Settings, token: &str) -> Option<Claims> {
    if settings.supabase_jwt_secret.is_empty() {
        return None;
    }
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&["authenticated"]);
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(settings.supabase_jwt_secret.as_bytes()),
        &validation,
    )
    .ok()
    .map(|d| d.claims)
}

async fn load_user(
    state: &AppState,
    source: &str,
    claims: &Claims,
    provision: Provision,
) -> Result<Option<Principal>, Response> {
    use crate::entities::users;
    if source == "looma" {
        let user = users::Entity::find_by_id(claims.sub.clone())
            .one(&state.db)
            .await
            .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e.to_string()))?;
        return Ok(user.map(principal_from_user));
    }

    // ① 已绑定：`users.supabase_uid == sub` —— 唯一可信的稳定命中。
    let existing = users::Entity::find()
        .filter(users::Column::SupabaseUid.eq(claims.sub.clone()))
        .one(&state.db)
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e.to_string()))?;
    if let Some(user) = existing {
        return Ok(Some(principal_from_user(user)));
    }

    let email = claims
        .email
        .clone()
        .filter(|e| !e.is_empty())
        .or_else(|| {
            claims
                .user_metadata
                .as_ref()
                .and_then(|m| m.get("email"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });

    // ② 未绑定但 email 命中历史行：这是"绑定"（link），不是"建档"，两种策略都放行。
    //    生产库 users.email **没有 UNIQUE 约束**，命中多行时不能瞎挑一行绑，交人工处理。
    if let Some(email) = email.clone() {
        let matches = users::Entity::find()
            .filter(users::Column::Email.eq(email.clone()))
            .all(&state.db)
            .await
            .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e.to_string()))?;
        match matches.len() {
            1 => {
                let user = matches.into_iter().next().expect("len == 1");
                let user_id = user.id.clone();
                let mut active: users::ActiveModel = user.into();
                active.supabase_uid = Set(Some(claims.sub.clone()));
                let saved = active.update(&state.db).await.map_err(|e| {
                    fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e.to_string())
                })?;
                tracing::info!(
                    user_id = %user_id,
                    supabase_uid = %claims.sub,
                    email = %email,
                    "identity: 按 email 绑定历史账号（未建档）"
                );
                return Ok(Some(principal_from_user(saved)));
            }
            n if n > 1 => {
                tracing::error!(
                    supabase_uid = %claims.sub,
                    email = %email,
                    candidates = n,
                    "identity: email 命中多行 users，拒绝自动绑定"
                );
                return Err(need_link(
                    &claims.sub,
                    Some(&email),
                    "ambiguous_email",
                    "该邮箱在 users 表对应多个账号，需人工确认绑定关系后再重试",
                ));
            }
            _ => {}
        }
    }

    // ③ 完全认不出：这是会产生"孤儿用户"的分叉点。
    if !provision.allows_auto_create() {
        tracing::warn!(
            supabase_uid = %claims.sub,
            email = ?email,
            "identity: 无绑定且无 email 命中，策略=NoAutoCreate，拒绝隐式建档（409 need_link）"
        );
        return Err(need_link(
            &claims.sub,
            email.as_deref(),
            "unlinked_login",
            "该登录态尚未绑定任何账号；请先显式绑定或新建账号，再重试",
        ));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = super::now_tz();
    let created = users::ActiveModel {
        id: Set(id),
        email: Set(email),
        email_hash: Set(None),
        password_hash: Set(None),
        wechat_openid: Set(None),
        supabase_uid: Set(Some(claims.sub.clone())),
        name: Set(claims
            .user_metadata
            .as_ref()
            .and_then(|m| m.get("full_name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())),
        tier: Set(Some("free".into())),
        role: Set(Some("user".into())),
        is_early_adopter: Set(None),
        created_at: Set(Some(now)),
        updated_at: Set(Some(now)),
    };
    let user = created.insert(&state.db).await.map_err(|e| {
        fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e.to_string())
    })?;
    Ok(Some(principal_from_user(user)))
}

/// `409 need_link`：身份未绑定，客户端需先显式绑定/建档。
///
/// 契约（客户端按 `error` 分支处理，不要只看 HTTP 码）：
/// ```json
/// {
///   "error": "need_link",
///   "message": "…",
///   "code": "need_link",
///   "reason": "unlinked_login" | "ambiguous_email",
///   "supabase_uid": "<JWT sub>",
///   "email": "<JWT email 或 null>"
/// }
/// ```
fn need_link(sub: &str, email: Option<&str>, reason: &'static str, message: &'static str) -> Response {
    fail_extra(
        StatusCode::CONFLICT,
        "need_link",
        message,
        json!({
            "code": "need_link",
            "reason": reason,
            "supabase_uid": sub,
            "email": email,
        }),
    )
}

fn principal_from_user(user: crate::entities::users::Model) -> Principal {
    Principal {
        user_id: user.id,
        tier: user.tier.unwrap_or_else(|| "free".into()),
        role: user.role.unwrap_or_else(|| "user".into()),
        guest: false,
    }
}

pub fn require_tier(principal: &Principal, min_tier: &str) -> Result<(), Response> {
    let rank = |t: &str| match t {
        "guest" => -1,
        "free" => 0,
        "supporter" => 1,
        "pro" => 2,
        "enterprise" => 3,
        _ => 0,
    };
    if rank(&principal.tier) < rank(min_tier) {
        return Err(fail(
            StatusCode::FORBIDDEN,
            "forbidden",
            format!("Requires tier '{min_tier}' or above"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[test]
    fn provision_gates_implicit_creation() {
        assert!(Provision::Auto.allows_auto_create());
        assert!(
            !Provision::NoAutoCreate.allows_auto_create(),
            "NoAutoCreate 必须拒绝隐式建档，否则 P0-1 会复现"
        );
    }

    /// 契约测试：客户端按 `error`/`reason` 分支处理，不只看 HTTP 码。
    #[tokio::test]
    async fn need_link_is_409_with_machine_readable_body() {
        let resp = need_link("sub-abc", Some("a@b.c"), "unlinked_login", "先绑定再来");
        assert_eq!(resp.status(), StatusCode::CONFLICT);

        let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
        let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(v["error"], "need_link");
        assert_eq!(v["code"], "need_link");
        assert_eq!(v["reason"], "unlinked_login");
        assert_eq!(v["supabase_uid"], "sub-abc");
        assert_eq!(v["email"], "a@b.c");
    }

    /// 邮箱缺失时也要给出稳定结构（`email: null`），客户端不必判空。
    #[tokio::test]
    async fn need_link_tolerates_missing_email() {
        let resp = need_link("sub-abc", None, "ambiguous_email", "多行");
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
        let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(v["reason"], "ambiguous_email");
        assert!(v["email"].is_null());
    }
}
