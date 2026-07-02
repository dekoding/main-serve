/// Token revocation and user registration/login handlers.
use axum::{
    Json,
    extract::{Request, State},
    http::StatusCode,
    response::IntoResponse,
};
use http_body_util::BodyExt;

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};

use crate::config::types::{JwtAlgorithm, JwtConfig};
use crate::error::AppError;
use crate::middleware::auth::validators::jwt::create_token;
use crate::server::state::AppState;
use crate::server::state::RevocationStoreBackend;
use crate::{
    db::query::select_one::{
        build_insert_user, build_select_by_field, build_select_user_for_login, build_table_columns,
    },
    handlers::common::get_registration_pool,
};
use serde::Deserialize;

/// Handle `POST /_main-serve/auth/revoke`.
///
/// Revokes the current token by extracting its `jti` claim and adding it
/// to the revocation store.
///
/// # Errors
///
/// Returns `AppError::Auth` if the request is unauthenticated, the token
/// has no `jti` claim, or JWT is not configured.
pub async fn handle_revoke(
    State(state): State<AppState>,
    req: Request,
) -> Result<impl IntoResponse, AppError> {
    let token = {
        let auth_header = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok());
        auth_header
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(ToString::to_string)
            .ok_or_else(|| AppError::Auth("No token provided".to_string()))?
    };
    // Drain the request body to avoid connection issues
    let _ = req.into_body().collect().await.map(|b| b.to_bytes());

    let (secret, algorithm, issuer, audience) = {
        let config = state.config.read().await;
        let jwt_config = config
            .auth
            .jwt
            .as_ref()
            .ok_or_else(|| AppError::Config("JWT is not configured".to_string()))?;
        (
            jwt_config.secret.clone(),
            jwt_config.algorithm,
            jwt_config.issuer.clone(),
            jwt_config.audience.clone(),
        )
    };

    let algorithm = match algorithm {
        JwtAlgorithm::HS256 => jsonwebtoken::Algorithm::HS256,
        JwtAlgorithm::HS384 => jsonwebtoken::Algorithm::HS384,
        JwtAlgorithm::HS512 => jsonwebtoken::Algorithm::HS512,
        JwtAlgorithm::RS256 => jsonwebtoken::Algorithm::RS256,
        JwtAlgorithm::RS384 => jsonwebtoken::Algorithm::RS384,
        JwtAlgorithm::RS512 => jsonwebtoken::Algorithm::RS512,
        JwtAlgorithm::ES256 => jsonwebtoken::Algorithm::ES256,
        JwtAlgorithm::ES384 => jsonwebtoken::Algorithm::ES384,
    };
    let key = jsonwebtoken::DecodingKey::from_secret(secret.as_bytes());

    let mut validation = jsonwebtoken::Validation::new(algorithm);
    if issuer.is_empty() {
        validation.set_issuer::<String>(&[]);
    } else {
        validation.set_issuer(&[&issuer]);
    }
    if audience.is_empty() {
        validation.set_audience::<String>(&[]);
    } else {
        validation.set_audience(&[&audience]);
    }
    validation.validate_exp = false;
    validation.validate_nbf = false;

    let token_data = jsonwebtoken::decode::<serde_json::Value>(&token, &key, &validation)
        .map_err(|e| AppError::Auth(format!("Failed to decode token for revocation: {e}")))?;

    let claims = &token_data.claims;

    let jti = claims
        .get("jti")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Token has no jti claim".to_string()))?;

    let exp = claims
        .get("exp")
        .and_then(|v| v.as_u64())
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(0);

    if let Some(store) = state.revocation_store.get() {
        let expires_at = std::time::Instant::now()
            .checked_add(std::time::Duration::from_secs(exp as u64))
            .unwrap_or_else(|| std::time::Instant::now() + std::time::Duration::from_secs(3600));
        store.revoke(jti, expires_at).await;
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "message": "Token revoked" })),
    ))
}

/// Request body for user registration.
#[derive(Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
}

/// Request body for user login.
#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Response body for auth endpoints.
#[derive(serde::Serialize)]
pub struct AuthResponse {
    pub token: String,
}

/// Get JWT configuration from app state.
///
/// # Errors
///
/// Returns `AppError::Config` if JWT is not configured.
async fn get_jwt_config(state: &AppState) -> Result<JwtConfig, AppError> {
    let config = state.config.read().await;
    let jwt_config = config
        .auth
        .jwt
        .as_ref()
        .ok_or_else(|| AppError::Config("JWT is not configured".to_string()))?;
    Ok(JwtConfig {
        secret: jwt_config.secret.clone(),
        algorithm: jwt_config.algorithm,
        issuer: jwt_config.issuer.clone(),
        audience: String::new(),
        expiry: 3600,
        role_claim: "role".to_string(),
        revocation: None,
    })
}

/// Create a JWT token for a user.
///
/// # Errors
///
/// Returns `AppError::Auth` if token creation fails.
fn create_jwt_token(
    user_id: &str,
    role: Option<&str>,
    jwt_config: &JwtConfig,
    jti: &str,
    user_email: &str,
) -> Result<String, AppError> {
    create_token(user_id, role, jwt_config, Some(jti), Some(user_email))
}

/// Handle `POST /_main-serve/register`.
///
/// Creates a new user in the database with an argon2id-hashed password,
/// then returns a JWT token.
///
/// # Errors
///
/// Returns `AppError::Config` if registration is not configured or enabled.
/// Returns `AppError::BadRequest` if the email is already registered
/// or the password is too short. Returns `AppError::Internal` for
/// database or hashing failures.
pub async fn handle_register(
    State(state): State<AppState>,
    Json(body): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AppError> {
    let (pool, register_config) = get_registration_pool(State(state.clone())).await?;

    if !register_config.enabled {
        return Err(AppError::Config(
            "User registration is not enabled".to_string(),
        ));
    }

    if body.password.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters".to_string(),
        ));
    }

    let password_hash = hash_password(&body.password)?;

    let driver = pool.driver();
    let table_name = &register_config.table;

    // Introspect table columns to determine if timestamps should be included.
    let built = build_table_columns(table_name, driver);
    let column_rows = pool
        .fetch_all_json(&built.sql, &built.params)
        .await
        .map_err(|e| AppError::Internal(format!("Database query error: {e}")))?;

    let col_names: Vec<String> = column_rows
        .iter()
        .filter_map(|row| {
            row.get("name")
                .or_else(|| row.get("column_name"))
                .or_else(|| row.get("COLUMN_NAME"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned())
        })
        .collect();

    let include_timestamps = col_names.contains(&"created_at".to_string())
        && col_names.contains(&"updated_at".to_string());

    // Check email uniqueness using builder.
    let built = build_select_by_field(
        table_name,
        &["email"],
        "email",
        body.email.clone().into(),
        driver,
    )
    .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
    let existing = pool
        .fetch_optional_json(&built.sql, &built.params)
        .await
        .map_err(|e| AppError::Internal(format!("Database query error: {e}")))?;

    if existing.is_some() {
        return Err(AppError::BadRequest("Email already registered".to_string()));
    }

    // Insert the new user using the builder.
    let built = build_insert_user(
        table_name,
        &body.email,
        &password_hash,
        &register_config.default_role,
        include_timestamps,
        driver,
    )
    .map_err(|e| AppError::Internal(format!("Failed to build insert query: {e}")))?;

    pool.execute_with_params(&built.sql, &built.params)
        .await
        .map_err(|e| AppError::Internal(format!("Database insert error: {e}")))?;

    // Fetch the newly created user to get their ID.
    let built = build_select_by_field(
        table_name,
        &["id", "email", "role"],
        "email",
        body.email.clone().into(),
        driver,
    )
    .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
    let user_row = pool
        .fetch_optional_json(&built.sql, &built.params)
        .await
        .map_err(|e| AppError::Internal(format!("Database query error: {e}")))?
        .ok_or_else(|| AppError::Internal("Failed to retrieve newly created user".to_string()))?;

    let user_id = user_row
        .get("id")
        .and_then(|v| v.as_i64())
        .map(|id| id.to_string())
        .unwrap_or_else(|| {
            // Fallback: use email as user_id if ID can't be determined
            body.email.clone()
        });
    let user_email = user_row
        .get("email")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or(body.email.clone());

    let jwt_config = get_jwt_config(&state).await?;
    let jti = uuid::Uuid::new_v4().to_string();

    let token = create_jwt_token(
        &user_id,
        Some(&register_config.default_role),
        &jwt_config,
        &jti,
        &user_email,
    )?;

    Ok((StatusCode::CREATED, Json(AuthResponse { token })))
}

/// Handle `POST /_main-serve/login`.
///
/// Verifies the user's password against the stored argon2id hash and
/// returns a JWT token.
///
/// # Errors
///
/// Returns `AppError::Auth` if the email is not found or the password is wrong.
/// Returns `AppError::Internal` for database or hashing failures.
pub async fn handle_login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    let (pool, _) = get_registration_pool(State(state.clone())).await?;

    let driver = pool.driver();

    let built = build_select_user_for_login("users", driver)
        .map_err(|e| AppError::Internal(format!("Failed to build query: {e}")))?;
    let row = pool
        .fetch_optional_json(&built.sql, &[body.email.clone().into()])
        .await
        .map_err(|e| AppError::Internal(format!("Database query error: {e}")))?;

    let (email, password_hash, role, user_id) = match row {
        Some(row) => {
            let email = row
                .get("email")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .ok_or_else(|| AppError::Auth("Invalid user record".to_string()))?;
            let password_hash = row
                .get("password_hash")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .ok_or_else(|| AppError::Auth("Invalid user record".to_string()))?;
            let role = row
                .get("role")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let uid = row
                .get("id")
                .and_then(|v| v.as_i64())
                .map(|id| id.to_string())
                .unwrap_or(email.clone());
            (email, password_hash, role, uid)
        }
        None => return Err(AppError::Auth("Invalid email or password".to_string())),
    };

    if !verify_password(&body.password, &password_hash)? {
        return Err(AppError::Auth("Invalid email or password".to_string()));
    }

    let jwt_config = get_jwt_config(&state).await?;
    let jti = uuid::Uuid::new_v4().to_string();

    let token = create_jwt_token(&user_id, role.as_deref(), &jwt_config, &jti, &email)?;

    Ok(Json(AuthResponse { token }))
}

/// Hash a password using argon2id with a random salt.
///
/// # Errors
///
/// Returns `AppError::Internal` if the hashing operation fails.
fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hashing error: {e}")))?;

    Ok(hash.to_string())
}

/// Verify a password against an argon2id hash.
///
/// # Errors
///
/// Returns `AppError::Internal` if the hash string is malformed.
/// Returns `Ok(false)` if the password does not match.
fn verify_password(password: &str, hash: &str) -> Result<bool, AppError> {
    let parsed_hash = PasswordHash::new(hash)
        .map_err(|_| AppError::Internal("Invalid password hash".to_string()))?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed_hash) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(AppError::Internal(format!(
            "Password verification error: {e}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_password_produces_valid_argon2_hash() {
        let hash = hash_password("testpassword123").unwrap();
        assert!(!hash.is_empty());
        assert!(hash.starts_with("$argon2id$"));
    }

    #[test]
    fn test_verify_password_correct_password() {
        let hash = hash_password("correctpassword").unwrap();
        assert!(verify_password("correctpassword", &hash).unwrap());
    }

    #[test]
    fn test_verify_password_wrong_password() {
        let hash = hash_password("correctpassword").unwrap();
        assert!(!verify_password("wrongpassword", &hash).unwrap());
    }

    #[test]
    fn test_hash_password_different_salts_produce_different_hashes() {
        let hash1 = hash_password("samepassword").unwrap();
        let hash2 = hash_password("samepassword").unwrap();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_hash_verify_roundtrip() {
        let passwords = [
            "short1234",
            "longerpassword123",
            "withspecial!@#$%",
            "unicode_test_test",
        ];
        for pwd in passwords {
            let hash = hash_password(pwd).unwrap();
            assert!(
                verify_password(pwd, &hash).unwrap(),
                "Password '{pwd}' should verify"
            );
            assert!(
                !verify_password("wrongpassword", &hash).unwrap(),
                "Wrong password should not verify for hash of '{pwd}'"
            );
        }
    }

    #[test]
    fn test_verify_password_malformed_hash() {
        let result = verify_password("testpassword", "not-a-valid-hash");
        assert!(result.is_err(), "Malformed hash should produce an error");
    }

    #[test]
    fn test_hash_password_empty_string() {
        let hash = hash_password("").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("", &hash).unwrap());
        assert!(!verify_password("notempty", &hash).unwrap());
    }
}
