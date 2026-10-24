//! Request extractors for JSON bodies, client identity and authentication.

use std::net::{IpAddr, SocketAddr};

use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, FromRequest, FromRequestParts, Path, Query, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use axum_extra::extract::cookie::CookieJar;
use serde::de::DeserializeOwned;

use super::error::ApiError;
use crate::app::{AppState, Security};
use crate::auth::{AuthenticatedSession, service};

pub const CSRF_HEADER: &str = "x-csrf-token";
pub const SECURE_SESSION_COOKIE: &str = "__Host-merton_session";
pub const DEV_SESSION_COOKIE: &str = "merton_session";

pub fn session_cookie_name(security: &Security) -> &'static str {
    if security.cookie_secure {
        SECURE_SESSION_COOKIE
    } else {
        DEV_SESSION_COOKIE
    }
}

/// `Json<T>` whose rejections are Problem Details.
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(Self(value)),
            Err(JsonRejection::MissingJsonContentType(err)) => {
                Err(ApiError::UnsupportedMediaType(err.body_text()))
            }
            Err(err) => Err(ApiError::BadRequest(err.body_text())),
        }
    }
}

/// `Query<T>` whose rejections are Problem Details (400).
pub struct ApiQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|err| ApiError::BadRequest(err.body_text()))
    }
}

/// A resource ID from the path. A malformed ID cannot name any resource, so
/// it is reported as `404`, the same as a missing or inaccessible one.
pub struct ResourceId(pub uuid::Uuid);

impl<S: Send + Sync> FromRequestParts<S> for ResourceId {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        Path::<uuid::Uuid>::from_request_parts(parts, state)
            .await
            .map(|Path(id)| Self(id))
            .map_err(|_| ApiError::NotFound)
    }
}

/// The client address, taking `X-Forwarded-For` into account only when the
/// direct peer is a configured trusted proxy.
pub struct ClientIp(pub IpAddr);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let ConnectInfo(peer) = ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
            .await
            .map_err(|_| {
                tracing::error!("connection info missing; serve with connect info enabled");
                ApiError::Internal
            })?;
        Ok(Self(resolve_client_ip(
            peer.ip(),
            &parts.headers,
            &state.security.trusted_proxies,
        )))
    }
}

/// Walks `X-Forwarded-For` from the right, skipping trusted proxies, so a
/// client cannot spoof its address by prepending entries.
fn resolve_client_ip(peer: IpAddr, headers: &HeaderMap, trusted: &[IpAddr]) -> IpAddr {
    if !trusted.contains(&peer) {
        return peer;
    }
    let forwarded: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .collect();
    for hop in forwarded.iter().rev() {
        match hop.parse::<IpAddr>() {
            Ok(ip) if trusted.contains(&ip) => continue,
            Ok(ip) => return ip,
            Err(_) => break,
        }
    }
    peer
}

/// Rejects cross-site browser requests to state-changing endpoints.
///
/// A present `Origin` must exactly match a trusted origin. Without `Origin`,
/// `Sec-Fetch-Site` (if sent) must be `same-origin` or `none`. Requests with
/// neither header come from non-browser clients, which cannot be used for CSRF.
pub struct TrustedOrigin;

impl FromRequestParts<AppState> for TrustedOrigin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        check_origin(&parts.headers, &state.security.trusted_origins)?;
        Ok(Self)
    }
}

fn check_origin(headers: &HeaderMap, trusted: &[String]) -> Result<(), ApiError> {
    let allowed = match headers.get(header::ORIGIN) {
        Some(origin) => origin
            .to_str()
            .is_ok_and(|origin| trusted.iter().any(|t| t.eq_ignore_ascii_case(origin))),
        None => matches!(
            headers.get("sec-fetch-site").map(|v| v.as_bytes()),
            None | Some(b"same-origin" | b"none")
        ),
    };
    if allowed {
        Ok(())
    } else {
        Err(ApiError::Forbidden {
            code: "untrusted_origin",
            detail: "The request origin is not trusted.",
        })
    }
}

/// The current session, resolved from the session cookie. The user identity
/// comes only from the server-side session record.
pub struct CurrentSession(pub AuthenticatedSession);

impl FromRequestParts<AppState> for CurrentSession {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(session_cookie_name(&state.security))
            .ok_or(ApiError::Unauthenticated)?;
        let session = service::authenticate(&state.db, token.value()).await?;
        Ok(Self(session))
    }
}

/// A current session on a state-changing request that passed the origin
/// check and carries a valid `X-CSRF-Token` for this session.
pub struct CsrfProtectedSession(pub AuthenticatedSession);

impl FromRequestParts<AppState> for CsrfProtectedSession {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        check_origin(&parts.headers, &state.security.trusted_origins)?;
        let CurrentSession(session) = CurrentSession::from_request_parts(parts, state).await?;

        let token = parts
            .headers
            .get(CSRF_HEADER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !state.security.csrf.verify(session.session_id, token) {
            return Err(ApiError::Forbidden {
                code: "csrf_invalid",
                detail: "A valid X-CSRF-Token header is required.",
            });
        }
        Ok(Self(session))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn ignores_forwarded_for_from_untrusted_peers() {
        let peer: IpAddr = "203.0.113.5".parse().unwrap();
        let spoofed = headers(&[("x-forwarded-for", "1.2.3.4")]);

        assert_eq!(resolve_client_ip(peer, &spoofed, &[]), peer);
    }

    #[test]
    fn uses_rightmost_untrusted_hop_behind_trusted_proxy() {
        let proxy: IpAddr = "10.0.0.2".parse().unwrap();
        let inner_proxy: IpAddr = "10.0.0.3".parse().unwrap();
        let forwarded = headers(&[("x-forwarded-for", "6.6.6.6, 198.51.100.7, 10.0.0.3")]);

        assert_eq!(
            resolve_client_ip(proxy, &forwarded, &[proxy, inner_proxy]),
            "198.51.100.7".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn falls_back_to_peer_on_garbage_forwarded_for() {
        let proxy: IpAddr = "10.0.0.2".parse().unwrap();
        let garbage = headers(&[("x-forwarded-for", "not-an-ip")]);

        assert_eq!(resolve_client_ip(proxy, &garbage, &[proxy]), proxy);
    }

    #[test]
    fn origin_check() {
        let trusted = ["https://merton.example.com".to_owned()];

        assert!(
            check_origin(
                &headers(&[("origin", "https://merton.example.com")]),
                &trusted
            )
            .is_ok()
        );
        assert!(check_origin(&headers(&[("origin", "https://evil.example")]), &trusted).is_err());
        assert!(check_origin(&headers(&[("origin", "null")]), &trusted).is_err());
        assert!(check_origin(&headers(&[("sec-fetch-site", "cross-site")]), &trusted).is_err());
        assert!(check_origin(&headers(&[("sec-fetch-site", "same-origin")]), &trusted).is_ok());
        assert!(check_origin(&HeaderMap::new(), &trusted).is_ok());
    }
}
