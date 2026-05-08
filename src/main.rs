use anyhow::Context;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::env;
use std::io::{self, Read, Write};

fn main() -> anyhow::Result<()> {
    let pty_system = native_pty_system();
    let size = PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    };
    let pty_pair = pty_system.openpty(size).context("openpty failed")?;

    let shell = env::var("SHELL").unwrap_or_else(|_| String::from("/bin/sh"));
    let cmd = CommandBuilder::new(shell.clone());

    let mut child = pty_pair
        .slave
        .spawn_command(cmd)
        .context("spawn_command failed")?;

    // Get reader and writer for master
    let mut reader = pty_pair
        .master
        .try_clone_reader()
        .context("clone reader failed")?;
    let mut writer = pty_pair
        .master
        .take_writer()
        .context("take writer failed")?;

    // Thread: stdin -> pty
    let write_thread = std::thread::spawn(move || -> anyhow::Result<()> {
        let mut stdin = io::stdin();
        io::copy(&mut stdin, &mut writer)?;
        Ok(())
    });

    // main thread: pty -> stdout
    let mut stdout = io::stdout();
    io::copy(&mut reader, &mut stdout).context("copy pty->stdout failed")?;

    let _ = write_thread.join();
    let _ = child.wait()?;
    Ok(())
}
