//! Search photos by what is in them ("собака", "костёр", "чек"): a local vision model in Ollama
//! (`--ai on`) writes a short description with keywords for every photo, in the background,
//! one photo at a time. Nothing leaves the computer. The descriptions live in vision.json next
//! to the settings and come with the photo list, so the page's search finds them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::Config;
use crate::storage::{self, Kind, Section};
use crate::web::App;

pub const DEFAULT_MODEL: &str = "qwen2.5vl:7b";
const OLLAMA: &str = "http://127.0.0.1:11434";
/// New photos are looked for this often.
const SCAN_EVERY: Duration = Duration::from_secs(60);
/// Longer side of the picture the model sees.
const SIDE: u32 = 768;
/// The first photo also loads the model into memory, which can take a minute.
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(300);
/// `ollama serve` is started by us at most this often, when Ollama doesn't answer.
const START_EVERY: Duration = Duration::from_secs(600);

/// With an example: without one the model copies the wording of the instructions instead.
const PROMPT: &str = "Опиши фото для поиска по нему. Ответь одной строкой в формате:\n\
что на фото по-русски (5–15 слов) | ключевые слова по-русски через запятую | те же ключевые слова по-английски через запятую\n\
Пример: Рыжая собака спит на диване в комнате | собака, диван, комната, сон | dog, sofa, room, sleep\n\
Если на фото есть заметный текст — добавь его главные слова. Пиши только эту строку.";

#[derive(Serialize, Deserialize, Clone)]
struct Entry {
    desc: String,
    /// The file as it was described; a changed file is described again.
    size: u64,
    mtime: i64,
}

#[derive(Default)]
struct Status {
    /// What stops descriptions right now (Ollama off, model missing), for the page and --status.
    problem: Option<String>,
    started_ollama: Option<Instant>,
}

pub struct Vision {
    file: PathBuf,
    /// Photos-section path → description.
    index: Mutex<HashMap<String, Entry>>,
    status: Mutex<Status>,
}

impl Vision {
    pub fn load(file: &Path) -> Vision {
        let index = std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Vision { file: file.to_path_buf(), index: Mutex::new(index), status: Default::default() }
    }

    fn save(&self, index: &HashMap<String, Entry>) {
        let tmp = self.file.with_extension("tmp");
        if serde_json::to_vec(index).ok().and_then(|b| std::fs::write(&tmp, b).ok()).is_some() {
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }

    /// Path → description, for the photo list.
    pub fn descriptions(&self) -> HashMap<String, String> {
        self.index.lock().unwrap().iter().map(|(p, e)| (p.clone(), e.desc.clone())).collect()
    }

    pub fn described(&self) -> usize {
        self.index.lock().unwrap().len()
    }

    pub fn problem(&self) -> Option<String> {
        self.status.lock().unwrap().problem.clone()
    }

    fn set_problem(&self, problem: Option<String>) {
        let mut st = self.status.lock().unwrap();
        if st.problem != problem {
            if let Some(p) = &problem {
                crate::log!("Поиск по фото: {p}");
            }
            st.problem = problem;
        }
    }
}

/// Model from the settings (read live, like the other switches); None when turned off.
pub fn model(config_file: &Path) -> Option<String> {
    let config = Config::load(config_file).ok().flatten()?;
    config.vision.then(|| config.vision_model.filter(|m| !m.trim().is_empty()).unwrap_or_else(|| DEFAULT_MODEL.to_string()))
}

/// Photos that can be shown to the model: decodable ones, and HEIC once a browser made its preview.
fn picture(app: &App, rel: &str, path: &Path, meta: &std::fs::Metadata) -> Option<PathBuf> {
    match Kind::of(rel) {
        Kind::Image => Some(path.to_path_buf()),
        Kind::Raw => Some(crate::thumbs::cache_path(&app.paths.thumbs, "preview", rel, meta)).filter(|p| p.is_file()),
        _ => None,
    }
}

pub fn spawn(app: App) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(SCAN_EVERY).await;
            if let Some(model) = model(&app.paths.config) {
                pass(&app, &model).await;
            }
        }
    });
}

/// Describes what is new since the last pass; stops at the first error (tried again next time).
async fn pass(app: &App, model: &str) {
    let root = app.section_root(Section::Photos);
    let items = tokio::task::spawn_blocking({
        let root = root.clone();
        move || storage::list(&root)
    })
    .await
    .unwrap_or_default();
    // Forget photos that are gone; keep the rest unless the file changed.
    let todo: Vec<(String, u64, i64)> = {
        let mut index = app.vision.index.lock().unwrap();
        let before = index.len();
        let mtimes: HashMap<&str, (u64, i64)> = items.iter().map(|i| (i.path.as_str(), (i.size, i.date))).collect();
        index.retain(|p, e| mtimes.get(p.as_str()) == Some(&(e.size, e.mtime)));
        if index.len() != before {
            app.vision.save(&index);
        }
        items.iter().filter(|i| matches!(i.kind, Kind::Image | Kind::Raw) && !index.contains_key(&i.path)).map(|i| (i.path.clone(), i.size, i.date)).collect()
    };
    if todo.is_empty() {
        return;
    }
    if let Err(e) = ready(app, model).await {
        app.vision.set_problem(Some(format!("{e:#}")));
        return;
    }
    for (rel, size, mtime) in todo {
        if self::model(&app.paths.config).as_deref() != Some(model) {
            return; // turned off or switched meanwhile
        }
        let Some(path) = storage::resolve(&root, &rel) else { continue };
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        let Some(src) = picture(app, &rel, &path, &meta) else { continue };
        let jpeg = match tokio::task::spawn_blocking(move || crate::thumbs::jpeg_for_model(&src, SIDE)).await {
            Ok(Ok(j)) => j,
            _ => continue, // undecodable: skip quietly, like the thumbnail does
        };
        match describe(model, &jpeg).await {
            Ok(desc) => {
                app.vision.set_problem(None);
                let mut index = app.vision.index.lock().unwrap();
                index.insert(rel, Entry { desc, size, mtime });
                app.vision.save(&index);
            }
            Err(e) => {
                app.vision.set_problem(Some(format!("{e:#}")));
                return;
            }
        }
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap_or_default()
}

/// Ollama answers and has the model; starts `ollama serve` when it is installed but not running.
async fn ready(app: &App, model: &str) -> Result<()> {
    let tags = match client().get(format!("{OLLAMA}/api/tags")).timeout(Duration::from_secs(5)).send().await {
        Ok(r) => r.json::<Value>().await.context("Ollama ответила непонятно")?,
        Err(_) => {
            let Some(ollama) = crate::runner::which("ollama") else {
                bail!("Ollama не установлена (https://ollama.com), а без неё фото не описать");
            };
            let mut st = app.vision.status.lock().unwrap();
            if st.started_ollama.is_none_or(|t| t.elapsed() > START_EVERY) {
                st.started_ollama = Some(Instant::now());
                let mut cmd = std::process::Command::new(ollama);
                cmd.arg("serve").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                if let Some(path) = crate::runner::tool_path() {
                    cmd.env("PATH", path);
                }
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
                }
                let _ = cmd.spawn();
                crate::log!("Поиск по фото: Ollama не отвечала — запускаю ollama serve");
            }
            bail!("Ollama не отвечает (запускается — попробую через минуту)");
        }
    };
    let have = tags["models"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str());
    let wanted = if model.contains(':') { model.to_string() } else { format!("{model}:latest") };
    if !have.into_iter().any(|name| name == wanted) {
        bail!("модель {model} не скачана: ollama pull {model}");
    }
    Ok(())
}

async fn describe(model: &str, jpeg: &[u8]) -> Result<String> {
    let body = json!({
        "model": model,
        "prompt": PROMPT,
        "images": [crate::terminal_store::base64(jpeg)],
        "stream": false,
        "keep_alive": "10m",
        "options": { "temperature": 0.1, "num_predict": 220 },
    });
    let res = client().post(format!("{OLLAMA}/api/generate")).json(&body).timeout(DESCRIBE_TIMEOUT).send().await.context("Ollama не ответила")?;
    let status = res.status();
    let answer: Value = res.json().await.context("Ollama ответила непонятно")?;
    if !status.is_success() {
        bail!("Ollama: {}", answer["error"].as_str().unwrap_or("ошибка"));
    }
    let text = clean(answer["response"].as_str().unwrap_or_default());
    if text.is_empty() {
        bail!("модель вернула пустое описание");
    }
    Ok(text)
}

/// One line, no markdown leftovers, a sane length.
fn clean(raw: &str) -> String {
    let one_line: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    one_line.trim_matches(|c: char| c == '"' || c == '*' || c == '`').chars().take(500).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_one_clean_line() {
        assert_eq!(clean("  **Собака на диване.**\n\n | собака, dog  "), "Собака на диване.** | собака, dog");
        assert_eq!(clean(""), "");
        assert_eq!(clean(&"я".repeat(900)).chars().count(), 500);
    }
}
