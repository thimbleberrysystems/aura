use std::io::Result as IoResult;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// Parsed command actions from a single-line control command.
enum CmdAction {
    ConfigShow,
    Unknown(String),
}

/// Parse a single-line control command and return a `CmdAction`.
fn parse_command(line: &str) -> CmdAction {
    let s = line.trim();
    match s {
        "config show" | "config" => CmdAction::ConfigShow,
        _ => CmdAction::Unknown(s.to_string()),
    }
}

/// Handle `config show` by querying the `Config` accessors and writing output.
async fn handle_config_show<W>(cfg: &crate::cfg::Config, w: &mut W) -> IoResult<()>
where
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let (logging_effective, logging_src) = cfg.logging_with_source();
    let (model_effective, model_src) = cfg.model_with_source();
    let (disable_summary_effective, disable_summary_src) = cfg.disable_summary_with_source();
    let (summarize_threshold_effective, summarize_threshold_src) = cfg.summarize_threshold_with_source();
    let (summarize_timeout_effective, summarize_timeout_src) = cfg.summarize_timeout_secs_with_source();
    let (control_tcp_effective, control_tcp_src) = cfg.control_tcp_with_source();

    let src_name = |s: crate::cfg::Source| match s {
        crate::cfg::Source::Env => "env",
        crate::cfg::Source::Config => "config file",
        crate::cfg::Source::Default => "default",
    };

    let mut out = String::new();
    out.push_str(&format!("logging: {} ({})\n", logging_effective, src_name(logging_src)));
    out.push_str(&format!("model: {} ({})\n", model_effective, src_name(model_src)));
    out.push_str(&format!("disable_summary: {} ({})\n", disable_summary_effective, src_name(disable_summary_src)));
    out.push_str(&format!("summarize_threshold: {} ({})\n", summarize_threshold_effective, src_name(summarize_threshold_src)));
    out.push_str(&format!("summarize_timeout_secs: {} ({})\n", summarize_timeout_effective, src_name(summarize_timeout_src)));
    out.push_str(&format!("control_tcp: {} ({})\n", control_tcp_effective, src_name(control_tcp_src)));

    if let Err(e) = w.write_all(out.as_bytes()).await {
        tracing::error!("tcp write error: {}", e);
        return Err(e);
    }
    Ok(())
}

async fn handle_unknown<W>(u: &str, w: &mut W) -> IoResult<()>
where
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let msg = format!("Unknown aura command: {}\n", u);
    if let Err(e) = w.write_all(msg.as_bytes()).await {
        tracing::error!("tcp write error: {}", e);
        return Err(e);
    }
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

    let cfg = crate::cfg::load_config();
    match parse_command(&cmdline) {
        CmdAction::ConfigShow => {
            if let Err(e) = handle_config_show(&cfg, &mut w).await {
                tracing::error!("process command error: {}", e);
                return Err(e);
            }
        }
        CmdAction::Unknown(u) => {
            if let Err(e) = handle_unknown(&u, &mut w).await {
                tracing::error!("process command error: {}", e);
                return Err(e);
            }
        }
    }

    // flush and close; log any flush error rather than ignore it
    if let Err(e) = w.flush().await {
        tracing::error!("tcp write flush error: {}", e);
    }
    Ok(())
}

async fn run_control_server() -> anyhow::Result<()> {
    // Start TCP loopback listener (portable fallback for Windows)
    let cfg_for_server = crate::cfg::load_config();
    let (tcp_addr, _tcp_src) = cfg_for_server.control_tcp_with_source();
    let tcp_listener = TcpListener::bind(&tcp_addr).await?;
    tracing::info!("control: tcp listening on {}", tcp_addr);

    // Accept loop — spawn a task per connection.
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

    Ok(())
}

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
