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

    // TCP-only client
    let tcp = env::var("AURA_CONTROL_TCP").unwrap_or_else(|_| "127.0.0.1:40001".to_string());
    let mut stream = std::net::TcpStream::connect(&tcp)?;
    stream.write_all(cmdline.as_bytes())?;
    let mut out = Vec::new();
    stream.read_to_end(&mut out)?;
    std::io::stdout().write_all(&out)?;
    Ok(())
}
