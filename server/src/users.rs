use crate::{
    config_store,
    emby::{EmbyClient, EmbyUser, EmbyUserPolicy},
    error::{AppError, AppResult},
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use serde::{Deserialize, Serialize};

const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";
const BPS_PER_MBPS: f64 = 1_000_000.0;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UsersResponse {
    pub users: Vec<UserSummary>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UserSummary {
    pub id: String,
    pub name: String,
    pub disabled: bool,
    pub last_activity_date: Option<String>,
    pub policy: UserPolicySummary,
    pub remote_bitrate_mbps: Option<f64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UserPolicySummary {
    #[serde(rename = "RemoteClientBitrateLimit")]
    pub remote_client_bitrate_limit: Option<i64>,
    #[serde(rename = "SimultaneousStreamLimit")]
    pub simultaneous_stream_limit: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateUserPolicyRequest {
    pub remote_bitrate_mbps: Option<f64>,
    pub simultaneous_stream_limit: Option<i64>,
    pub disabled: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UpdateUserPolicyResponse {
    pub ok: bool,
    pub user: UserSummary,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v2/users", get(list_users)).route(
        "/api/v2/users/{id}/policy",
        get(get_user_policy).put(update_user_policy),
    )
}

#[utoipa::path(get, path = "/api/v2/users", tag = "users", responses((status = 200, body = UsersResponse)))]
pub async fn list_users(State(state): State<AppState>) -> AppResult<Json<UsersResponse>> {
    let client = emby_client_from_config(&state).await?;
    Ok(Json(list_users_with_client(&client).await?))
}

#[utoipa::path(get, path = "/api/v2/users/{id}/policy", tag = "users", params(("id" = String, Path, description = "Emby user id")), responses((status = 200, body = UserSummary)))]
pub async fn get_user_policy(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<UserSummary>> {
    let client = emby_client_from_config(&state).await?;
    Ok(Json(get_user_policy_with_client(&client, &id).await?))
}

#[utoipa::path(put, path = "/api/v2/users/{id}/policy", tag = "users", params(("id" = String, Path, description = "Emby user id")), request_body = UpdateUserPolicyRequest, responses((status = 200, body = UpdateUserPolicyResponse)))]
pub async fn update_user_policy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateUserPolicyRequest>,
) -> AppResult<Json<UpdateUserPolicyResponse>> {
    let client = emby_client_from_config(&state).await?;
    Ok(Json(
        update_user_policy_with_client(&client, &id, req).await?,
    ))
}

pub async fn list_users_with_client(client: &EmbyClient) -> AppResult<UsersResponse> {
    let users = client.users().await?;
    Ok(UsersResponse {
        users: users.into_iter().map(UserSummary::from).collect(),
    })
}

pub async fn get_user_policy_with_client(client: &EmbyClient, id: &str) -> AppResult<UserSummary> {
    let user = client.user(id).await?;
    Ok(UserSummary::from(user))
}

pub async fn update_user_policy_with_client(
    client: &EmbyClient,
    id: &str,
    req: UpdateUserPolicyRequest,
) -> AppResult<UpdateUserPolicyResponse> {
    let mut user = client.user(id).await?;
    merge_policy(&mut user.policy, req)?;
    client.update_user_policy(id, &user.policy).await?;
    let updated = client.user(&id).await.unwrap_or(user);
    Ok(UpdateUserPolicyResponse {
        ok: true,
        user: UserSummary::from(updated),
    })
}

async fn emby_client_from_config(state: &AppState) -> AppResult<EmbyClient> {
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;
    Ok(EmbyClient::new(emby_url, api_key, state.http.clone()))
}

pub fn ensure_api_key_configured(api_key: &str) -> AppResult<()> {
    if api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "api_key is not configured; set it via /api/v2/config before managing Emby users"
                .to_string(),
        ));
    }
    Ok(())
}

fn merge_policy(policy: &mut EmbyUserPolicy, req: UpdateUserPolicyRequest) -> AppResult<()> {
    if let Some(mbps) = req.remote_bitrate_mbps {
        if !mbps.is_finite() || mbps < 0.0 {
            return Err(AppError::BadRequest(
                "remote_bitrate_mbps must be a finite non-negative number".to_string(),
            ));
        }
        policy.remote_client_bitrate_limit = Some((mbps * BPS_PER_MBPS).round() as i64);
    }
    if let Some(limit) = req.simultaneous_stream_limit {
        if limit < 0 {
            return Err(AppError::BadRequest(
                "simultaneous_stream_limit must be non-negative".to_string(),
            ));
        }
        policy.simultaneous_stream_limit = Some(limit);
    }
    if let Some(disabled) = req.disabled {
        policy.is_disabled = Some(disabled);
    }
    Ok(())
}

impl From<EmbyUser> for UserSummary {
    fn from(user: EmbyUser) -> Self {
        let remote_bitrate_mbps = user
            .policy
            .remote_client_bitrate_limit
            .map(|bps| bps as f64 / BPS_PER_MBPS);
        Self {
            id: user.id,
            name: user.name,
            disabled: user.policy.is_disabled.unwrap_or(false),
            last_activity_date: user.last_activity_date,
            policy: UserPolicySummary {
                remote_client_bitrate_limit: user.policy.remote_client_bitrate_limit,
                simultaneous_stream_limit: user.policy.simultaneous_stream_limit,
            },
            remote_bitrate_mbps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{UpdateUserPolicyRequest, merge_policy};
    use crate::{emby::EmbyUserPolicy, error::AppError};

    #[test]
    fn merge_policy_preserves_unknown_fields_and_converts_mbps() {
        let mut policy = EmbyUserPolicy {
            is_disabled: Some(false),
            remote_client_bitrate_limit: Some(1_000_000),
            simultaneous_stream_limit: Some(2),
            extra: serde_json::json!({"SomeFuturePolicy": true})
                .as_object()
                .unwrap()
                .clone(),
        };

        merge_policy(
            &mut policy,
            UpdateUserPolicyRequest {
                remote_bitrate_mbps: Some(12.5),
                simultaneous_stream_limit: Some(3),
                disabled: Some(true),
            },
        )
        .unwrap();

        assert_eq!(policy.remote_client_bitrate_limit, Some(12_500_000));
        assert_eq!(policy.simultaneous_stream_limit, Some(3));
        assert_eq!(policy.is_disabled, Some(true));
        assert_eq!(policy.extra["SomeFuturePolicy"], true);
    }

    #[test]
    fn missing_api_key_is_bad_request() {
        let err = super::ensure_api_key_configured(" \t").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
        assert!(err.to_string().contains("api_key is not configured"));
    }
}
