use std::sync::Arc;
use std::io::Result as IoResult;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use aura::context::AppContext;

/// Start the control server in the background. This spawns listeners for TCP
/// (loopback) and, on Unix, a Unix-domain socket. Each accepted connection is
/// handled by reading a single command line and writing a single-line reply.
pub fn start_control_server(app_ctx: Arc<AppContext>) {
    tokio::spawn(async move {
        if let Err(e) = run_control_server(app_ctx).await {
            tracing::error!("control server failed: {}", e);
        }
    });
}

async fn run_control_server(app_ctx: Arc<AppContext>) -> anyhow::Result<()> {
    // Start TCP loopback listener (portable fallback for Windows)
    let tcp_addr = std::env::var("AURA_CONTROL_TCP").unwrap_or_else(|_| "127.0.0.1:40001".to_string());
    let tcp_listener = TcpListener::bind(&tcp_addr).await?;
    tracing::info!("control: tcp listening on {}", tcp_addr);

    let tcp_ctx = Arc::clone(&app_ctx);
    tokio::spawn(async move {
        loop {
            match tcp_listener.accept().await {
                Ok((stream, _peer)) => {
                    let ctx = Arc::clone(&tcp_ctx);
                    tokio::spawn(async move {
                        if let Err(e) = handle_stream(stream, ctx).await {
                            tracing::error!("tcp conn error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("tcp accept error: {}", e);
                    break;
                }
            }
        }
    });

    // Unix-domain socket listener (Unix only)
    #[cfg(unix)]
    {
        use std::path::Path;
        use std::os::unix::fs::PermissionsExt;
        use tokio::net::UnixListener;

        let socket_path = std::env::var("AURA_CONTROL_SOCKET").unwrap_or_else(|_| {
            match std::env::var("XDG_RUNTIME_DIR") {
                Ok(dir) => format!("{}/aura.sock", dir),
                Err(_) => "/tmp/aura.sock".to_string(),
            }
        });

        if let Some(parent) = Path::new(&socket_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // remove stale socket
        let _ = std::fs::remove_file(&socket_path);

        let uds_listener = UnixListener::bind(&socket_path)?;
        // restrict permissions to user
        let _ = std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o700));
        tracing::info!("control: unix socket listening on {}", socket_path);

        let uds_ctx = Arc::clone(&app_ctx);
        tokio::spawn(async move {
            loop {
                match uds_listener.accept().await {
                    Ok((stream, _peer)) => {
                        let ctx = Arc::clone(&uds_ctx);
                        tokio::spawn(async move {
                            if let Err(e) = handle_stream(stream, ctx).await {
                                tracing::error!("uds conn error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        tracing::error!("uds accept error: {}", e);
                        break;
                    }
                }
            }
        });
    }

    Ok(())
}

async fn handle_stream<S>(stream: S, app_ctx: Arc<AppContext>) -> IoResult<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (r, mut w) = tokio::io::split(stream);
    let mut reader = BufReader::new(r);
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(());
    }
    let cmdline = line.trim_end_matches(&['\r', '\n'][..]).to_string();
    tracing::debug!("control: received command='{}'", cmdline);

    match aura::cmd::parse_command(&cmdline) {
        aura::cmd::CmdAction::Status => {
            let st = aura::cmd::status_string(&app_ctx);
            // ensure reply starts at column 0
            w.write_all(st.as_bytes()).await?;
            w.write_all(b"\n").await?;
        }
        aura::cmd::CmdAction::Unknown(u) => {
            let msg = format!("Unknown aura command: {}\n", u);
            w.write_all(msg.as_bytes()).await?;
        }
    }

    // flush and close
    let _ = w.flush().await;
    Ok(())
}
