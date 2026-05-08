use crate::context::AppContext;
use anyhow::Result;

/// Blocking status computation. This may perform file I/O or other blocking work,
/// so callers should run it inside `tokio::task::spawn_blocking`.
pub fn compute_status_blocking(_ctx: &AppContext) -> Result<String> {
	// For now, return a simple OK that could include runtime info.
	// Use `_ctx` to show context is available (e.g., for uptime).
	let uptime = _ctx.uptime_seconds();
	Ok(format!("OK uptime={}s", uptime))
}
