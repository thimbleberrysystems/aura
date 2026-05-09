use std::path::Path;
use std::sync::Arc;
use tonic::transport::Server;

pub use crate::grpc;
use crate::context::AppContext;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Serve gRPC over a Unix Domain Socket at `path` using the provided app context.
pub async fn serve_uds(path: &str, ctx: Arc<AppContext>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Remove stale socket if present.
    if Path::new(path).exists() {
        let _ = std::fs::remove_file(path);
    }
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Bind UDS listener and convert to tonic incoming stream.
    let uds = tokio::net::UnixListener::bind(path)?;
    // Set owner-only permissions on Unix.
    #[cfg(unix)]
    {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }

    let incoming = tokio_stream::wrappers::UnixListenerStream::new(uds);

    let svc = grpc::aura::aura_server::AuraServer::new(grpc::MyAura::new(ctx));
    Server::builder()
        .add_service(svc)
        .serve_with_incoming(incoming)
        .await?;
    Ok(())
}

/// Serve gRPC over TCP (address already supported by `grpc::serve_tcp`,
/// this function delegates to it).
pub async fn serve_tcp(addr: std::net::SocketAddr, ctx: Arc<AppContext>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    grpc::serve_tcp(addr, ctx).await
}
