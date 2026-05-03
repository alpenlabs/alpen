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
