//! Permanent address through Tailscale Funnel (`--tunnel funnel`): https://<computer>.<tailnet>.ts.net,
//! free, no own domain needed. Tailscale itself is installed and logged in by the owner; this
//! module only asks it to publish the local port and keeps that switched on.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

/// How often the published port is checked again (Tailscale restarts, settings reset).
const RECHECK: Duration = Duration::from_secs(600);

pub fn bin() -> Option<PathBuf> {
    crate::runner::which("tailscale").or_else(|| {
        let p = PathBuf::from(r"C:\Program Files\Tailscale\tailscale.exe");
        p.is_file().then_some(p)
    })
}

/// Output (stdout + stderr) of a tailscale command, given up after `limit`.
async fn tailscale(args: &[&str], limit: Duration) -> Result<(bool, String)> {
    let bin = bin().context("Tailscale не установлен: https://tailscale.com/download")?;
    let mut cmd = Command::new(bin);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = tokio::time::timeout(limit, cmd.output()).await.context("Tailscale не ответил вовремя")??;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text))
}

/// The computer's public name in Funnel ("pc.tail1234.ts.net"); fails when not logged in.
pub async fn dns_name() -> Result<String> {
    let (_, text) = tailscale(&["status", "--json"], Duration::from_secs(15)).await?;
    let json: serde_json::Value = serde_json::from_str(text.trim()).context("Tailscale вернул непонятный ответ")?;
    let state = json["BackendState"].as_str().unwrap_or("");
    if state != "Running" {
        bail!("Tailscale не подключён (состояние: {state}). Войдите: откройте Tailscale и нажмите Log in (или tailscale up)");
    }
    let name = json["Self"]["DNSName"].as_str().unwrap_or("").trim_end_matches('.').to_string();
    if name.is_empty() {
        bail!("Tailscale не сообщил имя компьютера");
    }
    Ok(name)
}

/// A link the owner must open once: Funnel (or HTTPS certificates) is off for the tailnet.
pub fn consent_link(text: &str) -> Option<String> {
    text.split_whitespace().find(|w| w.starts_with("https://login.tailscale.com/")).map(|w| w.trim_end_matches(['.', ',']).to_string())
}

/// Publishes http://127.0.0.1:<port> at https://<name>.ts.net (port 443). Repeating it is harmless.
pub async fn publish(port: u16) -> Result<()> {
    let target = format!("http://127.0.0.1:{port}");
    let (mut ok, mut text) = tailscale(&["funnel", "--bg", "--yes", &target], Duration::from_secs(30)).await?;
    // Older Tailscale has no --yes (it never asked there anyway).
    if !ok && text.contains("flag provided but not defined") {
        (ok, text) = tailscale(&["funnel", "--bg", &target], Duration::from_secs(30)).await?;
    }
    if let Some(link) = consent_link(&text) {
        bail!("в Tailscale ещё не разрешён Funnel. Откройте ссылку и разрешите, затем повторите: {link}");
    }
    if !ok {
        bail!("tailscale funnel: {}", text.trim());
    }
    Ok(())
}

/// Stops publishing (back to the Cloudflare tunnel).
pub async fn unpublish() {
    let _ = tailscale(&["funnel", "--https=443", "off"], Duration::from_secs(15)).await;
}

/// Keeps the port published for as long as the cloud runs; `on_url` once it is up.
pub async fn run<F: Fn(String)>(port: u16, url: String, on_url: F) {
    let mut announced = false;
    let mut last_error = String::new();
    loop {
        match publish(port).await {
            Ok(()) => {
                last_error.clear();
                if !announced {
                    announced = true;
                    on_url(url.clone());
                }
            }
            Err(e) => {
                let e = format!("{e:#}");
                if e != last_error {
                    crate::log!("Tailscale Funnel: {e}");
                    last_error = e;
                }
            }
        }
        tokio::time::sleep(if announced { RECHECK } else { Duration::from_secs(60) }).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_consent_link() {
        let text = "Funnel is not enabled on your tailnet.\nTo enable, visit:\n\n         https://login.tailscale.com/f/funnel?node=abc123\n";
        assert_eq!(consent_link(text).as_deref(), Some("https://login.tailscale.com/f/funnel?node=abc123"));
        assert_eq!(consent_link("Available on the internet: https://pc.tail1.ts.net/"), None);
    }
}
