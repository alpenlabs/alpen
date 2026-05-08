//! Authentication middleware for the admin RPC listener.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use http::{
    HeaderValue, Request, StatusCode,
    header::{AUTHORIZATION, CONTENT_TYPE},
};
use jsonrpsee::server::{HttpBody, HttpResponse};
use tower::{Layer, Service};

/// Tower layer that requires a bearer token on every admin RPC request.
#[derive(Clone, Debug)]
pub(crate) struct AdminAuthLayer {
    expected_authorization: HeaderValue,
}

impl AdminAuthLayer {
    /// Creates a new admin auth layer.
    pub(crate) fn new(token: String) -> Self {
        let expected_authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .expect("bearer token should be representable as a header value");
        Self {
            expected_authorization,
        }
    }
}

impl<S> Layer<S> for AdminAuthLayer {
    type Service = AdminAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AdminAuthService {
            inner,
            expected_authorization: self.expected_authorization.clone(),
        }
    }
}

/// Service produced by [`AdminAuthLayer`].
#[derive(Clone, Debug)]
pub(crate) struct AdminAuthService<S> {
    inner: S,
    expected_authorization: HeaderValue,
}

impl<S, B> Service<Request<B>> for AdminAuthService<S>
where
    S: Service<Request<B>, Response = HttpResponse> + Send + Clone + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        if !authorization_matches(
            request.headers().get(AUTHORIZATION),
            &self.expected_authorization,
        ) {
            return Box::pin(async { Ok(unauthorized_response()) });
        }

        Box::pin(self.inner.call(request))
    }
}

fn authorization_matches(presented: Option<&HeaderValue>, expected: &HeaderValue) -> bool {
    let Some(presented) = presented else {
        return false;
    };

    constant_time_eq(presented.as_bytes(), expected.as_bytes())
}

fn unauthorized_response() -> HttpResponse {
    HttpResponse::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(CONTENT_TYPE, HeaderValue::from_static("text/plain"))
        .body(HttpBody::from("Unauthorized\n"))
        .expect("static response should be valid")
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(left_byte ^ right_byte);
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_equal_slices() {
        assert!(constant_time_eq(b"Bearer token", b"Bearer token"));
        assert!(!constant_time_eq(b"Bearer token", b"Bearer wrong"));
        assert!(!constant_time_eq(b"Bearer token", b"Bearer token2"));
    }

    #[test]
    fn authorization_matches_exact_bearer_token() {
        let expected = HeaderValue::from_static("Bearer token");
        assert!(authorization_matches(Some(&expected), &expected));
        assert!(!authorization_matches(
            Some(&HeaderValue::from_static("token")),
            &expected,
        ));
        assert!(!authorization_matches(None, &expected));
    }
}
