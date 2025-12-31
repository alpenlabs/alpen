//! Profiling and metrics HTTP endpoints for performance monitoring.
//!
//! Provides:
//! - `/debug/pprof/profile?seconds=30` - CPU profiling in pprof format
//! - `/metrics` - Prometheus metrics endpoint

use pprof::ProfilerGuardBuilder;
use pprof::protos::Message;
use prometheus::{Encoder, TextEncoder};
use strata_common::metrics;
use tokio::task;
use tracing::*;

/// Start the profiling HTTP server on the given host and port.
///
/// Provides two endpoints:
/// - GET /debug/pprof/profile?seconds=<duration> - CPU profiling (flamegraph-compatible)
/// - GET /metrics - Prometheus metrics
pub async fn start_profiling_server(host: &str, port: u16) -> anyhow::Result<()> {
    use std::net::SocketAddr;
    use tokio::net::TcpListener;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    let listener = TcpListener::bind(&addr).await?;

    info!(%host, %port, "started profiling/metrics server");

    loop {
        let (mut socket, _) = listener.accept().await?;

        tokio::spawn(async move {
            let mut buffer = [0u8; 1024];
            if let Ok(n) = socket.read(&mut buffer).await {
                let request = String::from_utf8_lossy(&buffer[..n]);

                // Parse the HTTP request line
                let lines: Vec<&str> = request.lines().collect();
                if lines.is_empty() {
                    return;
                }

                let first_line = lines[0];
                let parts: Vec<&str> = first_line.split_whitespace().collect();
                if parts.len() < 2 {
                    return;
                }

                let method = parts[0];
                let path = parts[1];

                if method != "GET" {
                    let response = "HTTP/1.1 405 Method Not Allowed\r\n\r\n";
                    let _ = socket.write_all(response.as_bytes()).await;
                    return;
                }

                // Handle /metrics endpoint
                if path == "/metrics" {
                    match handle_metrics().await {
                        Ok(body) => {
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            let _ = socket.write_all(response.as_bytes()).await;
                        }
                        Err(e) => {
                            error!("failed to generate metrics: {}", e);
                            let response = "HTTP/1.1 500 Internal Server Error\r\n\r\n";
                            let _ = socket.write_all(response.as_bytes()).await;
                        }
                    }
                    return;
                }

                // Handle /debug/pprof/profile endpoint
                if path.starts_with("/debug/pprof/profile") {
                    // Parse query parameter for duration (default 30 seconds)
                    let duration_secs = if let Some(query_start) = path.find('?') {
                        let query = &path[query_start + 1..];
                        query
                            .split('&')
                            .find_map(|param| {
                                if param.starts_with("seconds=") {
                                    param.strip_prefix("seconds=")?.parse::<u64>().ok()
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(30)
                    } else {
                        30
                    };

                    info!("starting CPU profiling for {} seconds", duration_secs);

                    match handle_pprof(duration_secs).await {
                        Ok(body) => {
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
                                body.len()
                            );
                            let _ = socket.write_all(response.as_bytes()).await;
                            let _ = socket.write_all(&body).await;

                            info!("CPU profiling completed, sent {} bytes", body.len());
                        }
                        Err(e) => {
                            error!("failed to generate pprof: {}", e);
                            let response = "HTTP/1.1 500 Internal Server Error\r\n\r\n";
                            let _ = socket.write_all(response.as_bytes()).await;
                        }
                    }
                    return;
                }

                // 404 for unknown paths
                let response = "HTTP/1.1 404 Not Found\r\n\r\n";
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
    }
}

/// Handle the /metrics endpoint - return Prometheus metrics
async fn handle_metrics() -> anyhow::Result<String> {
    let encoder = TextEncoder::new();
    let metric_families = metrics::REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer)?;
    Ok(String::from_utf8(buffer)?)
}

/// Handle the /debug/pprof/profile endpoint - return CPU profile in pprof format
async fn handle_pprof(duration_secs: u64) -> anyhow::Result<Vec<u8>> {
    // Run profiling in a blocking task since it's CPU-intensive
    let result = task::spawn_blocking(move || {
        // Start profiling
        let guard = ProfilerGuardBuilder::default()
            .frequency(100) // 100 Hz sampling rate
            .blocklist(&["libc", "libgcc", "pthread", "vdso"])
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build profiler: {}", e))?;

        // Sleep for the requested duration
        std::thread::sleep(std::time::Duration::from_secs(duration_secs));

        // Stop profiling and generate report
        let report = guard
            .report()
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build report: {}", e))?;

        // Serialize to pprof protobuf format
        let mut buffer = Vec::new();
        report
            .pprof()
            .map_err(|e| anyhow::anyhow!("failed to generate pprof: {}", e))?
            .write_to_vec(&mut buffer)
            .map_err(|e| anyhow::anyhow!("failed to serialize pprof: {}", e))?;

        Ok::<Vec<u8>, anyhow::Error>(buffer)
    })
    .await?;

    result
}
