/// Authentication provider configurations.
use std::fmt;

use serde::Deserialize;

/// Top-level auth configuration - defines available auth providers.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    /// JWT authentication configuration.
    pub jwt: Option<JwtConfig>,
    /// API key authentication configuration.
    pub api_key: Option<ApiKeyConfig>,
    /// HTTP Basic authentication configuration.
    pub basic: Option<BasicAuthConfig>,
    /// OAuth2/OIDC authentication configuration.
    pub oauth2: Option<OAuth2Config>,
}

/// JWT authentication provider configuration.
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            secret: String::new(),
            algorithm: JwtAlgorithm::HS256,
            issuer: "main-serve".to_string(),
            audience: String::new(),
            expiry: 3600,
            role_claim: "role".to_string(),
        }
    }
}

/// Supported JWT signing algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
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
pub enum ApiKeyLocation {
    Header,
    Query,
}

/// A single API key with an optional role.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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
pub struct BasicAuthConfig {
    /// HTTP realm for WWW-Authenticate challenges.
    pub realm: String,
    /// List of valid users.
    #[serde(default)]
    pub users: Vec<BasicAuthUser>,
}

impl Default for BasicAuthConfig {
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
pub struct BasicAuthUser {
    /// Username.
    pub username: String,
    /// Argon2-hashed password.
    pub password_hash: String,
    /// Optional role assigned to this user.
    #[serde(default)]
    pub role: Option<String>,
}

/// OAuth2/OIDC authentication provider configuration.
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
}

fn default_state_ttl() -> u64 {
    300 // 5 minutes
}

fn default_max_pending_states() -> usize {
    1000
}

impl Default for OAuth2Config {
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
        }
    }
}

impl fmt::Debug for JwtConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JwtConfig")
            .field("secret", &"[REDACTED]")
            .field("algorithm", &self.algorithm)
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("expiry", &self.expiry)
            .field("role_claim", &self.role_claim)
            .finish()
    }
}

impl fmt::Debug for ApiKeyEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKeyEntry")
            .field("key", &"[REDACTED]")
            .field("role", &self.role)
            .finish()
    }
}

impl fmt::Debug for BasicAuthUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BasicAuthUser")
            .field("username", &self.username)
            .field("password_hash", &"[REDACTED]")
            .field("role", &self.role)
            .finish()
    }
}

impl fmt::Debug for OAuth2Config {
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
            .finish()
    }
}
