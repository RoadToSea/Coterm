//! Shell 家族识别与可执行文件解析。
//!
//! 本地终端不能假定用户装的是 POSIX Shell：Windows 默认是 PowerShell，命令交付方式、
//! 语法检查入口和回显行为都不一样。这里只做「这段配置对应哪个家族、要启动哪个可执行
//! 文件」的判断，真正的启动与 IO 在 [`crate::local::session`]。
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellFamily {
    Posix,
    PowerShell,
    Cmd,
    /// 能当终端用，但无法可靠地包裹命令；`exec` 必须拒绝而不是猜测。
    Unknown,
}

impl ShellFamily {
    /// 按可执行文件名判断家族，忽略目录与 Windows 扩展名。
    pub fn detect(shell: &Path) -> Self {
        let Some(name) = shell.file_stem().and_then(|name| name.to_str()) else {
            return Self::Unknown;
        };
        match name.to_ascii_lowercase().as_str() {
            "sh" | "bash" | "zsh" | "dash" | "ash" | "ksh" | "mksh" => Self::Posix,
            "pwsh" | "powershell" => Self::PowerShell,
            "cmd" => Self::Cmd,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Posix => "posix",
            Self::PowerShell => "powershell",
            Self::Cmd => "cmd",
            Self::Unknown => "unknown",
        }
    }
}

/// 把配置里的 `shell` 解析成实际可执行文件。
///
/// 带分隔符的按路径校验存在性，不带分隔符的按 PATH 查找；找不到就失败，绝不退回
/// 「随便挑一个 shell」——启动哪个解释器必须对用户可见。
pub fn resolve_executable(shell: &str) -> Option<PathBuf> {
    let shell = shell.trim();
    if shell.is_empty() {
        return None;
    }
    let candidate = Path::new(shell);
    if shell.contains(['/', '\\']) {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let direct = directory.join(shell);
        if direct.is_file() {
            return Some(direct);
        }
        if cfg!(windows) && candidate.extension().is_none() {
            for extension in ["exe", "cmd", "bat"] {
                let with_extension = directory.join(format!("{shell}.{extension}"));
                if with_extension.is_file() {
                    return Some(with_extension);
                }
            }
        }
    }
    None
}

/// 用户没有指定 Shell 时的探测顺序：先看用户偏好，再退回系统自带的解释器。
pub fn detect_default() -> Option<PathBuf> {
    if cfg!(windows) {
        let candidates: Vec<String> =
            ["pwsh.exe", "powershell.exe"]
                .iter()
                .map(|name| (*name).to_owned())
                .chain(std::env::var("SystemRoot").ok().map(|root| {
                    format!("{root}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe")
                }))
                .chain(std::env::var("COMSPEC").ok())
                .collect();
        for candidate in candidates {
            if let Some(path) = resolve_executable(&candidate) {
                return Some(path);
            }
        }
        return None;
    }
    let configured = std::env::var("SHELL").unwrap_or_default();
    let configured = Path::new(configured.trim());
    if configured.is_absolute() && configured.is_file() {
        return Some(configured.to_path_buf());
    }
    ["/bin/bash", "/bin/sh"]
        .iter()
        .map(Path::new)
        .find(|path| path.is_file())
        .map(Path::to_path_buf)
}

/// 解析用户 home。仅用于 `cwd` 留空时的起点，不参与任何路径约束。
pub fn home_directory() -> Option<PathBuf> {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

/// 解析失败的原因。带上类型，基座才能给出稳定错误码而不是匹配提示文字。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveProblem {
    ShellNotFound,
    CwdUnavailable,
}

/// 解析后的启动目标，避免每次操作都重复查 PATH。
#[derive(Clone, Debug)]
pub struct ResolvedShell {
    pub path: PathBuf,
    pub family: ShellFamily,
    pub cwd: PathBuf,
}

impl ResolvedShell {
    /// `cwd` 留空时使用 home；两者都不可用时返回错误而不是在未知目录里启动 Shell。
    pub fn resolve(
        shell: &str,
        cwd: &str,
        family: Option<ShellFamily>,
    ) -> std::result::Result<Self, ResolveProblem> {
        let path = if shell.trim().is_empty() {
            detect_default().ok_or(ResolveProblem::ShellNotFound)?
        } else {
            resolve_executable(shell).ok_or(ResolveProblem::ShellNotFound)?
        };
        let cwd = if cwd.trim().is_empty() {
            home_directory().ok_or(ResolveProblem::CwdUnavailable)?
        } else {
            PathBuf::from(cwd.trim())
        };
        if !cwd.is_dir() {
            return Err(ResolveProblem::CwdUnavailable);
        }
        Ok(Self {
            family: family.unwrap_or_else(|| ShellFamily::detect(&path)),
            path,
            cwd,
        })
    }

    /// 从已解析 Shell 生成默认名称 String；无额外参数且不执行 IO，不返回错误。
    pub fn default_name(&self) -> String {
        let name = self
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned());
        format!("{name} @ {}", self.cwd.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_follow_the_executable_name() {
        let cases = [
            ("/bin/bash", ShellFamily::Posix),
            (
                "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
                ShellFamily::PowerShell,
            ),
            ("pwsh", ShellFamily::PowerShell),
            ("cmd.exe", ShellFamily::Cmd),
            ("/usr/bin/fish", ShellFamily::Unknown),
        ];
        for (path, family) in cases {
            assert_eq!(ShellFamily::detect(Path::new(path)), family, "{path}");
        }
    }

    #[test]
    fn blank_and_missing_shells_do_not_resolve_to_something_arbitrary() {
        assert!(resolve_executable("   ").is_none());
        assert!(resolve_executable("/definitely/not/a/shell").is_none());
        assert!(
            ResolvedShell::resolve("", "", None).is_ok(),
            "a default shell should exist"
        );
    }

    #[test]
    fn labels_show_the_file_name_and_directory() {
        let resolved = ResolvedShell {
            path: PathBuf::from("/bin/bash"),
            family: ShellFamily::Posix,
            cwd: PathBuf::from("/srv"),
        };
        assert_eq!(resolved.default_name(), "bash @ /srv");
    }

    #[test]
    fn existing_directory_is_required() {
        let directory = std::env::temp_dir();
        assert!(ResolvedShell::resolve("", directory.to_str().unwrap_or_default(), None).is_ok());
        assert!(ResolvedShell::resolve("", "/definitely/not/a/directory", None).is_err());
    }
}
