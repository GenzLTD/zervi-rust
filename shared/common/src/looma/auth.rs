use super::error::fail;
use super::x1::Settings;
use super::AppState;
use axum::{
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    response::Response,
};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::Deserialize;
use serde_json::Value;

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
    let token = bearer(headers);
    match (token, mode) {
        (None, AuthMode::Required) => Err(fail(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or invalid Authorization header",
        )),
        (None, AuthMode::Optional) => Ok(guest()),
        (Some(token), mode) => match verify_token(&state.settings, &token) {
            Ok((source, claims)) => match load_user(state, source, &claims).await {
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
) -> Result<Option<Principal>, Response> {
    use crate::entities::users;
    if source == "looma" {
        let user = users::Entity::find_by_id(claims.sub.clone())
            .one(&state.db)
            .await
            .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e.to_string()))?;
        return Ok(user.map(principal_from_user));
    }

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
    if let Some(email) = email.clone() {
        if let Some(user) = users::Entity::find()
            .filter(users::Column::Email.eq(email))
            .one(&state.db)
            .await
            .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e.to_string()))?
        {
            let mut active: users::ActiveModel = user.into();
            active.supabase_uid = Set(Some(claims.sub.clone()));
            let saved = active.update(&state.db).await.map_err(|e| {
                fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e.to_string())
            })?;
            return Ok(Some(principal_from_user(saved)));
        }
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
