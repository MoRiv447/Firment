use anyhow::{Context, Result, bail};
use clap::CommandFactory;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Default install location: `%USERPROFILE%\.firment\bin` (or
/// `$HOME/.firment/bin` on Unix). `FIRMENT_BIN_DIR` overrides it for tests.
pub fn default_bin_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("FIRMENT_BIN_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".firment").join("bin")
}

pub fn exe_name() -> &'static str {
    if cfg!(windows) { "firm.exe" } else { "firm" }
}

pub fn install(to: Option<PathBuf>, files_only: bool) -> Result<()> {
    let source = std::env::current_exe().context("无法确定当前可执行文件路径（current_exe）")?;
    let dir = to.unwrap_or_else(default_bin_dir);
    let (target, completions) = install_files(&source, &dir)?;

    if !files_only {
        match add_user_path(&dir) {
            Ok(true) => println!("已把 {} 加入用户 PATH", dir.display()),
            Ok(false) => println!("用户 PATH 已包含 {}，跳过", dir.display()),
            Err(e) => eprintln!("⚠ 更新用户 PATH 失败: {e:#}"),
        }
        match discover_profile() {
            Some(profile) => match ensure_profile_completion(&profile, &completions) {
                Ok(true) => println!("已把补全注册到 PowerShell profile: {}", profile.display()),
                Ok(false) => println!("PowerShell profile 已包含补全，跳过"),
                Err(e) => eprintln!("⚠ 写入 PowerShell profile 失败: {e:#}"),
            },
            None => eprintln!("⚠ 找不到 PowerShell profile，补全未注册（不影响 firm 本身）"),
        }
    }

    println!(
        "已安装: {}\n补全: {}",
        target.display(),
        completions.display()
    );
    if files_only {
        println!("（files-only：未修改 PATH 与 PowerShell profile）");
    } else {
        println!("请新开一个终端，之后直接输入 firm 即可唤起。");
    }
    Ok(())
}

/// Copy the executable and generate the PowerShell completion file.
/// Exposed separately so tests can run it against a temp directory.
pub fn install_files(source: &Path, dir: &Path) -> Result<(PathBuf, PathBuf)> {
    fs::create_dir_all(dir)?;
    let target = dir.join(exe_name());
    fs::copy(source, &target).with_context(|| {
        format!(
            "复制 {} -> {} 失败（如果目标正在运行，请先退出）",
            source.display(),
            target.display()
        )
    })?;
    let completions = dir.join("firm.completions.ps1");
    let mut file = fs::File::create(&completions)?;
    let mut cmd = crate::Cli::command();
    clap_complete::generate(
        clap_complete::shells::PowerShell,
        &mut cmd,
        "firm",
        &mut file,
    );
    Ok((target, completions))
}

pub fn update(source: Option<PathBuf>, to: Option<PathBuf>) -> Result<()> {
    let source = source.unwrap_or_else(|| {
        std::env::current_exe().expect("无法确定当前可执行文件路径（current_exe）")
    });
    let dir = to.unwrap_or_else(default_bin_dir);
    let target = dir.join(exe_name());
    let current = std::env::current_exe()?;
    update_impl(&source, &target, &current)?;

    let output = std::process::Command::new(&target)
        .arg("--version")
        .output()
        .with_context(|| format!("校验新版本失败（{} 无法执行 --version）", target.display()))?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    println!(
        "已更新: {} -> {}\n新版本: {}",
        source.display(),
        target.display(),
        version
    );
    Ok(())
}

pub fn update_impl(source: &Path, target: &Path, current: &Path) -> Result<()> {
    if same_file(current, target) {
        bail!(
            "当前运行的就是已安装的 firm（{}）。\n请从构建目录运行，例如: .\\target\\release\\firm update",
            target.display()
        );
    }
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("安装目录无效: {}", target.display()))?;
    fs::create_dir_all(parent)?;
    replace_file(source, target)?;
    Ok(())
}

fn replace_file(source: &Path, target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("目标没有父目录: {}", target.display()))?;
    let tmp = parent.join(format!(".firm.update.{}.tmp", std::process::id()));
    fs::copy(source, &tmp).with_context(|| {
        format!(
            "复制更新文件失败: {} -> {}",
            source.display(),
            tmp.display()
        )
    })?;

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let tmp_wide: Vec<u16> = tmp
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let target_wide: Vec<u16> = target
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let ok = unsafe {
            MoveFileExW(
                tmp_wide.as_ptr(),
                target_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if ok == 0 {
            let _ = fs::remove_file(&tmp);
            bail!(
                "替换 {} 失败（错误码 {}；如果目标正在运行，请先退出再更新）",
                target.display(),
                std::io::Error::last_os_error()
            );
        }
    }
    #[cfg(not(windows))]
    {
        fs::rename(&tmp, target)?;
    }
    Ok(())
}

pub fn add_user_path(dir: &Path) -> Result<bool> {
    add_user_path_impl(&RegistryPathEnv, dir)
}

pub trait PathEnv {
    fn read_user_path(&self) -> Result<String>;
    fn write_user_path(&self, value: &str) -> Result<()>;
}

/// Append `dir` to the user PATH once (case-insensitive, `;` separated).
/// Returns true when the entry was added.
pub fn add_user_path_impl(env: &dyn PathEnv, dir: &Path) -> Result<bool> {
    let current = env.read_user_path().unwrap_or_default();
    let needle = normalize_path(dir);
    let mut parts: Vec<String> = current
        .split(';')
        .filter(|p| !p.trim().is_empty())
        .map(|p| p.to_string())
        .collect();
    if parts.iter().any(|p| normalize_path(Path::new(p)) == needle) {
        return Ok(false);
    }
    let dir_str = dir
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_string();
    parts.push(dir_str);
    env.write_user_path(&parts.join(";"))?;
    Ok(true)
}

pub struct RegistryPathEnv;

#[cfg(windows)]
impl PathEnv for RegistryPathEnv {
    fn read_user_path(&self) -> Result<String> {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let env = hkcu.open_subkey_with_flags("Environment", KEY_READ)?;
        Ok(env.get_value("Path").unwrap_or_default())
    }

    fn write_user_path(&self, value: &str) -> Result<()> {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let env = hkcu.open_subkey_with_flags("Environment", KEY_WRITE)?;
        env.set_value("Path", &value)?;
        broadcast_environment_change();
        Ok(())
    }
}

#[cfg(not(windows))]
impl PathEnv for RegistryPathEnv {
    fn read_user_path(&self) -> Result<String> {
        Ok(String::new())
    }

    fn write_user_path(&self, _value: &str) -> Result<()> {
        broadcast_environment_change();
        Ok(())
    }
}

/// Does the effective PATH (process PATH + user registry PATH) contain `dir`?
pub fn user_path_contains(dir: &Path) -> bool {
    let needle = normalize_path(dir);
    let mut sources = vec![std::env::var("PATH").unwrap_or_default()];
    if let Ok(user) = RegistryPathEnv.read_user_path() {
        sources.push(user);
    }
    sources.iter().any(|path| {
        path.split(';')
            .any(|p| normalize_path(Path::new(p)) == needle)
    })
}

#[cfg(windows)]
fn broadcast_environment_change() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };
    let data: Vec<u16> = "Environment\0".encode_utf16().collect();
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            data.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            std::ptr::null_mut(),
        );
    }
}

#[cfg(not(windows))]
fn broadcast_environment_change() {}

fn discover_profile() -> Option<PathBuf> {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Write-Output $PROFILE",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let path = text.trim();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

fn ensure_profile_completion(profile: &Path, completions: &Path) -> Result<bool> {
    let line = format!(". \"{}\"", completions.display());
    if profile.exists() {
        let content = fs::read_to_string(profile)?;
        if content.lines().any(|l| l.trim() == line) {
            return Ok(false);
        }
    }
    if let Some(parent) = profile.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(profile)?;
    writeln!(file, "{line}")?;
    Ok(true)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn normalize_path(path: &Path) -> String {
    let s = path
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_string();
    if cfg!(windows) { s.to_lowercase() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct MemoryPathEnv {
        value: Arc<Mutex<String>>,
    }

    impl PathEnv for MemoryPathEnv {
        fn read_user_path(&self) -> Result<String> {
            Ok(self.value.lock().unwrap().clone())
        }

        fn write_user_path(&self, value: &str) -> Result<()> {
            *self.value.lock().unwrap() = value.to_string();
            Ok(())
        }
    }

    #[test]
    fn path_append_is_case_insensitive_and_deduplicated() {
        let dir = PathBuf::from(r"C:\Users\me\.firment\bin");
        let env = MemoryPathEnv {
            value: Arc::new(Mutex::new(
                r"C:\Windows;C:\Users\ME\.FIRMENT\BIN\".to_string(),
            )),
        };
        assert!(!add_user_path_impl(&env, &dir).unwrap());
        assert_eq!(
            *env.value.lock().unwrap(),
            r"C:\Windows;C:\Users\ME\.FIRMENT\BIN\"
        );

        let env = MemoryPathEnv {
            value: Arc::new(Mutex::new("C:\\Windows".to_string())),
        };
        assert!(add_user_path_impl(&env, &dir).unwrap());
        assert_eq!(
            *env.value.lock().unwrap(),
            format!(r"C:\Windows;{}", dir.display())
        );
    }

    #[test]
    fn install_files_copies_exe_and_writes_completions() {
        let dir = tempfile::tempdir().unwrap();
        let source = std::env::current_exe().unwrap();
        let (target, completions) = install_files(&source, dir.path()).unwrap();
        assert!(target.is_file());
        assert!(completions.is_file());
        let content = fs::read_to_string(&completions).unwrap();
        assert!(content.contains("firm"));
    }

    #[test]
    fn update_replaces_target_and_rejects_running_self() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(exe_name());
        fs::write(&target, b"old").unwrap();
        let source = dir.path().join("new.exe");
        fs::write(&source, b"new-bytes").unwrap();

        update_impl(&source, &target, &dir.path().join("other.exe")).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new-bytes");

        let err = update_impl(&source, &target, &target).unwrap_err();
        assert!(err.to_string().contains("当前运行的就是已安装"));
    }

    #[test]
    fn profile_line_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("Microsoft.PowerShell_profile.ps1");
        let completions = dir.path().join("firm.completions.ps1");
        assert!(ensure_profile_completion(&profile, &completions).unwrap());
        assert!(!ensure_profile_completion(&profile, &completions).unwrap());
        let content = fs::read_to_string(&profile).unwrap();
        assert_eq!(content.lines().count(), 1);
    }
}
