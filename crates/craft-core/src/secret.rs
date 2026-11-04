//! 密钥存储：~/.craftwork/keys.json（0600）。
//! 密钥只存在于主进程与该文件；损坏时降级启动（新空存储）并给出警告，由用户在设置页重填。
//!
//! 对应 src/main/core/secret.ts。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::platform::write_file_atomic_str;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeysFile {
    version: i32,
    providers: HashMap<String, String>,
}

impl Default for KeysFile {
    fn default() -> Self {
        Self {
            version: 1,
            providers: HashMap::new(),
        }
    }
}

pub struct SecretStore {
    file_path: PathBuf,
    data: KeysFile,
    warnings: Vec<String>,
}

impl SecretStore {
    fn new(file_path: PathBuf) -> Self {
        Self {
            file_path,
            data: KeysFile::default(),
            warnings: Vec::new(),
        }
    }

    fn ensure_dir(dir: &Path) {
        let _ = fs::create_dir_all(dir);
        set_dir_mode_0700(dir);
    }

    pub fn open(dir: &Path) -> SecretStore {
        let file_path = dir.join("keys.json");
        let mut store = SecretStore::new(file_path.clone());
        Self::ensure_dir(dir);
        if !file_path.exists() {
            return store;
        }
        // 权限过宽时自动收紧
        if !tighten_mode(&file_path) {
            store.warnings.push("keys.json 权限过宽，自动收紧 0600 失败。".into());
        }
        match fs::read_to_string(&file_path).map_err(|e| e.to_string()).and_then(|text| {
            serde_json::from_str::<KeysFile>(&text).map_err(|e| e.to_string())
        }) {
            Ok(parsed) => store.data = parsed,
            Err(_) => {
                store
                    .warnings
                    .push("keys.json 读取失败（可能已损坏），已按空密钥库启动；请重新配置密钥。".into());
            }
        }
        store
    }

    /// 损坏降级：空存储（之后 set 会覆盖原文件）。
    pub fn new_empty(dir: &Path) -> SecretStore {
        Self::ensure_dir(dir);
        SecretStore::new(dir.join("keys.json"))
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn get(&self, provider_id: &str) -> String {
        self.data.providers.get(provider_id).cloned().unwrap_or_default()
    }

    /// 取密钥：文件值优先（空串视为未配置），再按 envNames 逐个回退环境变量。
    pub fn get_for(&self, provider_id: &str, env_names: &[&str]) -> String {
        let file = self.get(provider_id);
        if !file.is_empty() {
            return file;
        }
        for name in env_names {
            let Ok(v) = std::env::var(name) else { continue };
            if !v.is_empty() {
                return v;
            }
        }
        String::new()
    }

    pub fn source(&self, provider_id: &str) -> String {
        if self.get(provider_id).is_empty() {
            String::new()
        } else {
            "file".into()
        }
    }

    pub fn source_for(&self, provider_id: &str, env_names: &[&str]) -> String {
        if !self.get(provider_id).is_empty() {
            return "file".into();
        }
        for name in env_names {
            if std::env::var(name).is_ok_and(|v| !v.is_empty()) {
                return "env".into();
            }
        }
        String::new()
    }

    /// 遮罩展示：≤10 位全点；否则前 5 位…后 4 位。
    pub fn masked_for(&self, provider_id: &str, env_names: &[&str]) -> String {
        let key = self.get_for(provider_id, env_names);
        if key.is_empty() {
            return String::new();
        }
        let chars: Vec<char> = key.chars().collect();
        if chars.len() <= 10 {
            return "••••••".into();
        }
        format!("{}…{}", chars[..5].iter().collect::<String>(), &key[key.len() - 4..])
    }

    pub fn set(&mut self, provider_id: &str, value: &str) {
        if value.is_empty() {
            self.data.providers.remove(provider_id);
        } else {
            self.data.providers.insert(provider_id.to_string(), value.to_string());
        }
        self.persist();
    }

    pub fn delete(&mut self, provider_id: &str) {
        self.data.providers.remove(provider_id);
        self.persist();
    }

    fn persist(&self) {
        // 同目录临时文件 → 0600 → 原子替换
        let text = serde_json::to_string_pretty(&self.data).unwrap_or_default();
        if let Err(err) = write_file_atomic_str(&self.file_path, &format!("{text}\n"), 0o600) {
            eprintln!("[craftwork] keys.json 写入失败：{err}");
        }
    }
}

#[cfg(unix)]
fn current_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).ok().map(|m| m.permissions().mode())
}

#[cfg(not(unix))]
fn current_mode(_path: &Path) -> Option<u32> {
    Some(0o600)
}

/// 权限过宽时收紧 0600；返回是否处于收紧状态（含已收紧成功）。
#[cfg(unix)]
fn tighten_mode(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match current_mode(path) {
        Some(mode) if mode & 0o077 != 0 => {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
            current_mode(path).is_some_and(|m| m & 0o077 == 0)
        }
        Some(_) => true,
        None => false,
    }
}

#[cfg(not(unix))]
fn tighten_mode(_path: &Path) -> bool {
    true
}

#[cfg(unix)]
fn set_dir_mode_0700(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_dir_mode_0700(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "craftwork-secret-{tag}-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 写入读取_文件权限收紧_遮罩() {
        let dir = tmp_dir("basic");
        {
            let mut store = SecretStore::open(&dir);
            store.set("p1", "sk-abcdefghijklmnop");
            assert_eq!(store.get("p1"), "sk-abcdefghijklmnop");
            assert_eq!(store.source("p1"), "file");
            assert_eq!(store.masked_for("p1", &[]), "sk-ab…mnop");
            assert_eq!(store.masked_for("p2", &[]), "");
            assert_eq!(store.get_for("p2", &["CRAFTWORK_TEST_ENV_KEY"]), "");
        }
        // 重新打开：值持久化、权限 0600
        let store = SecretStore::open(&dir);
        assert_eq!(store.get("p1"), "sk-abcdefghijklmnop");
        #[cfg(unix)]
        {
            let mode = current_mode(&dir.join("keys.json")).unwrap_or(0);
            assert_eq!(mode & 0o777, 0o600);
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 空值set即删除_env回退链() {
        unsafe { std::env::set_var("CRAFTWORK_SECRET_TEST_KEY", "env-value") };
        let dir = tmp_dir("env");
        let mut store = SecretStore::open(&dir);
        store.set("p1", "x");
        store.set("p1", "");
        assert_eq!(store.get("p1"), "");
        assert_eq!(store.get_for("p1", &["CRAFTWORK_SECRET_TEST_KEY"]), "env-value");
        assert_eq!(store.source_for("p1", &["CRAFTWORK_SECRET_TEST_KEY"]), "env");
        unsafe { std::env::remove_var("CRAFTWORK_SECRET_TEST_KEY") };
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 损坏文件降级并给出警告() {
        let dir = tmp_dir("corrupt");
        fs::write(dir.join("keys.json"), "{not json").unwrap();
        let store = SecretStore::open(&dir);
        assert!(store.get("p1").is_empty());
        assert!(store.warnings().iter().any(|w| w.contains("读取失败")));
        // set 覆盖原文件恢复可用
        let mut store = store;
        store.set("p1", "ok");
        assert_eq!(SecretStore::open(&dir).get("p1"), "ok");
        fs::remove_dir_all(&dir).ok();
    }
}
