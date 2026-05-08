use tonic::{Request, Response, Status};

pub mod aura {
    tonic::include_proto!("aura");
}

use aura::aura_server::Aura;
use aura::{StatusReply, StatusRequest};

#[derive(Default, Clone)]
pub struct MyAura {}

#[tonic::async_trait]
impl Aura for MyAura {
    async fn status(&self, _request: Request<StatusRequest>) -> Result<Response<StatusReply>, Status> {
        let reply = StatusReply { status: "OK".to_string() };
        Ok(Response::new(reply))
    }
}

pub async fn serve_tcp(addr: std::net::SocketAddr) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let svc = aura::aura_server::AuraServer::new(MyAura::default());
    tonic::transport::Server::builder()
        .add_service(svc)
        .serve(addr)
        .await?;
    Ok(())
}
