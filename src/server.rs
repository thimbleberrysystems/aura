use std::io::Result as IoResult;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// Start the control server in the background. This spawns listeners for TCP
/// (loopback) and, on Unix, a Unix-domain socket. Each accepted connection is
/// handled by reading a single command line and writing a single-line reply.
pub fn start_control_server() {
    tokio::spawn(async move {
        if let Err(e) = run_control_server().await {
            tracing::error!("control server failed: {}", e);
        }
    });
}

async fn run_control_server() -> anyhow::Result<()> {
    // Start TCP loopback listener (portable fallback for Windows)
    let tcp_addr = std::env::var("AURA_CONTROL_TCP").unwrap_or_else(|_| "127.0.0.1:40001".to_string());
    let tcp_listener = TcpListener::bind(&tcp_addr).await?;
    tracing::info!("control: tcp listening on {}", tcp_addr);

    tokio::spawn(async move {
        loop {
            match tcp_listener.accept().await {
                Ok((stream, _peer)) => {
                    tokio::spawn(async move {
                        if let Err(e) = handle_stream(stream).await {
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

    Ok(())
}

async fn handle_stream<S>(stream: S) -> IoResult<()>
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

    match crate::cmd::parse_command(&cmdline) {
        crate::cmd::CmdAction::Status => {
            let st = crate::cmd::status_string();
            // ensure reply starts at column 0
            w.write_all(st.as_bytes()).await?;
            w.write_all(b"\n").await?;
        }
        crate::cmd::CmdAction::Unknown(u) => {
            let msg = format!("Unknown aura command: {}\n", u);
            w.write_all(msg.as_bytes()).await?;
        }
    }

    // flush and close
    let _ = w.flush().await;
    Ok(())
}
