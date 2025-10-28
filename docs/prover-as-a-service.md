# Prover-as-a-Service (PaaS) Design Document

**Version:** 1.0.0
**Status:** Design Phase
**Branch:** `PaaS`
**Authors:** Development Team
**Last Updated:** October 28, 2025

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Motivation](#motivation)
3. [Architecture Overview](#architecture-overview)
4. [Core Components](#core-components)
5. [API Design](#api-design)
6. [Proof Lifecycle](#proof-lifecycle)
7. [Error Handling & Retry Strategy](#error-handling--retry-strategy)
8. [Storage Layer](#storage-layer)
9. [Security & Authentication](#security--authentication)
10. [Deployment Patterns](#deployment-patterns)
11. [Monitoring & Observability](#monitoring--observability)
12. [Implementation Roadmap](#implementation-roadmap)
13. [Integration Examples](#integration-examples)

---

## Executive Summary

**Prover-as-a-Service (PaaS)** is a standalone microservice that provides zero-knowledge proof generation capabilities for Strata's guest programs. It exposes a RESTful/gRPC API for clients to submit proof requests, track their status, and retrieve completed proofs. The service handles all aspects of proof generation including task management, retry logic, error handling, and persistent storage.

### Key Features

- **Single Program Focus**: Optimized for proving a single guest code program with multiple proof contexts
- **Robust Retry Mechanism**: Exponential backoff with configurable limits
- **Production-Ready Error Handling**: Comprehensive error codes and structured responses
- **Persistent Storage**: Durable proof storage with efficient retrieval
- **Horizontal Scalability**: Stateless API layer with shared storage backend
- **Full Observability**: Metrics, logging, and tracing support
- **Flexible Deployment**: Docker, Kubernetes, or standalone binary

### Use Cases

1. **External Proof Generation**: Third-party services needing proof generation without running full nodes
2. **Load Distribution**: Distribute proof workload across multiple prover instances
3. **Proof Marketplace**: Foundation for proof generation marketplace
4. **Development/Testing**: Isolated proof generation for testing and development

---

## Motivation

### Current Limitations

The existing `prover-client` binary is designed as:
- A standalone daemon tightly coupled with Strata infrastructure
- Requires full node context (Bitcoin RPC, Sequencer RPC, Reth RPC)
- Not easily consumable as a service by external clients
- Lacks clear API boundaries for programmatic access

### Goals

1. **Decoupling**: Separate proof generation from node infrastructure
2. **API-First Design**: Clean, well-documented API for external consumption
3. **Operational Excellence**: Production-ready with comprehensive error handling
4. **Scalability**: Support horizontal scaling for high-throughput scenarios
5. **Developer Experience**: Easy to integrate, test, and deploy

### Non-Goals

1. **Multi-Program Support**: This version focuses on single program proving (can be extended later)
2. **Proof Verification**: Verification is handled by clients/nodes (out of scope)
3. **Payment/Billing**: No built-in payment system (can be added as extension)

---

## Architecture Overview

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         Clients                              │
│  (Sequencer, Full Nodes, External Services, CLIs)          │
└────────────────────┬────────────────────────────────────────┘
                     │
                     │ HTTP/gRPC API
                     │
┌────────────────────▼────────────────────────────────────────┐
│                    API Gateway Layer                         │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐     │
│  │ REST Handler │  │ gRPC Handler │  │ Auth Middleware│    │
│  └──────────────┘  └──────────────┘  └──────────────┘     │
└────────────────────┬────────────────────────────────────────┘
                     │
                     │ Internal API
                     │
┌────────────────────▼────────────────────────────────────────┐
│                  Service Core Layer                          │
│  ┌──────────────────────────────────────────────────────┐  │
│  │            ProverService (Service Framework)          │  │
│  │  - Task Management                                    │  │
│  │  - Worker Pool Coordination                           │  │
│  │  - Retry Logic                                        │  │
│  │  - Status Tracking                                    │  │
│  └──────────────────────────────────────────────────────┘  │
└────────────────────┬────────────────────────────────────────┘
                     │
         ┌───────────┴───────────┐
         │                       │
         ▼                       ▼
┌─────────────────┐    ┌─────────────────┐
│  Storage Layer  │    │  ZkVM Backend   │
│  (PostgreSQL/   │    │  (SP1/Native)   │
│   RocksDB)      │    │                 │
└─────────────────┘    └─────────────────┘
```

### Component Layers

#### 1. API Gateway Layer
- **REST Handler**: RESTful HTTP endpoints (primary interface)
- **gRPC Handler**: gRPC endpoints (optional, for high-performance scenarios)
- **Auth Middleware**: API key authentication, rate limiting
- **Request Validation**: Input validation and sanitization

#### 2. Service Core Layer
- **ProverService**: Core service using `crates/service` framework
- **Task Manager**: Proof task lifecycle management
- **Worker Pool**: Manages proving backend workers (SP1/Native)
- **Retry Engine**: Exponential backoff and retry orchestration
- **Status Tracker**: Real-time task status monitoring

#### 3. Storage Layer
- **Proof Database**: Persistent storage for completed proofs
- **Task Database**: Task metadata and state tracking
- **Cache Layer** (optional): Redis for hot proof retrieval

#### 4. ZkVM Backend
- **SP1 Prover**: Primary proving backend
- **Native Prover**: Fallback/development backend

---

## Core Components

### 1. ProverService

Central service implementing the `AsyncService` trait from `crates/service`.

```rust
pub struct ProverService;

impl Service for ProverService {
    type State = ProverServiceState;
    type Msg = ProverCommand;
    type Status = ProverServiceStatus;

    fn get_status(s: &Self::State) -> Self::Status {
        s.generate_status()
    }
}

impl AsyncService for ProverService {
    async fn on_launch(state: &mut Self::State) -> anyhow::Result<()>;
    async fn process_input(state: &mut Self::State, cmd: &Self::Msg) -> anyhow::Result<Response>;
    async fn before_shutdown(state: &mut Self::State, err: Option<&anyhow::Error>) -> anyhow::Result<()>;
}
```

### 2. ProverServiceState

```rust
pub struct ProverServiceState {
    // Configuration
    config: PaaSConfig,
    guest_program: GuestProgramConfig,

    // Core components
    task_manager: TaskManager,
    worker_pool: WorkerPool,
    storage: Arc<dyn ProofStorage>,

    // Metrics
    metrics: ProverMetrics,

    // Background tasks
    worker_handles: Vec<JoinHandle<()>>,
    cleanup_handle: Option<JoinHandle<()>>,
}
```

### 3. TaskManager

Manages proof task lifecycle with state machine:

```
┌─────────┐
│ Pending │
└────┬────┘
     │
     ▼
┌─────────────┐      ┌────────────────┐
│ Queued      │─────▶│ Proving        │
└─────────────┘      └───┬────────────┘
                         │
                ┌────────┼────────┐
                │        │        │
                ▼        ▼        ▼
         ┌──────────┐ ┌──────┐ ┌────────────┐
         │Completed │ │Failed│ │TransientErr│
         └──────────┘ └──────┘ └──────┬─────┘
                                       │
                                       │ retry
                                       ▼
                                  ┌─────────┐
                                  │ Queued  │
                                  └─────────┘
```

### 4. WorkerPool

Manages proving backend workers with:
- Configurable worker count per backend (SP1/Native)
- Load balancing across workers
- Health monitoring and auto-recovery
- Graceful shutdown

### 5. Storage Abstraction

```rust
#[async_trait]
pub trait ProofStorage: Send + Sync {
    async fn store_proof(&self, task_id: TaskId, proof: ProofData) -> Result<()>;
    async fn get_proof(&self, task_id: TaskId) -> Result<Option<ProofData>>;
    async fn update_task_status(&self, task_id: TaskId, status: TaskStatus) -> Result<()>;
    async fn get_task(&self, task_id: TaskId) -> Result<Option<Task>>;
    async fn list_tasks(&self, filter: TaskFilter, pagination: Pagination) -> Result<Vec<Task>>;
    async fn delete_task(&self, task_id: TaskId) -> Result<()>;
}
```

Implementations:
- **PostgresStorage**: Production-ready with ACID guarantees
- **RocksDBStorage**: Embedded option for single-instance deployments
- **MemoryStorage**: Testing and development

---

## API Design

### RESTful API

Base URL: `https://prover.example.com/api/v1`

#### Endpoints

##### 1. Submit Proof Request

**POST** `/proofs`

Request:
```json
{
  "context": {
    "type": "Checkpoint",
    "checkpoint_index": 42
  },
  "priority": "normal",
  "backend": "sp1",
  "callback_url": "https://client.example.com/webhook/proof-complete",
  "metadata": {
    "client_id": "sequencer-1",
    "request_id": "req-123"
  }
}
```

Response (201 Created):
```json
{
  "task_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "queued",
  "created_at": "2025-10-28T10:30:00Z",
  "estimated_completion": "2025-10-28T10:35:00Z",
  "proof_url": "/api/v1/proofs/550e8400-e29b-41d4-a716-446655440000"
}
```

##### 2. Get Proof Status

**GET** `/proofs/{task_id}`

Response (200 OK):
```json
{
  "task_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "proving",
  "progress": 0.65,
  "created_at": "2025-10-28T10:30:00Z",
  "started_at": "2025-10-28T10:30:15Z",
  "context": {
    "type": "Checkpoint",
    "checkpoint_index": 42
  },
  "backend": "sp1",
  "metadata": {
    "client_id": "sequencer-1",
    "request_id": "req-123"
  }
}
```

##### 3. Get Proof Result

**GET** `/proofs/{task_id}/result`

Response (200 OK) when completed:
```json
{
  "task_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "completed",
  "proof": {
    "receipt": "base64_encoded_proof_receipt...",
    "public_values": "base64_encoded_public_values...",
    "verification_key": "base64_encoded_vk..."
  },
  "completed_at": "2025-10-28T10:34:22Z",
  "duration_ms": 4207,
  "backend": "sp1"
}
```

Response (404 Not Found) when not completed:
```json
{
  "error": {
    "code": "PROOF_NOT_READY",
    "message": "Proof is still being generated",
    "status": "proving",
    "retry_after": 30
  }
}
```

##### 4. Download Proof Binary

**GET** `/proofs/{task_id}/download`

Response: Binary proof data with appropriate `Content-Type` header

##### 5. List Proofs

**GET** `/proofs?status=completed&limit=10&offset=0`

Query Parameters:
- `status`: Filter by status (pending, queued, proving, completed, failed)
- `backend`: Filter by backend (sp1, native)
- `from_date`: Start date filter (ISO 8601)
- `to_date`: End date filter (ISO 8601)
- `limit`: Results per page (default: 20, max: 100)
- `offset`: Pagination offset

Response (200 OK):
```json
{
  "proofs": [
    {
      "task_id": "550e8400-e29b-41d4-a716-446655440000",
      "status": "completed",
      "created_at": "2025-10-28T10:30:00Z",
      "completed_at": "2025-10-28T10:34:22Z",
      "backend": "sp1"
    }
  ],
  "pagination": {
    "total": 142,
    "limit": 10,
    "offset": 0,
    "has_more": true
  }
}
```

##### 6. Cancel Proof Request

**DELETE** `/proofs/{task_id}`

Response (200 OK):
```json
{
  "task_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "cancelled",
  "cancelled_at": "2025-10-28T10:32:00Z"
}
```

##### 7. Get Service Health

**GET** `/health`

Response (200 OK):
```json
{
  "status": "healthy",
  "version": "1.0.0",
  "uptime_seconds": 86400,
  "worker_pools": {
    "sp1": {
      "total_workers": 20,
      "busy_workers": 15,
      "available_workers": 5
    },
    "native": {
      "total_workers": 5,
      "busy_workers": 2,
      "available_workers": 3
    }
  },
  "storage": {
    "status": "connected",
    "type": "postgresql"
  },
  "metrics": {
    "total_proofs": 1523,
    "completed_proofs": 1487,
    "failed_proofs": 36,
    "average_duration_ms": 4523
  }
}
```

##### 8. Get Service Metrics

**GET** `/metrics`

Response: Prometheus-compatible metrics

```
# HELP paas_proofs_total Total number of proof requests
# TYPE paas_proofs_total counter
paas_proofs_total{status="completed",backend="sp1"} 1487
paas_proofs_total{status="failed",backend="sp1"} 36

# HELP paas_proof_duration_seconds Proof generation duration
# TYPE paas_proof_duration_seconds histogram
paas_proof_duration_seconds_bucket{backend="sp1",le="1"} 0
paas_proof_duration_seconds_bucket{backend="sp1",le="5"} 1234
paas_proof_duration_seconds_bucket{backend="sp1",le="10"} 1487
```

---

## Proof Lifecycle

### State Transitions

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Task created, waiting to be queued
    Pending,

    /// Task in worker queue, waiting for available worker
    Queued,

    /// Proof generation in progress
    Proving {
        worker_id: WorkerId,
        started_at: DateTime<Utc>,
        progress: f32,
    },

    /// Proof successfully generated
    Completed {
        completed_at: DateTime<Utc>,
        duration_ms: u64,
    },

    /// Permanent failure (no more retries)
    Failed {
        failed_at: DateTime<Utc>,
        error: ErrorInfo,
        retry_count: u32,
    },

    /// Transient failure, will be retried
    TransientFailure {
        failed_at: DateTime<Utc>,
        error: ErrorInfo,
        retry_count: u32,
        next_retry_at: DateTime<Utc>,
    },

    /// Task cancelled by user
    Cancelled {
        cancelled_at: DateTime<Utc>,
    },
}
```

### Lifecycle Flow

1. **Submission**: Client submits proof request via API
2. **Validation**: Request validated against schema
3. **Pending**: Task created in database with `Pending` status
4. **Queueing**: Task moved to `Queued` when ready for processing
5. **Worker Assignment**: Available worker picks up task
6. **Proving**: Worker generates proof, updates progress
7. **Completion**:
   - **Success**: Proof stored, status → `Completed`
   - **Transient Error**: Status → `TransientFailure`, schedule retry
   - **Permanent Error**: Status → `Failed`
8. **Notification**: Webhook called if `callback_url` provided
9. **Retrieval**: Client retrieves proof via API
10. **Cleanup**: Old proofs archived/deleted based on retention policy

---

## Error Handling & Retry Strategy

### Error Categories

#### 1. Client Errors (4xx)

**Code**: `CLIENT_ERROR_*`
**HTTP Status**: 400-499
**Action**: No retry, client must fix request

Examples:
- `INVALID_REQUEST`: Malformed JSON or missing required fields
- `INVALID_CONTEXT`: Invalid proof context parameters
- `TASK_NOT_FOUND`: Task ID doesn't exist
- `UNAUTHORIZED`: Invalid or missing API key
- `RATE_LIMIT_EXCEEDED`: Client exceeded rate limit

#### 2. Server Errors (5xx)

**Code**: `SERVER_ERROR_*`
**HTTP Status**: 500-599
**Action**: Automatic retry with backoff

Examples:
- `INTERNAL_ERROR`: Unexpected server error
- `STORAGE_ERROR`: Database connection/operation failed
- `WORKER_UNAVAILABLE`: No workers available (temporary)

#### 3. Proving Errors

**Code**: `PROVING_ERROR_*`
**Action**: Depends on error type

##### Transient (Retryable)
- `PROVING_NETWORK_ERROR`: Network error during proof generation (SP1 network issues)
- `PROVING_TIMEOUT`: Proof generation timeout (may succeed on retry)
- `PROVING_WORKER_CRASH`: Worker crashed unexpectedly

##### Permanent (Non-retryable)
- `PROVING_INVALID_INPUT`: Invalid input to proving system
- `PROVING_OUT_OF_MEMORY`: Insufficient memory (proof too large)
- `PROVING_PROGRAM_ERROR`: Guest program panic/error

### Error Response Format

```json
{
  "error": {
    "code": "PROVING_NETWORK_ERROR",
    "message": "Network error during proof generation",
    "details": {
      "zkvm": "sp1",
      "underlying_error": "Failed to connect to SP1 network",
      "retry_count": 2
    },
    "request_id": "req-550e8400-e29b-41d4-a716-446655440000",
    "timestamp": "2025-10-28T10:32:15Z",
    "retry_info": {
      "retryable": true,
      "retry_after": 60,
      "max_retries": 15
    }
  }
}
```

### Retry Strategy

#### Exponential Backoff

```rust
pub struct RetryPolicy {
    /// Maximum number of retry attempts
    pub max_retries: u32,

    /// Base delay in seconds (first retry)
    pub base_delay: u64,

    /// Multiplier for each subsequent retry
    pub multiplier: f64,

    /// Maximum delay cap in seconds
    pub max_delay: u64,

    /// Add random jitter to prevent thundering herd
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 15,
            base_delay: 5,      // 5 seconds
            multiplier: 1.5,
            max_delay: 3600,    // 1 hour
            jitter: true,
        }
    }
}
```

#### Retry Schedule Example

| Retry # | Base Delay | With Jitter Range |
|---------|------------|-------------------|
| 1       | 5s         | 4-6s             |
| 2       | 7.5s       | 6-9s             |
| 3       | 11s        | 9-13s            |
| 4       | 17s        | 14-20s           |
| 5       | 25s        | 20-30s           |
| 10      | 114s       | 91-137s          |
| 15      | 386s       | 309-463s         |

#### Circuit Breaker

For repeated failures, implement circuit breaker pattern:

```rust
pub struct CircuitBreaker {
    /// Number of failures before opening circuit
    failure_threshold: u32,

    /// Time to wait before attempting to close circuit
    timeout: Duration,

    /// Current state
    state: CircuitState,
}

pub enum CircuitState {
    Closed,      // Normal operation
    Open,        // Failing, reject requests immediately
    HalfOpen,    // Testing if service recovered
}
```

---

## Storage Layer

### Database Schema

#### Tasks Table

```sql
CREATE TABLE tasks (
    task_id UUID PRIMARY KEY,
    status VARCHAR(50) NOT NULL,
    context JSONB NOT NULL,
    priority VARCHAR(20) DEFAULT 'normal',
    backend VARCHAR(20) NOT NULL,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    queued_at TIMESTAMPTZ,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,

    -- Error tracking
    retry_count INT DEFAULT 0,
    last_error JSONB,
    next_retry_at TIMESTAMPTZ,

    -- Metadata
    metadata JSONB,
    callback_url TEXT,

    -- Metrics
    duration_ms BIGINT,

    -- Indexes
    INDEX idx_status (status),
    INDEX idx_created_at (created_at),
    INDEX idx_next_retry_at (next_retry_at),
    INDEX idx_backend (backend)
);
```

#### Proofs Table

```sql
CREATE TABLE proofs (
    task_id UUID PRIMARY KEY REFERENCES tasks(task_id),
    proof_receipt BYTEA NOT NULL,
    public_values BYTEA,
    verification_key BYTEA,

    -- Storage optimization
    compressed BOOLEAN DEFAULT false,
    compression_algorithm VARCHAR(20),

    -- Timestamps
    stored_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    accessed_at TIMESTAMPTZ,

    -- Indexes
    INDEX idx_stored_at (stored_at),
    FOREIGN KEY (task_id) REFERENCES tasks(task_id) ON DELETE CASCADE
);
```

#### Workers Table (for monitoring)

```sql
CREATE TABLE workers (
    worker_id VARCHAR(50) PRIMARY KEY,
    backend VARCHAR(20) NOT NULL,
    status VARCHAR(20) NOT NULL,
    current_task_id UUID REFERENCES tasks(task_id),

    -- Timestamps
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Metrics
    total_proofs INT DEFAULT 0,
    failed_proofs INT DEFAULT 0,
    average_duration_ms BIGINT,

    INDEX idx_status (status),
    INDEX idx_backend (backend)
);
```

### Storage Optimizations

1. **Proof Compression**: Compress large proofs using zstd or lz4
2. **Hot/Cold Storage**: Move old proofs to cheaper storage (S3/GCS)
3. **Proof Expiry**: Automatic cleanup after retention period
4. **Read Replicas**: Scale read operations with database replicas
5. **Caching**: Redis cache for frequently accessed proofs

---

## Security & Authentication

### API Key Authentication

```http
Authorization: Bearer paas_live_abc123def456...
```

API Key format: `paas_{environment}_{random}`
- Environment: `test`, `live`
- Random: 32 character alphanumeric

### Rate Limiting

Per API key:
- **Submissions**: 100 requests/minute
- **Status Checks**: 1000 requests/minute
- **Downloads**: 500 requests/minute

Rate limit headers:
```http
X-RateLimit-Limit: 100
X-RateLimit-Remaining: 73
X-RateLimit-Reset: 1698501600
```

### Request Validation

1. **Input Sanitization**: Validate all inputs against schema
2. **Size Limits**: Enforce maximum request/response sizes
3. **Content-Type Validation**: Ensure proper content types
4. **CORS Configuration**: Restrict origins in production

### TLS/HTTPS

- **Production**: TLS 1.3 required
- **Certificate Management**: Auto-renewal via Let's Encrypt
- **HSTS**: HTTP Strict Transport Security enabled

### Audit Logging

Log all API access:
```json
{
  "timestamp": "2025-10-28T10:30:00Z",
  "request_id": "req-550e8400",
  "api_key": "paas_live_abc...def",
  "endpoint": "/api/v1/proofs",
  "method": "POST",
  "status_code": 201,
  "duration_ms": 45,
  "ip_address": "203.0.113.42",
  "user_agent": "strata-client/1.0.0"
}
```

---

## Deployment Patterns

### 1. Docker Deployment

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin strata-paas

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/strata-paas /usr/local/bin/
EXPOSE 8080
ENTRYPOINT ["strata-paas"]
```

Run:
```bash
docker run -p 8080:8080 \
  -e PAAS_DATABASE_URL=postgresql://... \
  -e PAAS_API_KEY=paas_live_... \
  strata-paas:latest
```

### 2. Kubernetes Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: strata-paas
spec:
  replicas: 3
  selector:
    matchLabels:
      app: strata-paas
  template:
    metadata:
      labels:
        app: strata-paas
    spec:
      containers:
      - name: paas
        image: strata-paas:1.0.0
        ports:
        - containerPort: 8080
        env:
        - name: PAAS_DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: paas-secrets
              key: database-url
        resources:
          requests:
            memory: "2Gi"
            cpu: "1000m"
          limits:
            memory: "4Gi"
            cpu: "2000m"
        livenessProbe:
          httpGet:
            path: /health
            port: 8080
          initialDelaySeconds: 30
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /health
            port: 8080
          initialDelaySeconds: 5
          periodSeconds: 5
---
apiVersion: v1
kind: Service
metadata:
  name: strata-paas
spec:
  selector:
    app: strata-paas
  ports:
  - protocol: TCP
    port: 80
    targetPort: 8080
  type: LoadBalancer
```

### 3. Standalone Binary

```bash
# Configuration via environment variables
export PAAS_HOST=0.0.0.0
export PAAS_PORT=8080
export PAAS_DATABASE_URL=postgresql://localhost/paas
export PAAS_LOG_LEVEL=info

# Or configuration file
strata-paas --config /etc/paas/config.toml

# With systemd
sudo systemctl start strata-paas
sudo systemctl enable strata-paas
```

### Configuration File Format

```toml
# config.toml
[server]
host = "0.0.0.0"
port = 8080
tls_enabled = true
tls_cert_path = "/etc/paas/cert.pem"
tls_key_path = "/etc/paas/key.pem"

[guest_program]
program_path = "/var/lib/paas/guest_program.elf"
program_type = "checkpoint"

[worker_pools]
sp1_workers = 20
native_workers = 5

[database]
url = "postgresql://user:pass@localhost/paas"
max_connections = 20
connection_timeout = 30

[retry]
max_retries = 15
base_delay_seconds = 5
multiplier = 1.5
max_delay_seconds = 3600

[storage]
compression_enabled = true
retention_days = 30

[rate_limiting]
enabled = true
requests_per_minute = 100

[observability]
metrics_enabled = true
tracing_enabled = true
log_level = "info"
```

---

## Monitoring & Observability

### Key Metrics

#### Request Metrics
- `paas_requests_total{endpoint, method, status}` - Total API requests
- `paas_request_duration_seconds{endpoint}` - Request duration histogram
- `paas_request_size_bytes{endpoint}` - Request size histogram
- `paas_response_size_bytes{endpoint}` - Response size histogram

#### Proof Metrics
- `paas_proofs_total{status, backend}` - Total proofs by status
- `paas_proof_duration_seconds{backend}` - Proof generation duration
- `paas_queue_depth{backend}` - Number of queued tasks
- `paas_active_proofs{backend}` - Currently proving tasks

#### Worker Metrics
- `paas_workers_total{backend, status}` - Worker count by status
- `paas_worker_utilization{backend}` - Worker pool utilization %
- `paas_worker_errors_total{backend, error_type}` - Worker errors

#### Storage Metrics
- `paas_storage_operations_total{operation, status}` - Storage ops
- `paas_storage_size_bytes` - Total storage used
- `paas_storage_latency_seconds{operation}` - Storage operation latency

#### System Metrics
- `paas_memory_usage_bytes` - Memory usage
- `paas_cpu_usage_percent` - CPU usage
- `paas_goroutines_count` - Active goroutines/tasks

### Logging

Structured JSON logging:

```json
{
  "timestamp": "2025-10-28T10:30:00.123Z",
  "level": "INFO",
  "service": "paas",
  "trace_id": "550e8400-e29b-41d4-a716-446655440000",
  "span_id": "446655440000",
  "message": "Proof generation completed",
  "task_id": "550e8400-e29b-41d4-a716-446655440000",
  "backend": "sp1",
  "duration_ms": 4207,
  "proof_size_bytes": 1048576
}
```

Log Levels:
- **ERROR**: Unrecoverable errors requiring attention
- **WARN**: Recoverable errors, degraded operation
- **INFO**: Normal operations, proof completions
- **DEBUG**: Detailed debugging information

### Distributed Tracing

OpenTelemetry integration:

```rust
use opentelemetry::trace::{Span, Tracer};

async fn handle_proof_request(request: ProofRequest) -> Result<ProofResponse> {
    let tracer = global::tracer("paas");
    let mut span = tracer.start("handle_proof_request");

    span.set_attribute("task_id", request.task_id.to_string());
    span.set_attribute("backend", request.backend.to_string());

    // ... processing

    span.end();
}
```

### Alerting

Critical alerts:
- Worker pool exhaustion (> 90% utilization)
- High error rate (> 5% of requests)
- Database connection failures
- Storage capacity (> 80% full)
- High latency (p99 > 10s)

---

## Implementation Roadmap

### Phase 1: Foundation (Weeks 1-2)

**Goal**: Core service infrastructure

Tasks:
- [ ] Create `crates/paas/` directory structure
- [ ] Implement `ProverService` using service framework
- [ ] Design and implement storage trait
- [ ] Implement PostgreSQL storage backend
- [ ] Basic configuration management
- [ ] Unit tests for core components

Deliverables:
- Working `ProverService` with in-memory task management
- PostgreSQL schema and basic operations
- Configuration file support

### Phase 2: API Layer (Weeks 3-4)

**Goal**: RESTful API implementation

Tasks:
- [ ] Implement HTTP server (Axum/Actix)
- [ ] API endpoint handlers
- [ ] Request validation middleware
- [ ] Error handling and response formatting
- [ ] API documentation (OpenAPI/Swagger)
- [ ] Integration tests for all endpoints

Deliverables:
- Fully functional REST API
- OpenAPI specification
- Postman/Insomnia collection

### Phase 3: Proving Engine (Weeks 5-6)

**Goal**: Proof generation and worker management

Tasks:
- [ ] Implement worker pool management
- [ ] Integrate SP1 proving backend
- [ ] Implement retry logic with exponential backoff
- [ ] Progress tracking during proving
- [ ] Proof compression and storage optimization
- [ ] Worker health monitoring

Deliverables:
- End-to-end proof generation working
- Retry mechanism operational
- Worker pool scaling

### Phase 4: Production Readiness (Weeks 7-8)

**Goal**: Security, monitoring, and deployment

Tasks:
- [ ] API key authentication
- [ ] Rate limiting
- [ ] TLS/HTTPS support
- [ ] Metrics and Prometheus integration
- [ ] Structured logging
- [ ] Docker containerization
- [ ] Kubernetes manifests
- [ ] Load testing and optimization

Deliverables:
- Production-ready service
- Deployment documentation
- Performance benchmarks

### Phase 5: Advanced Features (Weeks 9-10)

**Goal**: Enhanced functionality

Tasks:
- [ ] Webhook notifications
- [ ] Batch proof requests
- [ ] Priority queue implementation
- [ ] Redis caching layer
- [ ] Proof archival to S3/GCS
- [ ] Admin dashboard

Deliverables:
- Feature-complete PaaS
- Admin tools
- Comprehensive documentation

---

## Integration Examples

### Example 1: Strata Sequencer Integration

```rust
// Sequencer submits checkpoint proof request
use paas_client::PaaSClient;

#[tokio::main]
async fn main() -> Result<()> {
    let client = PaaSClient::new("https://prover.example.com")
        .with_api_key("paas_live_abc123...");

    // Submit proof request
    let request = ProofRequest {
        context: ProofContext::Checkpoint { index: 42 },
        priority: Priority::High,
        backend: Backend::SP1,
        callback_url: Some("https://sequencer.example.com/webhook/proof".to_string()),
    };

    let task = client.submit_proof(request).await?;
    println!("Proof task submitted: {}", task.task_id);

    // Poll for completion
    loop {
        let status = client.get_task_status(task.task_id).await?;

        match status.status {
            TaskStatus::Completed { .. } => {
                let proof = client.get_proof(task.task_id).await?;
                println!("Proof generated: {} bytes", proof.receipt.len());
                break;
            }
            TaskStatus::Failed { error, .. } => {
                eprintln!("Proof failed: {:?}", error);
                break;
            }
            _ => {
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }

    Ok(())
}
```

### Example 2: CLI Tool

```bash
# Install CLI
cargo install strata-paas-cli

# Configure
paas-cli config set api-key paas_live_abc123...
paas-cli config set endpoint https://prover.example.com

# Submit proof
paas-cli proof submit \
  --context checkpoint:42 \
  --backend sp1 \
  --wait

# Output:
# ✓ Proof submitted: task_id=550e8400-e29b-41d4-a716-446655440000
# ⏳ Waiting for proof generation...
# ✓ Proof completed in 4.2s
# 💾 Proof saved to ./proof_checkpoint_42.bin

# List proofs
paas-cli proof list --status completed --limit 10

# Download proof
paas-cli proof download 550e8400-e29b-41d4-a716-446655440000 \
  --output ./my_proof.bin

# Get proof status
paas-cli proof status 550e8400-e29b-41d4-a716-446655440000
```

### Example 3: Python SDK

```python
from strata_paas import PaaSClient, ProofContext, Backend

# Initialize client
client = PaaSClient(
    endpoint="https://prover.example.com",
    api_key="paas_live_abc123..."
)

# Submit proof request
task = client.submit_proof(
    context=ProofContext.checkpoint(index=42),
    backend=Backend.SP1,
    callback_url="https://example.com/webhook"
)

print(f"Task ID: {task.task_id}")

# Wait for completion (blocking)
proof = client.wait_for_proof(task.task_id, timeout=300)

print(f"Proof size: {len(proof.receipt)} bytes")
print(f"Duration: {proof.duration_ms}ms")

# Or use async
import asyncio

async def generate_proof():
    task = await client.submit_proof_async(...)
    proof = await client.wait_for_proof_async(task.task_id)
    return proof

proof = asyncio.run(generate_proof())
```

### Example 4: Webhook Handler

```rust
// Server receiving webhook notifications
use actix_web::{post, web, HttpResponse};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct ProofWebhook {
    task_id: String,
    status: String,
    proof_url: Option<String>,
    error: Option<ErrorInfo>,
}

#[post("/webhook/proof")]
async fn handle_proof_webhook(
    webhook: web::Json<ProofWebhook>
) -> HttpResponse {
    match webhook.status.as_str() {
        "completed" => {
            println!("Proof completed: {}", webhook.task_id);

            // Fetch and use proof
            let proof_url = webhook.proof_url.unwrap();
            let proof = fetch_proof(&proof_url).await.unwrap();

            // Process proof...

            HttpResponse::Ok().finish()
        }
        "failed" => {
            eprintln!("Proof failed: {:?}", webhook.error);
            HttpResponse::Ok().finish()
        }
        _ => HttpResponse::BadRequest().finish()
    }
}
```

---

## Appendix

### A. API Error Codes Reference

| Code | HTTP Status | Description | Retryable |
|------|-------------|-------------|-----------|
| `INVALID_REQUEST` | 400 | Malformed request | No |
| `INVALID_CONTEXT` | 400 | Invalid proof context | No |
| `TASK_NOT_FOUND` | 404 | Task ID not found | No |
| `PROOF_NOT_READY` | 404 | Proof not yet completed | No |
| `UNAUTHORIZED` | 401 | Invalid API key | No |
| `RATE_LIMIT_EXCEEDED` | 429 | Rate limit exceeded | Yes |
| `INTERNAL_ERROR` | 500 | Server error | Yes |
| `STORAGE_ERROR` | 503 | Database unavailable | Yes |
| `WORKER_UNAVAILABLE` | 503 | No workers available | Yes |
| `PROVING_NETWORK_ERROR` | 500 | Network error during proving | Yes |
| `PROVING_TIMEOUT` | 500 | Proof generation timeout | Yes |
| `PROVING_INVALID_INPUT` | 400 | Invalid proving input | No |
| `PROVING_OUT_OF_MEMORY` | 500 | Insufficient memory | No |

### B. Performance Benchmarks

Target SLAs (Service Level Agreements):

| Metric | Target | Measurement |
|--------|--------|-------------|
| API Latency (p50) | < 50ms | Time to return proof status |
| API Latency (p99) | < 200ms | Time to return proof status |
| Proof Generation (SP1) | < 5min | Average time for checkpoint proof |
| Proof Generation (Native) | < 30s | Average time for checkpoint proof |
| Throughput | > 100 proofs/hour | With 20 SP1 workers |
| Uptime | 99.9% | Monthly uptime target |

### C. Glossary

- **Guest Program**: The program being proven (e.g., checkpoint validation logic)
- **Host Program**: The prover service running on the host system
- **ZkVM**: Zero-knowledge virtual machine (SP1, RISC0, etc.)
- **Proof Receipt**: The generated zero-knowledge proof artifact
- **Public Values**: Public outputs of the guest program
- **Verification Key**: Key used to verify the proof

### D. References

- [Strata Architecture Documentation](../CLAUDE.md)
- [Service Framework Documentation](../CLAUDE.md#service-framework-cratesservice)
- [SP1 Documentation](https://docs.succinct.xyz/)
- [OpenAPI Specification](https://swagger.io/specification/)
- [Prometheus Metrics](https://prometheus.io/docs/concepts/metric_types/)

---

**End of Document**
