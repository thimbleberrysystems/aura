use std::sync::Arc;
use tonic::{Request, Response, Status};

pub mod aura {
    tonic::include_proto!("aura");
}

use aura::aura_server::Aura;
use aura::{StatusReply, StatusRequest};

use crate::context::AppContext;

#[derive(Clone)]
pub struct MyAura {
    ctx: Arc<AppContext>,
}

impl MyAura {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }
}

#[tonic::async_trait]
impl Aura for MyAura {
    async fn status(&self, _request: Request<StatusRequest>) -> Result<Response<StatusReply>, Status> {
        let ctx = self.ctx.clone();
        // Run blocking status computation off the tokio runtime.
        let result = tokio::task::spawn_blocking(move || {
            crate::tools::status::compute_status_blocking(&ctx)
        })
        .await
        .map_err(|e| Status::internal(format!("join error: {}", e)))?;

        match result {
            Ok(s) => Ok(Response::new(StatusReply { status: s })),
            Err(e) => Err(Status::internal(format!("status error: {}", e))),
        }
    }
}

pub async fn serve_tcp(addr: std::net::SocketAddr, ctx: Arc<AppContext>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let svc = aura::aura_server::AuraServer::new(MyAura::new(ctx));
    tonic::transport::Server::builder()
        .add_service(svc)
        .serve(addr)
        .await?;
    Ok(())
}
