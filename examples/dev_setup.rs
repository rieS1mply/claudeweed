//! Creates a throwaway development setup in `.dev/` (git-ignored), so the cloud can be run
//! without the interactive first-run questions and without touching the real one:
//!
//! ```sh
//! cargo run --example dev_setup
//! cargo run -- --dir .dev --no-tunnel --port 8788
//! ```
//!
//! Site: http://127.0.0.1:8788, login `dev`, password `devpassword`. Terminal is on,
//! Telegram is off, files go to `.dev/storage`. Run it again to reset the login.

use argon2::password_hash::{PasswordHasher, SaltString};
use argon2::Argon2;

const USERNAME: &str = "dev";
const PASSWORD: &str = "devpassword";
const PORT: u16 = 8788;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::current_dir()?.join(".dev");
    let storage = dir.join("storage");
    std::fs::create_dir_all(&storage)?;

    let mut salt = [0u8; 16];
    getrandom::getrandom(&mut salt).map_err(|e| e.to_string())?;
    let salt = SaltString::encode_b64(&salt).map_err(|e| e.to_string())?;
    let hash = Argon2::default().hash_password(PASSWORD.as_bytes(), &salt).map_err(|e| e.to_string())?.to_string();

    let config = serde_json::json!({
        "storage_dir": storage,
        "username": USERNAME,
        "password_hash": hash,
        "port": PORT,
        "terminal": true,
    });
    std::fs::write(dir.join("config.json"), serde_json::to_vec_pretty(&config)?)?;

    println!("Готово: {}", dir.display());
    println!("Запуск:  cargo run -- --dir .dev --no-tunnel --port {PORT}");
    println!("Сайт:    http://127.0.0.1:{PORT}  (логин {USERNAME}, пароль {PASSWORD})");
    Ok(())
}
