use anyhow::Context;
use std::io::{Read, Write};

use aura::cfg::load_config;

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    if args.len() <= 1 {
        eprintln!("usage: aura-cli <command>");
        std::process::exit(2);
    }
    args.remove(0);
    let cmd = args.join(" ");
    let cmdline = format!("{}\n", cmd);

    let cfg = load_config().context("failed to load configuration")?;
    let tcp = cfg.control_tcp().context("control_tcp is missing in config file; set server.control_tcp in config/aura.toml")?;
    let mut stream = std::net::TcpStream::connect(&tcp)?;
    stream.write_all(cmdline.as_bytes())?;
    let mut out = Vec::new();
    stream.read_to_end(&mut out)?;
    std::io::stdout().write_all(&out)?;
    Ok(())
}
