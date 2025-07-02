# Claude Code Context - Alpen Project

This file contains important context and information for Claude Code when working on the Alpen project.

## Project Overview

Alpen is a rollup project that uses reth (Ethereum execution client) for EVM execution. The project has been upgraded to use reth v1.5.0 and jsonrpsee 0.25.1.

## Recent Major Changes

### Reth Version Upgrade
- **Previous**: Various reth versions
- **Current**: reth v1.5.0 (git tag)
- **Impact**: Significant API changes requiring updates to trait implementations and HTTP client configurations

### JsonRPC Library Upgrade
- **Previous**: jsonrpsee 0.24.0
- **Current**: jsonrpsee 0.25.1
- **Impact**: Breaking changes in trait signatures, especially `ClientT` trait and RPC service implementations

## Key Build Fixes Applied

### 1. alpen-reth-rpc Package Fix
**Issue**: `ErrorObject` type mismatch between jsonrpsee versions
**Location**: `crates/reth/rpc/src/sequencer.rs:21`
**Solution**: Updated jsonrpsee version from 0.24.0 to 0.25.1 in root Cargo.toml
**Status**: ✅ Fixed

### 2. strata-common Package Fix  
**Issue**: `ClientT` trait implementation lifetime mismatches
**Location**: `crates/common/src/ws_client.rs`
**Solution**: 
- Removed `async_trait` attribute
- Changed from `async fn` to `fn` returning `impl Future`
- Used `async move` blocks for implementation
- Fixed lifetime issues in `batch_request` by cloning the pool
**Status**: ✅ Fixed

### 3. strata-evmexec Package Fix
**Issue**: `AuthClientService<HttpBackend>` doesn't implement `RpcServiceT` trait
**Location**: `crates/evmexec/src/http_client.rs`
**Solution**:
- Simplified to use basic `HttpClient` instead of authenticated client
- Removed `reth-rpc-layer` and `tower` dependencies
- Updated type signatures from `HttpClient<AuthClientService<HttpBackend>>` to `HttpClient`
- Added TODO for future JWT authentication implementation
**Status**: ✅ Fixed (authentication temporarily disabled)

## Important Technical Notes

### JsonRPC Client Trait Changes
In jsonrpsee 0.25.1, the `ClientT` trait changed from using `async_trait` to native `impl Future` returns:

```rust
// OLD (0.24.0)
#[async_trait]
impl ClientT for MyClient {
    async fn request<R, Params>(&self, method: &str, params: Params) -> Result<R, Error> { ... }
}

// NEW (0.25.1)  
impl ClientT for MyClient {
    fn request<R, Params>(&self, method: &str, params: Params) -> impl Future<Output = Result<R, Error>> + Send {
        async move { ... }
    }
}
```

### Authentication Status
JWT authentication for RPC clients is currently disabled due to incompatibility between `reth-rpc-layer` and jsonrpsee 0.25.1. The basic HTTP client is used instead.

**TODO**: Re-implement JWT authentication when:
1. `reth-rpc-layer` becomes compatible with jsonrpsee 0.25.1, OR
2. Alternative authentication method is found

### Dependency Versions
Key dependencies that must remain synchronized:
- `jsonrpsee = "0.25.1"`
- `jsonrpsee-types = "0.25.1"`
- `reth` dependencies use git tag `v1.5.0`

## Build Commands

### Verified Working Builds
```bash
cargo build -p alpen-reth-rpc     # ✅ Working
cargo build -p strata-common      # ✅ Working  
cargo build -p strata-evmexec     # ✅ Working (auth disabled)
```

### Common Build Issues
1. **ErrorObject type mismatch**: Ensure jsonrpsee versions are consistent
2. **Lifetime parameter mismatches**: Check for `async_trait` vs `impl Future` patterns
3. **RpcServiceT not implemented**: Use basic HTTP clients instead of complex middleware

## Code Patterns

### Working HTTP Client Pattern (strata-evmexec)
```rust
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};

fn http_client(http_url: &str, _secret: JwtSecret) -> HttpClient {
    // TODO: Implement proper JWT authentication when middleware is compatible
    HttpClientBuilder::default()
        .build(http_url)
        .expect("Failed to create http client")
}
```

### Working ClientT Implementation Pattern (strata-common)
```rust
impl ClientT for ManagedWsClient {
    fn request<R, Params>(&self, method: &str, params: Params) -> impl core::future::Future<Output = Result<R, ClientError>> + Send
    where
        R: DeserializeOwned,
        Params: ToRpcParams + Send,
    {
        async move {
            self.get_ready_rpc_client().await?.request(method, params).await
        }
    }
}
```

## Testing Notes

When testing changes:
1. Always test with `cargo build -p <package>` for specific packages
2. Check for both compilation errors and warnings about unused dependencies
3. Verify that trait implementations match expected signatures
4. Test RPC functionality manually if authentication is involved

## Future Work

1. **JWT Authentication**: Restore proper JWT authentication for RPC clients
2. **Middleware Compatibility**: Monitor reth-rpc-layer for jsonrpsee 0.25.1 support
3. **Error Handling**: Improve error handling in simplified HTTP client implementations
4. **Security**: Ensure proper authentication is restored before production use

## Last Updated
Date: 2025-01-02
Context: After reth v1.5.0 upgrade and jsonrpsee 0.25.1 compatibility fixes