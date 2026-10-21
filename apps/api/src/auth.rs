use std::{env, error::Error, fmt, time::Duration};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use reqwest::{Client, Url, redirect::Policy};
use serde::Deserialize;
use tokio::{sync::Mutex, time::Instant};

const MAX_TOKEN_BYTES: usize = 8 * 1024;
const MAX_JWKS_BYTES: usize = 256 * 1024;
const MAX_JWKS_KEYS: usize = 32;
const JWKS_TTL: Duration = Duration::from_secs(300);
// An unknown key id is not a reason to fetch on every request. Refreshes are
// coalesced to one per cooldown window, so a caller cannot turn an attacker
// chosen `kid` into unbounded auth-provider traffic.
const JWKS_REFRESH_COOLDOWN: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedIdentity {
    pub issuer: String,
    pub subject: String,
}

#[derive(Debug)]
pub enum AuthConfigError {
    MissingPair,
    InvalidUrl,
    ClientConfiguration,
}

impl fmt::Display for AuthConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPair => formatter.write_str(
                "NEON_AUTH_BASE_URL and NEON_AUTH_JWKS_URL must be configured together",
            ),
            Self::InvalidUrl => formatter.write_str(
                "Neon Auth base and JWKS URLs must be valid HTTPS URLs (loopback HTTP is allowed locally)",
            ),
            Self::ClientConfiguration => {
                formatter.write_str("Neon Auth HTTP client could not be configured")
            }
        }
    }
}

impl Error for AuthConfigError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenError {
    Invalid,
    Unavailable,
}

#[derive(Clone)]
pub struct NeonJwtVerifier {
    issuer: String,
    jwks_url: Url,
    client: Client,
    cache: std::sync::Arc<Mutex<JwksCache>>,
    refresh_cooldown: Duration,
}

#[derive(Clone, Default)]
struct JwksCache {
    keys: Option<JwkSet>,
    expires_at: Option<Instant>,
    last_attempt: Option<Instant>,
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
}

impl NeonJwtVerifier {
    pub fn from_env() -> Result<Option<Self>, AuthConfigError> {
        let base_url = env::var("NEON_AUTH_BASE_URL").ok();
        let jwks_url = env::var("NEON_AUTH_JWKS_URL").ok();

        match (base_url, jwks_url) {
            (None, None) => Ok(None),
            (Some(base_url), Some(jwks_url)) => {
                Self::new(base_url.trim(), jwks_url.trim()).map(Some)
            }
            _ => Err(AuthConfigError::MissingPair),
        }
    }

    pub fn new(base_url: &str, jwks_url: &str) -> Result<Self, AuthConfigError> {
        let base_url = Url::parse(base_url).map_err(|_| AuthConfigError::InvalidUrl)?;
        let jwks_url = Url::parse(jwks_url).map_err(|_| AuthConfigError::InvalidUrl)?;
        if !is_allowed_auth_url(&base_url)
            || base_url.query().is_some()
            || base_url.fragment().is_some()
            || !is_allowed_auth_url(&jwks_url)
            || jwks_url.fragment().is_some()
        {
            return Err(AuthConfigError::InvalidUrl);
        }

        let issuer = base_url.origin().ascii_serialization();
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(3))
            .redirect(Policy::none())
            .build()
            .map_err(|_| AuthConfigError::ClientConfiguration)?;

        Ok(Self {
            issuer,
            jwks_url,
            client,
            cache: std::sync::Arc::new(Mutex::new(JwksCache::default())),
            refresh_cooldown: JWKS_REFRESH_COOLDOWN,
        })
    }

    pub async fn verify(&self, token: &str) -> Result<AuthenticatedIdentity, TokenError> {
        if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
            return Err(TokenError::Invalid);
        }

        let header = decode_header(token).map_err(|_| TokenError::Invalid)?;
        if header.alg != Algorithm::EdDSA {
            return Err(TokenError::Invalid);
        }
        let key_id = header.kid.as_deref().ok_or(TokenError::Invalid)?;
        if key_id.is_empty() || key_id.len() > 256 {
            return Err(TokenError::Invalid);
        }
        let keys = self.jwks_for(key_id).await?;
        verify_token_with_keys(token, &keys, &self.issuer)
    }

    async fn jwks_for(&self, key_id: &str) -> Result<JwkSet, TokenError> {
        let now = Instant::now();
        {
            let cache = self.cache.lock().await;
            if let Some(keys) = cache.keys.as_ref() {
                let fresh = cache.expires_at.is_some_and(|expires| expires > now);
                let has_key = keys.find(key_id).is_some();
                let recently_attempted = cache
                    .last_attempt
                    .is_some_and(|attempt| attempt + self.refresh_cooldown > now);
                if fresh && (has_key || recently_attempted) {
                    return Ok(keys.clone());
                }
            }
            if cache
                .last_attempt
                .is_some_and(|attempt| attempt + self.refresh_cooldown > now)
            {
                return cache.keys.clone().ok_or(TokenError::Unavailable);
            }
        }

        // Claim this refresh window before doing network I/O. Concurrent callers
        // observe the claim and use the current key set instead of queueing
        // another fetch behind a shared lock.
        {
            let mut cache = self.cache.lock().await;
            if cache
                .last_attempt
                .is_some_and(|attempt| attempt + self.refresh_cooldown > Instant::now())
            {
                return cache.keys.clone().ok_or(TokenError::Unavailable);
            }
            cache.last_attempt = Some(Instant::now());
        }

        let fetched = self.fetch_jwks().await;
        let mut cache = self.cache.lock().await;
        match fetched {
            Ok(keys) => {
                cache.keys = Some(keys.clone());
                cache.expires_at = Some(Instant::now() + JWKS_TTL);
                Ok(keys)
            }
            // A transient provider failure should not invalidate a key set that
            // already verified a token successfully.
            Err(error) => cache.keys.clone().ok_or(error),
        }
    }

    async fn fetch_jwks(&self) -> Result<JwkSet, TokenError> {
        let response = self
            .client
            .get(self.jwks_url.clone())
            .send()
            .await
            .map_err(|_| TokenError::Unavailable)?;
        if !response.status().is_success() {
            return Err(TokenError::Unavailable);
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_JWKS_BYTES as u64)
        {
            return Err(TokenError::Unavailable);
        }
        let initial_capacity = response
            .content_length()
            .unwrap_or(4096)
            .min(MAX_JWKS_BYTES as u64) as usize;
        let mut body = Vec::with_capacity(initial_capacity);
        let mut response = response;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| TokenError::Unavailable)?
        {
            if body.len().saturating_add(chunk.len()) > MAX_JWKS_BYTES {
                return Err(TokenError::Unavailable);
            }
            body.extend_from_slice(&chunk);
        }
        let keys: JwkSet = serde_json::from_slice(&body).map_err(|_| TokenError::Unavailable)?;
        if keys.keys.is_empty() || keys.keys.len() > MAX_JWKS_KEYS {
            return Err(TokenError::Unavailable);
        }
        Ok(keys)
    }
}

fn verify_token_with_keys(
    token: &str,
    keys: &JwkSet,
    issuer: &str,
) -> Result<AuthenticatedIdentity, TokenError> {
    let header = decode_header(token).map_err(|_| TokenError::Invalid)?;
    if header.alg != Algorithm::EdDSA {
        return Err(TokenError::Invalid);
    }
    let key_id = header.kid.as_deref().ok_or(TokenError::Invalid)?;
    if key_id.is_empty() || key_id.len() > 256 {
        return Err(TokenError::Invalid);
    }
    let jwk = keys.find(key_id).ok_or(TokenError::Invalid)?;
    let decoding_key = DecodingKey::from_jwk(jwk).map_err(|_| TokenError::Invalid)?;

    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_issuer(&[issuer]);
    validation.required_spec_claims.insert("iss".to_owned());
    validation.required_spec_claims.insert("sub".to_owned());
    validation.validate_nbf = true;
    // Neon Managed Auth tokens are issuer-scoped and do not expose a configurable audience.
    validation.validate_aud = false;

    let claims = decode::<Claims>(token, &decoding_key, &validation)
        .map_err(|_| TokenError::Invalid)?
        .claims;
    if claims.sub.trim().is_empty() || claims.sub.len() > 256 {
        return Err(TokenError::Invalid);
    }

    Ok(AuthenticatedIdentity {
        issuer: issuer.to_owned(),
        subject: claims.sub,
    })
}

fn is_allowed_auth_url(url: &Url) -> bool {
    if url.username() != "" || url.password().is_some() {
        return false;
    }
    if url.scheme() == "https" {
        return true;
    }
    if url.scheme() != "http" {
        return false;
    }

    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode, jwk::JwkSet};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use serde::Serialize;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::{NeonJwtVerifier, TokenError, verify_token_with_keys};

    const ISSUER: &str = "https://auth.example.test";
    const KID: &str = "test-key-1";
    const SEED: [u8; 32] = [42; 32];

    #[derive(Serialize)]
    struct TestClaims {
        sub: String,
        iss: String,
        exp: usize,
    }

    fn test_keys() -> (JwkSet, EncodingKey) {
        let pair = Ed25519KeyPair::from_seed_unchecked(&SEED).expect("test seed is valid");
        let public_key = URL_SAFE_NO_PAD.encode(pair.public_key().as_ref());
        let jwks = serde_json::from_value(serde_json::json!({
            "keys": [{
                "kty": "OKP",
                "crv": "Ed25519",
                "alg": "EdDSA",
                "use": "sig",
                "kid": KID,
                "x": public_key
            }]
        }))
        .expect("test JWKS is valid");

        let mut private_key_der = vec![
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        private_key_der.extend_from_slice(&SEED);
        (jwks, EncodingKey::from_ed_der(&private_key_der))
    }

    fn test_token(encoding_key: &EncodingKey, issuer: &str, expires_at: usize) -> String {
        test_token_with_kid(encoding_key, issuer, expires_at, KID)
    }

    fn test_token_with_kid(
        encoding_key: &EncodingKey,
        issuer: &str,
        expires_at: usize,
        key_id: &str,
    ) -> String {
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(key_id.to_owned());
        encode(
            &header,
            &TestClaims {
                sub: "neon-user-123".to_owned(),
                iss: issuer.to_owned(),
                exp: expires_at,
            },
            encoding_key,
        )
        .expect("test token signs")
    }

    fn verify_with_keys(token: &str, keys: &JwkSet, issuer: &str) -> Result<String, TokenError> {
        verify_token_with_keys(token, keys, issuer).map(|identity| identity.subject)
    }

    #[test]
    fn accepts_a_valid_issuer_scoped_eddsa_token() {
        let (keys, signing_key) = test_keys();
        let token = test_token(
            &signing_key,
            ISSUER,
            jsonwebtoken::get_current_timestamp() as usize + 60,
        );

        assert_eq!(
            verify_with_keys(&token, &keys, ISSUER),
            Ok("neon-user-123".to_owned())
        );
    }

    #[test]
    fn rejects_wrong_issuer_expiry_and_signature() {
        let (keys, signing_key) = test_keys();
        let wrong_issuer = test_token(
            &signing_key,
            "https://other.example.test",
            jsonwebtoken::get_current_timestamp() as usize + 60,
        );
        let expired = test_token(&signing_key, ISSUER, 1);
        let mut invalid_signature = test_token(
            &signing_key,
            ISSUER,
            jsonwebtoken::get_current_timestamp() as usize + 60,
        );
        invalid_signature.push('x');

        assert_eq!(
            verify_with_keys(&wrong_issuer, &keys, ISSUER),
            Err(TokenError::Invalid)
        );
        assert_eq!(
            verify_with_keys(&expired, &keys, ISSUER),
            Err(TokenError::Invalid)
        );
        assert_eq!(
            verify_with_keys(&invalid_signature, &keys, ISSUER),
            Err(TokenError::Invalid)
        );
    }

    #[test]
    fn rejects_algorithm_confusion_and_unknown_key_ids() {
        let (keys, signing_key) = test_keys();
        let now = jsonwebtoken::get_current_timestamp() as usize + 60;
        let mut wrong_algorithm_header = Header::new(Algorithm::HS256);
        wrong_algorithm_header.kid = Some(KID.to_owned());
        let wrong_algorithm = encode(
            &wrong_algorithm_header,
            &TestClaims {
                sub: "neon-user-123".to_owned(),
                iss: ISSUER.to_owned(),
                exp: now,
            },
            &EncodingKey::from_secret(b"not-a-neon-key"),
        )
        .expect("test HMAC token signs");
        let mut unknown_kid_header = Header::new(Algorithm::EdDSA);
        unknown_kid_header.kid = Some("unknown-key".to_owned());
        let unknown_kid = encode(
            &unknown_kid_header,
            &TestClaims {
                sub: "neon-user-123".to_owned(),
                iss: ISSUER.to_owned(),
                exp: now,
            },
            &signing_key,
        )
        .expect("test token signs");

        assert_eq!(
            verify_with_keys(&wrong_algorithm, &keys, ISSUER),
            Err(TokenError::Invalid)
        );
        assert_eq!(
            verify_with_keys(&unknown_kid, &keys, ISSUER),
            Err(TokenError::Invalid)
        );
    }

    #[test]
    fn only_allows_https_or_loopback_auth_endpoints() {
        assert!(NeonJwtVerifier::new(ISSUER, "https://auth.example.test/jwks").is_ok());
        assert!(
            NeonJwtVerifier::new("http://localhost:3000/auth", "http://127.0.0.1:3000/jwks")
                .is_ok()
        );
        assert!(NeonJwtVerifier::new("http://[::1]:3000/auth", "http://[::1]:3000/jwks").is_ok());
        assert!(
            NeonJwtVerifier::new("http://auth.example.test", "http://auth.example.test/jwks")
                .is_err()
        );
        assert!(
            NeonJwtVerifier::new(
                "https://user:password@auth.example.test",
                "https://auth.example.test/jwks"
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn fetches_and_reuses_a_bounded_jwks_cache() {
        let (keys, signing_key) = test_keys();
        let body = serde_json::to_vec(&keys).expect("JWKS serializes");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback listener binds");
        let address = listener.local_addr().expect("listener address exists");
        let issuer = format!("http://{address}");
        let token = test_token(
            &signing_key,
            &issuer,
            jsonwebtoken::get_current_timestamp() as usize + 60,
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("JWKS request arrives");
            let mut request = [0_u8; 1024];
            let _ = stream
                .read(&mut request)
                .await
                .expect("request headers are readable");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("response headers write");
            stream.write_all(&body).await.expect("JWKS body writes");
        });

        let verifier = NeonJwtVerifier::new(&format!("{issuer}/auth"), &format!("{issuer}/jwks"))
            .expect("loopback verifier configuration is valid");
        assert_eq!(
            verifier.verify(&token).await,
            Ok(super::AuthenticatedIdentity {
                issuer: issuer.clone(),
                subject: "neon-user-123".to_owned()
            })
        );
        assert_eq!(
            verifier.verify(&token).await,
            Ok(super::AuthenticatedIdentity {
                issuer: issuer.clone(),
                subject: "neon-user-123".to_owned()
            })
        );
        // An unknown key id must not trigger a fresh fetch inside the cooldown
        // window. The fixture listener is gone, so a refetch would surface as
        // Unavailable instead of a fast Invalid.
        let unknown = test_token_with_kid(
            &signing_key,
            &issuer,
            jsonwebtoken::get_current_timestamp() as usize + 60,
            "unknown-key",
        );
        assert_eq!(verifier.verify(&unknown).await, Err(TokenError::Invalid));
        server.await.expect("JWKS fixture finishes");
    }

    #[tokio::test]
    async fn refreshes_jwks_once_when_a_signing_key_rotates() {
        let (keys, signing_key) = test_keys();
        let old_body = serde_json::to_vec(&keys).expect("old JWKS serializes");
        let mut rotated: serde_json::Value =
            serde_json::to_value(&keys).expect("JWKS converts to JSON");
        let mut rotated_key = rotated["keys"][0].clone();
        rotated_key["kid"] = serde_json::Value::String("rotated-key".to_owned());
        rotated["keys"]
            .as_array_mut()
            .expect("JWKS has a key array")
            .push(rotated_key);
        let rotated_body = serde_json::to_vec(&rotated).expect("rotated JWKS serializes");

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback listener binds");
        let address = listener.local_addr().expect("listener address exists");
        let issuer = format!("http://{address}");
        let expires_at = jsonwebtoken::get_current_timestamp() as usize + 60;
        let old_token = test_token(&signing_key, &issuer, expires_at);
        let rotated_token = test_token_with_kid(&signing_key, &issuer, expires_at, "rotated-key");
        let server = tokio::spawn(async move {
            for body in [old_body, rotated_body] {
                let (mut stream, _) = listener.accept().await.expect("JWKS request arrives");
                let mut request = [0_u8; 1024];
                let _ = stream
                    .read(&mut request)
                    .await
                    .expect("request headers are readable");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("response headers write");
                stream.write_all(&body).await.expect("JWKS body writes");
            }
        });

        let mut verifier =
            NeonJwtVerifier::new(&format!("{issuer}/auth"), &format!("{issuer}/jwks"))
                .expect("loopback verifier configuration is valid");
        verifier.refresh_cooldown = std::time::Duration::ZERO;
        assert_eq!(
            verifier.verify(&old_token).await,
            Ok(super::AuthenticatedIdentity {
                issuer: issuer.clone(),
                subject: "neon-user-123".to_owned(),
            })
        );
        assert_eq!(
            verifier.verify(&rotated_token).await,
            Ok(super::AuthenticatedIdentity {
                issuer: issuer.clone(),
                subject: "neon-user-123".to_owned(),
            })
        );
        server.await.expect("JWKS rotation fixture finishes");
    }
}
