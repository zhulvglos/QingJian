//! 只复用本机基础工作区的完整语音资源，不读取其他账号配置或复制录音。
use std::path::{Path, PathBuf};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};

const FIELDS: [(&str, &str); 3] = [
    ("sensePython", "sensevoice/python/python.exe"),
    ("senseRuntime", "sensevoice/runtime"),
    ("senseModels", "sensevoice/models"),
];

fn on_d_drive(config: &Value) -> bool {
    FIELDS.iter().all(|(key, _)| config[*key].as_str().is_some_and(|s| {
        let path = Path::new(s);
        path.is_absolute() && s.to_ascii_lowercase().starts_with("d:\\")
    }))
}

pub fn detect_and_reuse(config: &mut Value, base: &Path) -> bool {
    if crate::sensevoice::available(config) { return false; }
    let current_root = PathBuf::from(config["root"].as_str().unwrap_or(""));
    // 自定义路径即使失效也不能静默覆盖；只修复首次进入账号后的默认空路径。
    if FIELDS.iter().any(|(key, relative)| Path::new(config[*key].as_str().unwrap_or("")) != current_root.join(relative)) {
        return false;
    }
    let database = base.join("qingjian-v1.sqlite3");
    let saved = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()
        .and_then(|c| c.query_row("SELECT value FROM app_settings WHERE key='audio_config'", [], |r| r.get::<_, String>(0)).ok())
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let mut conventional = json!({"root": base.join("voice")});
    crate::sensevoice::defaults(&mut conventional);
    for mut candidate in saved.into_iter().chain(std::iter::once(conventional)) {
        crate::sensevoice::defaults(&mut candidate);
        if !on_d_drive(&candidate) || !crate::sensevoice::available(&candidate) { continue; }
        // 仅保存三项本机资源引用；账号工作目录、录音、密钥和校验结果都不复制。
        for (key, _) in FIELDS { config[key] = candidate[key].clone(); }
        config["reusedLocalResources"] = json!(true);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, Value, Value) {
        let base = PathBuf::from(std::env::var_os("TEMP").unwrap()).join(format!("voice-reuse-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let mut original = json!({"root": base.join("voice")});
        crate::sensevoice::defaults(&mut original);
        for path in crate::sensevoice::files(&original) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"synthetic fixture, not real model").unwrap();
        }
        let mut account = json!({"root": base.join("accounts/a/voice")});
        crate::sensevoice::defaults(&mut account);
        (base, original, account)
    }
    #[test] fn reuses_complete_resources_without_changing_account_root_or_copying_settings() {
        let (base, mut original, mut account) = fixture();
        original["unrelatedSetting"] = json!("must not copy");
        let c=Connection::open(base.join("qingjian-v1.sqlite3")).unwrap();
        c.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT)").unwrap();
        c.execute("INSERT INTO app_settings VALUES('audio_config',?1)", [original.to_string()]).unwrap(); drop(c);
        let root=account["root"].clone();
        assert!(detect_and_reuse(&mut account,&base));
        assert_eq!(account["root"],root); assert!(account.get("unrelatedSetting").is_none());
        assert!(crate::sensevoice::available(&account));
        assert!(!detect_and_reuse(&mut account,&base));
        assert!(!Path::new(root.as_str().unwrap()).join("sensevoice").exists());
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test] fn missing_resource_never_reports_available_or_reuses_partial_installation() {
        let (base,original,mut account)=fixture();
        std::fs::remove_file(&crate::sensevoice::files(&original)[0]).unwrap();
        let before=account.clone(); assert!(!detect_and_reuse(&mut account,&base));assert_eq!(before,account);
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test] fn explicit_custom_paths_are_never_overwritten() {
        let (base,_,mut account)=fixture(); account["senseModels"]=json!(base.join("custom-models"));
        let before=account.clone();assert!(!detect_and_reuse(&mut account,&base));assert_eq!(before,account);
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test] fn conventional_installation_is_found_without_reading_other_accounts() {
        let (base,original,mut account)=fixture();assert!(detect_and_reuse(&mut account,&base));
        assert_eq!(account["sensePython"],original["sensePython"]);
        std::fs::remove_dir_all(base.join("voice")).unwrap();
        let mut other=json!({"root":base.join("accounts/b/voice")});crate::sensevoice::defaults(&mut other);
        assert!(!detect_and_reuse(&mut other,&base));
        std::fs::remove_dir_all(base).unwrap();
    }
}
