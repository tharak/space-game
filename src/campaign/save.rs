//! Saving and loading the campaign: JSON in `localStorage` on the web, a
//! file next to the game natively.

use super::state::Campaign;

const KEY: &str = "successor-stars-campaign-v1";

pub fn to_json(c: &Campaign) -> String {
    serde_json::to_string(c).expect("campaign serializes")
}

pub fn from_json(s: &str) -> Option<Campaign> {
    serde_json::from_str(s).ok()
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

#[cfg(not(target_arch = "wasm32"))]
fn path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("successor-stars").join(format!("{KEY}.json"))
}

pub fn save(c: &Campaign) {
    let json = to_json(c);
    #[cfg(target_arch = "wasm32")]
    if let Some(s) = storage() {
        let _ = s.set_item(KEY, &json);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, json);
    }
}

pub fn load() -> Option<Campaign> {
    #[cfg(target_arch = "wasm32")]
    {
        storage()?.get_item(KEY).ok()?.and_then(|s| from_json(&s))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_to_string(path()).ok().and_then(|s| from_json(&s))
    }
}

pub fn clear() {
    #[cfg(target_arch = "wasm32")]
    if let Some(s) = storage() {
        let _ = s.remove_item(KEY);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = std::fs::remove_file(path());
    }
}
