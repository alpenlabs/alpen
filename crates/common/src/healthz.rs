//! HTTP health check helpers.

use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll},
};

use futures::future::{ready, Either, Ready};
use http::{
    header::{CACHE_CONTROL, CONTENT_TYPE},
    Method, Request, StatusCode,
};
use jsonrpsee::{
    server::{HttpBody, HttpResponse, ServerBuilder, ServerHandle},
    RpcModule,
};
use tower::{Layer, Service, ServiceBuilder};

/// Path exposed by all application health checks.
pub const HEALTHZ_PATH: &str = "/healthz";

/// Shared readiness state for health check responses.
#[derive(Clone, Debug, Default)]
pub struct HealthCheckState {
    ready: Arc<AtomicBool>,
}

impl HealthCheckState {
    /// Creates a new health check state marked not ready.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new health check state marked ready.
    pub fn ready() -> Self {
        let state = Self::new();
        state.mark_ready();
        state
    }

    /// Marks the service ready.
    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    /// Marks the service not ready.
    pub fn mark_not_ready(&self) {
        self.ready.store(false, Ordering::Release);
    }

    /// Returns whether the service is ready.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
}

/// Tower layer that serves `GET /healthz` before [`jsonrpsee`] dispatch.
#[derive(Clone, Debug)]
pub struct HealthCheckLayer {
    state: HealthCheckState,
}

impl HealthCheckLayer {
    /// Creates a new health check layer.
    pub fn new(state: HealthCheckState) -> Self {
        Self { state }
    }
}

impl<S> Layer<S> for HealthCheckLayer {
    type Service = HealthCheckService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        HealthCheckService {
            inner,
            state: self.state.clone(),
        }
    }
}

/// Service produced by [`HealthCheckLayer`].
#[derive(Clone, Debug)]
pub struct HealthCheckService<S> {
    inner: S,
    state: HealthCheckState,
}

impl<S, B> Service<Request<B>> for HealthCheckService<S>
where
    S: Service<Request<B>, Response = HttpResponse> + Send + Clone + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Either<Ready<Result<Self::Response, Self::Error>>, S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        if request.uri().path() == HEALTHZ_PATH {
            let response = match *request.method() {
                Method::GET => health_response(self.state.is_ready()),
                _ => method_not_allowed_response(),
            };
            return Either::Left(ready(Ok(response)));
        }

        Either::Right(self.inner.call(request))
    }
}

/// Starts a standalone HTTP server that only exposes `GET /healthz`.
///
/// The standalone server intentionally uses [`jsonrpsee`] so it shares the same
/// HTTP service stack as the RPC listeners. The empty module lets the health
/// middleware answer `/healthz` while any other request is handled by
/// [`jsonrpsee`]'s normal HTTP fallback.
pub async fn start_health_check_server(
    addr: String,
    state: HealthCheckState,
) -> io::Result<ServerHandle> {
    let module = RpcModule::new(());
    let middleware = ServiceBuilder::new().layer(HealthCheckLayer::new(state));
    let server = ServerBuilder::new()
        .set_http_middleware(middleware)
        .build(addr)
        .await?;

    Ok(server.start(module))
}

fn health_response(ready: bool) -> HttpResponse {
    let (status, body) = if ready {
        (StatusCode::OK, "ok\n")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready\n")
    };

    response(status, body)
}

fn method_not_allowed_response() -> HttpResponse {
    response(StatusCode::METHOD_NOT_ALLOWED, "method not allowed\n")
}

fn response(status: StatusCode, body: &'static str) -> HttpResponse {
    HttpResponse::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(CACHE_CONTROL, "no-store")
        .body(HttpBody::from(body))
        .expect("static health check response should be valid")
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, net::TcpListener, time::Duration};

    use http::Uri;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        time::timeout,
    };
    use tower::service_fn;

    use super::*;

    #[tokio::test]
    async fn healthz_returns_ok_when_ready() {
        let state = HealthCheckState::ready();
        let mut service = HealthCheckLayer::new(state).layer(service_fn(inner_response));
        let request = Request::builder()
            .method(Method::GET)
            .uri(HEALTHZ_PATH)
            .body(HttpBody::from(""))
            .unwrap();

        let response = service.call(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn healthz_returns_unavailable_before_ready() {
        let state = HealthCheckState::new();
        let mut service = HealthCheckLayer::new(state).layer(service_fn(inner_response));
        let request = Request::builder()
            .method(Method::GET)
            .uri(HEALTHZ_PATH)
            .body(HttpBody::from(""))
            .unwrap();

        let response = service.call(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn healthz_rejects_non_get_requests() {
        let state = HealthCheckState::ready();
        let mut service = HealthCheckLayer::new(state).layer(service_fn(inner_response));
        let request = Request::builder()
            .method(Method::POST)
            .uri(HEALTHZ_PATH)
            .body(HttpBody::from(""))
            .unwrap();

        let response = service.call(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn non_healthz_requests_fall_through() {
        let state = HealthCheckState::ready();
        let mut service = HealthCheckLayer::new(state).layer(service_fn(inner_response));
        let request = Request::builder()
            .method(Method::POST)
            .uri(Uri::from_static("/"))
            .body(HttpBody::from(""))
            .unwrap();

        let response = service.call(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    }

    #[tokio::test]
    async fn standalone_server_serves_healthz() {
        let addr = available_addr();
        let handle = start_health_check_server(addr.clone(), HealthCheckState::ready())
            .await
            .unwrap();

        let response = raw_http_request(
            &addr,
            "GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;

        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response:?}");
        assert!(response.ends_with("ok\n"), "{response:?}");

        handle.stop().unwrap();
    }

    async fn inner_response(_: Request<HttpBody>) -> Result<HttpResponse, Infallible> {
        Ok(response(StatusCode::IM_A_TEAPOT, "inner\n"))
    }

    fn available_addr() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().to_string()
    }

    async fn raw_http_request(addr: &str, request: &str) -> String {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();

        let mut response = vec![0; 1024];
        let len = timeout(Duration::from_secs(1), stream.read(&mut response))
            .await
            .unwrap()
            .unwrap();
        String::from_utf8(response[..len].to_vec()).unwrap()
    }
}
