use std::env;
use std::io::{Read, Write};

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = env::args().collect();
    if args.len() <= 1 {
        eprintln!("usage: aura-cli <command>");
        std::process::exit(2);
    }
    args.remove(0);
    let cmd = args.join(" ");
    let cmdline = format!("{}\n", cmd);

    // Try Unix-domain socket first (on Unix), then TCP fallback.
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let socket = env::var("AURA_CONTROL_SOCKET").unwrap_or_else(|_| {
            match env::var("XDG_RUNTIME_DIR") {
                Ok(dir) => format!("{}/aura.sock", dir),
                Err(_) => "/tmp/aura.sock".to_string(),
            }
        });
        if let Ok(mut s) = UnixStream::connect(&socket) {
            s.write_all(cmdline.as_bytes())?;
            let mut out = Vec::new();
            s.read_to_end(&mut out)?;
            std::io::stdout().write_all(&out)?;
            return Ok(());
        }
    }

    // TCP fallback
    let tcp = env::var("AURA_CONTROL_TCP").unwrap_or_else(|_| "127.0.0.1:40001".to_string());
    let mut stream = std::net::TcpStream::connect(&tcp)?;
    stream.write_all(cmdline.as_bytes())?;
    let mut out = Vec::new();
    stream.read_to_end(&mut out)?;
    std::io::stdout().write_all(&out)?;
    Ok(())
}
