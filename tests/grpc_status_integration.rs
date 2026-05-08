// Integration test: starts the gRPC server and calls the `Status` RPC.
//
// This test exercises the real server code from `crate::grpc` over TCP
// (127.0.0.1:50052). It verifies the RPC returns the expected string.
//
// Running `cargo test` will compile the proto-generated code (via build.rs),
// start the server in a background task, call the client, assert the reply,
// and then abort the server task.

use std::time::Duration;

#[tokio::test]
async fn status_returns_ok() -> Result<(), Box<dyn std::error::Error>> {
    // Use a test-only port to avoid colliding with real runs.

    let addr = "127.0.0.1:50052".parse().unwrap();

    // Spawn the gRPC server from the library module.
    // Create a shared app context and start the server with it.
    let ctx = std::sync::Arc::new(aura::context::AppContext::new());
    let server_handle = tokio::spawn(async move {
        // This will run until aborted.
        aura::grpc::serve_tcp(addr, ctx).await.expect("gRPC server failed");
    });

    // Give the server a short moment to bind and start.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Create a client and call Status RPC.
    let mut client = aura::grpc::aura::aura_client::AuraClient::connect("http://127.0.0.1:50052").await?;
    let req = aura::grpc::aura::StatusRequest {};
    let resp = client.status(req).await?;
    let got = resp.into_inner().status;
    assert!(got.starts_with("OK"), "unexpected status: {}", got);

    // Stop the server task.
    server_handle.abort();

    Ok(())
}
