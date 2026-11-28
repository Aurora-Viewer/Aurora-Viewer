//! Local bearer-token authentication shared by the viewer's MCP server and the
//! `aurora-mcp-stdio` proxy.
//!
//! The server needs a token by default. `AURORA_MCP_TOKEN` sets it; otherwise a
//! random one is made at every launch. Either way it is written to a file only
//! the user can read ([`token_file_path`]). The stdio proxy reads that file by
//! itself, so agents need no setup. Only `AURORA_MCP_NO_AUTH=1` turns
//! authentication off.

use std::path::PathBuf;
use uuid::Uuid;

/// The file holding the server's token for `port`.
///
/// `$XDG_RUNTIME_DIR` (per-user, 0700) when set; otherwise
/// `%LOCALAPPDATA%\Aurora` on Windows or `~/.cache/aurora` elsewhere; the
/// system temp dir as a last resort.
pub fn token_file_path(port: u16) -> PathBuf {
    let name = format!("aurora-mcp-{port}.token");
    if let Some(dir) = std::env::var_os("AURORA_MCP_TOKEN_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join(name);
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join(name);
    }
    if cfg!(windows)
        && let Some(dir) = std::env::var_os("LOCALAPPDATA").filter(|d| !d.is_empty())
    {
        return PathBuf::from(dir).join("Aurora").join(name);
    }
    if let Some(home) = std::env::var_os("HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(home).join(".cache").join("aurora").join(name);
    }
    std::env::temp_dir().join(name)
}

/// `AURORA_MCP_NO_AUTH=1`: the server takes requests without a token.
pub fn auth_disabled() -> bool {
    matches!(
        std::env::var("AURORA_MCP_NO_AUTH").as_deref().map(str::trim),
        Ok("1" | "true" | "yes")
    )
}

fn env_token() -> Option<String> {
    std::env::var("AURORA_MCP_TOKEN")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// 256 random bits from the OS generator (two v4 UUIDs), hex-encoded.
pub fn generate_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// The server side: the token to require (None when authentication is off),
/// and the file it was published to.
pub fn server_token(port: u16) -> (Option<String>, Option<PathBuf>) {
    if auth_disabled() {
        log::warn!("aurora-mcp: AURORA_MCP_NO_AUTH=1, the MCP endpoint accepts any local client");
        return (None, None);
    }
    let token = env_token().unwrap_or_else(generate_token);
    let path = token_file_path(port);
    match write_private(&path, &token) {
        Ok(()) => (Some(token), Some(path)),
        Err(e) => {
            log::warn!(
                "aurora-mcp: could not write the token file {}: {e}; set AURORA_MCP_TOKEN for clients",
                path.display()
            );
            (Some(token), None)
        }
    }
}

/// The client side (stdio proxy): `AURORA_MCP_TOKEN`, else the token file
/// the running viewer wrote for `port`. Read on every request, so a viewer
/// restart (with a new token) is picked up.
pub fn client_token(port: u16) -> Option<String> {
    env_token().or_else(|| {
        std::fs::read_to_string(token_file_path(port))
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
    })
}

fn write_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // only tighten a directory we own and made for this (not $XDG_RUNTIME_DIR / tmp)
            if dir.ends_with("aurora") {
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
        }
    }
    // never follow a file someone else planted: remove, then create fresh
    let _ = std::fs::remove_file(path);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    let mut f = opts.open(path)?;
    f.write_all(contents.as_bytes())?;
    f.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_long_and_distinct() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    #[test]
    fn private_file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("aurora-mcp-auth-test-{}", Uuid::new_v4().simple()));
        let path = dir.join("t.token");
        write_private(&path, "abc").expect("write");
        write_private(&path, "def").expect("rewrite");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "def");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
