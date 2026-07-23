/// Authentication provider configurations.
use std::fmt;

use serde::Deserialize;

/// Password hashing algorithm for user registration.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
/// PasswordHashAlgorithm
pub enum PasswordHashAlgorithm {
    #[default]
    Argon2id,
}

/// User registration configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// RegisterConfig
pub struct RegisterConfig {
    /// Whether registration is enabled.
    pub enabled: bool,
    /// Database table to create users in.
    pub table: String,
    /// Named database to use.
    pub database: String,
    /// Default role assigned to newly registered users.
    pub default_role: String,
    /// Password hashing algorithm.
    #[serde(default)]
    pub password_hash: PasswordHashAlgorithm,
}

impl Default for RegisterConfig {
    /// Returns a RegisterConfig with registration disabled by default.
    fn default() -> Self {
        Self {
            enabled: false,
            table: String::new(),
            database: String::new(),
            default_role: "user".to_string(),
            password_hash: PasswordHashAlgorithm::Argon2id,
        }
    }
}

/// Top-level auth configuration - defines available auth providers.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// AuthConfig
pub struct AuthConfig {
    /// JWT authentication configuration.
    pub jwt: Option<JwtConfig>,
    /// API key authentication configuration.
    pub api_key: Option<ApiKeyConfig>,
    /// HTTP Basic authentication configuration.
    pub basic: Option<BasicAuthConfig>,
    /// OAuth2/OIDC authentication configuration.
    pub oauth2: Option<OAuth2Config>,
    /// User registration configuration.
    #[serde(default)]
    pub register: Option<RegisterConfig>,
}

/// Token revocation store type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
/// RevocationStoreType
pub enum RevocationStoreType {
    InMemory,
    Database,
}

/// Configuration for JWT token revocation.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// JwtRevocationConfig
pub struct JwtRevocationConfig {
    /// Store type: "in_memory" or "database".
    pub store: RevocationStoreType,
    /// Database table name for the revocation store (only when store is "database").
    /// Defaults to "token_blacklist".
    pub db_table: Option<String>,
    /// Interval in seconds between cleanup runs for expired revocation entries.
    /// Only applicable for "database" store.
    pub cleanup_interval_secs: Option<u64>,
}

impl Default for JwtRevocationConfig {
    /// Returns a JwtRevocationConfig with in-memory store and 1-hour cleanup interval.
    fn default() -> Self {
        Self {
            store: RevocationStoreType::InMemory,
            db_table: Some("token_blacklist".to_string()),
            cleanup_interval_secs: Some(3600),
        }
    }
}

/// JWT authentication provider configuration.
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// JwtConfig
pub struct JwtConfig {
    /// HMAC secret or RSA/EC key material.
    pub secret: String,
    /// Signing algorithm.
    pub algorithm: JwtAlgorithm,
    /// Expected `iss` claim (empty = no validation).
    pub issuer: String,
    /// Expected `aud` claim (empty = no validation).
    pub audience: String,
    /// Token lifetime in seconds.
    pub expiry: u64,
    /// Name of the JWT claim that contains the user's role.
    pub role_claim: String,
    /// Token revocation configuration. When set, JWTs with a `jti` claim
    /// are checked against the revocation store.
    #[serde(default)]
    pub revocation: Option<JwtRevocationConfig>,
}

impl Default for JwtConfig {
    /// Returns a JwtConfig with HS256 signing and a 1-hour token expiry.
    fn default() -> Self {
        Self {
            secret: String::new(),
            algorithm: JwtAlgorithm::HS256,
            issuer: "main-serve".to_string(),
            audience: String::new(),
            expiry: 3600,
            role_claim: "role".to_string(),
            revocation: None,
        }
    }
}

/// Supported JWT signing algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
/// JwtAlgorithm
pub enum JwtAlgorithm {
    HS256,
    HS384,
    HS512,
    RS256,
    RS384,
    RS512,
    ES256,
    ES384,
}

/// API key authentication provider configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// ApiKeyConfig
pub struct ApiKeyConfig {
    /// Where to look for the API key.
    pub location: ApiKeyLocation,
    /// Header name or query parameter name.
    pub name: String,
    /// List of valid API keys.
    #[serde(default)]
    pub keys: Vec<ApiKeyEntry>,
}

impl Default for ApiKeyConfig {
    /// Returns an ApiKeyConfig with header-based lookup and `X-API-Key` as the default header.
    fn default() -> Self {
        Self {
            location: ApiKeyLocation::Header,
            name: "X-API-Key".to_string(),
            keys: Vec::new(),
        }
    }
}

/// Where to look for the API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
/// ApiKeyLocation
pub enum ApiKeyLocation {
    Header,
    Query,
}

/// A single API key with an optional role.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
/// ApiKeyEntry
pub struct ApiKeyEntry {
    /// The API key value.
    pub key: String,
    /// Optional role assigned to this key.
    #[serde(default)]
    pub role: Option<String>,
}

/// HTTP Basic authentication provider configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// BasicAuthConfig
pub struct BasicAuthConfig {
    /// HTTP realm for WWW-Authenticate challenges.
    pub realm: String,
    /// List of valid users.
    #[serde(default)]
    pub users: Vec<BasicAuthUser>,
}

impl Default for BasicAuthConfig {
    /// Returns a BasicAuthConfig with "main-serve" as the default authentication realm.
    fn default() -> Self {
        Self {
            realm: "main-serve".to_string(),
            users: Vec::new(),
        }
    }
}

/// A user for HTTP Basic authentication.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
/// BasicAuthUser
pub struct BasicAuthUser {
    /// Username.
    pub username: String,
    /// Argon2-hashed password.
    pub password_hash: String,
    /// Optional role assigned to this user.
    #[serde(default)]
    pub role: Option<String>,
}

/// OAuth2 role mapping match mode.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
/// RoleMatchMode
pub enum RoleMatchMode {
    #[default]
    Exact,
    Contains,
}

/// Configuration for mapping IdP roles/groups to Main Serve roles.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// RoleMappingConfig
pub struct RoleMappingConfig {
    /// Default Main Serve role when no IdP role maps.
    pub default_role: String,
    /// Name of the claim in the userinfo response containing IdP roles/groups.
    /// Defaults to "groups".
    #[serde(default = "default_role_claim")]
    pub role_claim: String,
    /// Mapping from IdP role/group values to Main Serve roles.
    #[serde(default)]
    pub role_map: std::collections::HashMap<String, String>,
    /// How to match IdP roles against the role_map keys.
    #[serde(default)]
    pub match_mode: RoleMatchMode,
}

/// Returns "groups" as the default IdP role claim name.
fn default_role_claim() -> String {
    "groups".to_string()
}

impl Default for RoleMappingConfig {
    /// Returns a RoleMappingConfig with exact matching and "user" as the default role.
    fn default() -> Self {
        Self {
            default_role: "user".to_string(),
            role_claim: default_role_claim(),
            role_map: std::collections::HashMap::new(),
            match_mode: RoleMatchMode::default(),
        }
    }
}

/// OAuth2/OIDC authentication provider configuration.
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// OAuth2Config
pub struct OAuth2Config {
    /// Provider name (informational only).
    pub provider: String,
    /// `IdP` authorization endpoint URL.
    pub authorization_url: String,
    /// `IdP` token exchange endpoint URL.
    pub token_url: String,
    /// `IdP` userinfo endpoint URL (used for token introspection).
    pub userinfo_url: String,
    /// `OAuth2` client ID.
    pub client_id: String,
    /// `OAuth2` client secret.
    pub client_secret: String,
    /// Requested scopes.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Redirect URL for the authorization code callback.
    pub redirect_url: String,
    /// URL to redirect to after successful authentication.
    pub success_url: String,
    /// Name of the cookie used to store the minted JWT.
    pub cookie_name: String,
    /// Maximum lifetime (in seconds) for a pending `OAuth2` authorization state.
    #[serde(default = "default_state_ttl")]
    pub state_ttl: u64,
    /// Maximum number of pending `OAuth2` authorization flows allowed simultaneously.
    #[serde(default = "default_max_pending_states")]
    pub max_pending_states: usize,
    /// OAuth2 role mapping configuration. When set, IdP roles/groups from
    /// the userinfo response are mapped to Main Serve roles.
    #[serde(default)]
    pub role_mapping: Option<RoleMappingConfig>,
}

/// Returns 300 seconds (5 minutes) as the default OAuth2 authorization state TTL.
fn default_state_ttl() -> u64 {
    300 // 5 minutes
}

/// Returns 1000 as the default maximum number of concurrent pending OAuth2 authorization states.
fn default_max_pending_states() -> usize {
    1000
}

impl Default for OAuth2Config {
    /// Returns an OAuth2Config with a 5-minute state TTL and no role mapping.
    fn default() -> Self {
        Self {
            provider: "generic".to_string(),
            authorization_url: String::new(),
            token_url: String::new(),
            userinfo_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            scopes: vec!["openid".to_string()],
            redirect_url: String::new(),
            success_url: "/".to_string(),
            cookie_name: "main_serve_token".to_string(),
            state_ttl: default_state_ttl(),
            max_pending_states: default_max_pending_states(),
            role_mapping: None,
        }
    }
}

impl fmt::Debug for JwtConfig {
    /// Formats the struct with the secret field redacted.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JwtConfig")
            .field("secret", &"[REDACTED]")
            .field("algorithm", &self.algorithm)
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("expiry", &self.expiry)
            .field("role_claim", &self.role_claim)
            .field("revocation", &self.revocation)
            .finish()
    }
}

impl fmt::Debug for ApiKeyEntry {
    /// Formats the struct with the key field redacted.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKeyEntry")
            .field("key", &"[REDACTED]")
            .field("role", &self.role)
            .finish()
    }
}

impl fmt::Debug for BasicAuthUser {
    /// Formats the struct with the password_hash field redacted.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BasicAuthUser")
            .field("username", &self.username)
            .field("password_hash", &"[REDACTED]")
            .field("role", &self.role)
            .finish()
    }
}

impl fmt::Debug for OAuth2Config {
    /// Formats the struct with the client_secret field redacted.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuth2Config")
            .field("provider", &self.provider)
            .field("authorization_url", &self.authorization_url)
            .field("token_url", &self.token_url)
            .field("userinfo_url", &self.userinfo_url)
            .field("client_id", &self.client_id)
            .field("client_secret", &"[REDACTED]")
            .field("scopes", &self.scopes)
            .field("redirect_url", &self.redirect_url)
            .field("success_url", &self.success_url)
            .field("cookie_name", &self.cookie_name)
            .field("state_ttl", &self.state_ttl)
            .field("max_pending_states", &self.max_pending_states)
            .field("role_mapping", &self.role_mapping)
            .finish()
    }
}
