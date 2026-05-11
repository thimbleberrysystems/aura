/// Parsed command actions from a `/aura` command line.
pub enum CmdAction {
    Status,
    Unknown(String),
}

/// Parse a line that follows the `/aura ` prefix (the caller should strip the
/// leading `/aura `). Returns a `CmdAction` describing what to do.
pub fn parse_command(line: &str) -> CmdAction {
    let s = line.trim();
    if s == "status" {
        CmdAction::Status
    } else {
        CmdAction::Unknown(s.to_string())
    }
}

/// Format a human-readable status string for the `status` command.
pub fn status_string() -> String {
    "OK".to_string()
}
