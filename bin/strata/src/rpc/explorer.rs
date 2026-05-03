//! Embedded OL explorer HTML served at `GET /explorer`.
//!
//! `jsonrpsee` only handles JSON-RPC; to serve a static HTML route on the
//! same port we compose a [`tower::Layer`] that intercepts `GET /explorer`
//! and short-circuits with the embedded HTML response. Everything else
//! falls through to the inner jsonrpsee service unchanged.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use http::{Method, StatusCode};
use jsonrpsee::server::{HttpBody, HttpRequest, HttpResponse};
use tower::{Layer, Service};

/// Path served by the embedded explorer HTML.
const EXPLORER_PATH: &str = "/explorer";

/// HTML document for the OL explorer, embedded at compile time.
const EXPLORER_HTML: &str = include_str!("../../static/ol-explorer.html");

/// Tower layer that serves [`EXPLORER_HTML`] at `GET /explorer` and
/// delegates everything else to the inner service.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ExplorerLayer;

impl<S> Layer<S> for ExplorerLayer {
    type Service = ExplorerService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ExplorerService { inner }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ExplorerService<S> {
    inner: S,
}

impl<S> Service<HttpRequest<HttpBody>> for ExplorerService<S>
where
    S: Service<HttpRequest<HttpBody>, Response = HttpResponse<HttpBody>> + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = HttpResponse<HttpBody>;
    type Error = S::Error;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: HttpRequest<HttpBody>) -> Self::Future {
        if req.method() == Method::GET && req.uri().path() == EXPLORER_PATH {
            let response = HttpResponse::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/html; charset=utf-8")
                .header("Cache-Control", "no-cache")
                .body(HttpBody::from(EXPLORER_HTML.to_string()))
                .expect("static explorer response always builds");
            return Box::pin(async move { Ok(response) });
        }
        let fut = self.inner.call(req);
        Box::pin(fut)
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use http::Request;

    use super::*;

    /// Mock inner service that returns 418 with a marker header so we can
    /// distinguish delegated responses from short-circuited ones.
    #[derive(Clone, Copy)]
    struct MockInner;

    impl Service<HttpRequest<HttpBody>> for MockInner {
        type Response = HttpResponse<HttpBody>;
        type Error = Infallible;
        type Future =
            Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _req: HttpRequest<HttpBody>) -> Self::Future {
            Box::pin(async {
                Ok(HttpResponse::builder()
                    .status(StatusCode::IM_A_TEAPOT)
                    .header("x-from-inner", "yes")
                    .body(HttpBody::empty())
                    .expect("mock response builds"))
            })
        }
    }

    fn req(method: Method, path: &str) -> HttpRequest<HttpBody> {
        Request::builder()
            .method(method)
            .uri(path)
            .body(HttpBody::empty())
            .expect("test request builds")
    }

    #[tokio::test]
    async fn get_explorer_returns_html_short_circuit() {
        let mut svc = ExplorerLayer.layer(MockInner);
        let res = svc
            .call(req(Method::GET, "/explorer"))
            .await
            .expect("call");
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get("content-type").expect("content-type"),
            "text/html; charset=utf-8"
        );
        // Inner marker absent => we short-circuited rather than delegating.
        assert!(res.headers().get("x-from-inner").is_none());
    }

    #[tokio::test]
    async fn get_other_path_delegates_to_inner() {
        let mut svc = ExplorerLayer.layer(MockInner);
        let res = svc
            .call(req(Method::GET, "/something-else"))
            .await
            .expect("call");
        assert_eq!(res.status(), StatusCode::IM_A_TEAPOT);
        assert_eq!(
            res.headers().get("x-from-inner").expect("inner marker"),
            "yes"
        );
    }

    #[tokio::test]
    async fn post_explorer_path_delegates_to_inner() {
        // Only GET /explorer is intercepted; jsonrpsee handles POST for
        // JSON-RPC on every path including /explorer.
        let mut svc = ExplorerLayer.layer(MockInner);
        let res = svc
            .call(req(Method::POST, "/explorer"))
            .await
            .expect("call");
        assert_eq!(res.status(), StatusCode::IM_A_TEAPOT);
    }
}
