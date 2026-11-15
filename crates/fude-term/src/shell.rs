//! ShellKind から、PTY で起動するコマンドを組み立てる。

use std::path::PathBuf;

use fude_core::settings::NativeShell;
use fude_core::workspace::{ProjectRoot, ShellKind};

/// PTY で起動するコマンド。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// 親の環境に足す変数。
    pub env: Vec<(String, String)>,
}

/// WSL に引き継ぐ変数（/u は Windows から WSL への向きだけ）。
const WSLENV_ADD: &str = "COLORTERM/u:TERM_PROGRAM/u:FUDE_TERMINAL_ID/u";

/// シェルの種類とプロジェクトの場所から、起動するコマンドを組み立てる。作業ディレクトリは root.host_path()。
/// WSL のシェルの --cd は、ローカルのプロジェクトなら Windows のパス、同じディストリビューションの
/// プロジェクトなら Linux のパス、別のディストリビューションならホーム（~）。
/// terminal_id は FUDE_TERMINAL_ID として渡す（hook と、WSL の稼働の確認がターミナルを見分けるため。設計書 §5.8）。
/// native は設定の terminal.shell（ShellKind::Native のときに使う）。WSL のディストリビューションが空なら -d を付けない
/// （wsl.exe の既定のディストリビューション）。env は環境変数の読み取り（テストで差し替える）。
pub fn spawn_spec(
    kind: &ShellKind,
    root: &ProjectRoot,
    terminal_id: &str,
    native: &NativeShell,
    env: &dyn Fn(&str) -> Option<String>,
) -> SpawnSpec {
    let mut vars = vec![
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
        ("TERM_PROGRAM".to_string(), "fude".to_string()),
        ("FUDE_TERMINAL_ID".to_string(), terminal_id.to_string()),
    ];
    let (program, args) = match kind {
        ShellKind::Wsl { distro } => {
            let mut add = WSLENV_ADD.to_string();
            // 開発中のインスタンス（FUDE_INSTANCE）を、WSL の hook が送り先を選ぶために渡す。
            if env("FUDE_INSTANCE").is_some_and(|v| !v.is_empty()) {
                add.push_str(":FUDE_INSTANCE/u");
            }
            let wslenv = match env("WSLENV").filter(|v| !v.is_empty()) {
                Some(existing) => format!("{existing}:{add}"),
                None => add,
            };
            vars.push(("WSLENV".into(), wslenv));
            let cd = match root {
                ProjectRoot::Local(path) => path.display().to_string(),
                ProjectRoot::Wsl { distro: d, path } if d == distro => path.clone(),
                ProjectRoot::Wsl { .. } => "~".to_string(),
            };
            let mut args = if distro.is_empty() { Vec::new() } else { vec!["-d".into(), distro.clone()] };
            args.extend(["--cd".into(), cd]);
            ("wsl.exe".to_string(), args)
        }
        ShellKind::Native if cfg!(windows) => match native {
            NativeShell::Pwsh => ("pwsh.exe".to_string(), vec!["-NoLogo".to_string()]),
            NativeShell::Powershell => ("powershell.exe".to_string(), vec!["-NoLogo".to_string()]),
            NativeShell::Cmd => ("cmd.exe".to_string(), Vec::new()),
            NativeShell::Program(p) => (p.clone(), Vec::new()),
            NativeShell::Auto | NativeShell::Wsl => {
                let program = if find_in_path("pwsh.exe", env) { "pwsh.exe" } else { "powershell.exe" };
                (program.to_string(), vec!["-NoLogo".to_string()])
            }
        },
        ShellKind::Native => {
            // Finder から起動すると LANG が無く、日本語が崩れるため、文字の種類だけ UTF-8 にする。
            if ["LANG", "LC_ALL", "LC_CTYPE"].iter().all(|k| env(k).is_none()) {
                vars.push(("LC_CTYPE".into(), "UTF-8".into()));
            }
            let program = match native {
                NativeShell::Program(p) => p.clone(),
                NativeShell::Pwsh => "pwsh".to_string(),
                _ => env("SHELL").filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into()),
            };
            (program, vec!["-l".to_string()])
        }
    };
    SpawnSpec { program, args, cwd: root.host_path(), env: vars }
}

/// PATH のどこかに name のファイルがあるか。
fn find_in_path(name: &str, env: &dyn Fn(&str) -> Option<String>) -> bool {
    env("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use fude_core::workspace::ProjectRoot;
    use std::path::Path;

    fn spec_with(kind: &ShellKind, root: &str, vars: &[(&str, &str)]) -> SpawnSpec {
        spec_in(kind, &ProjectRoot::Local(PathBuf::from(root)), vars)
    }

    fn spec_in(kind: &ShellKind, root: &ProjectRoot, vars: &[(&str, &str)]) -> SpawnSpec {
        let map: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        spawn_spec(kind, root, "p1", &NativeShell::Auto, &|k| map.get(k).cloned())
    }

    fn wsl_root() -> ProjectRoot {
        ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/x".into() }
    }

    #[test]
    fn wsl_project_wsl_shell_cds_linux_path() {
        let spec = spec_in(&ShellKind::Wsl { distro: "Ubuntu".into() }, &wsl_root(), &[]);
        assert_eq!(spec.args, vec!["-d", "Ubuntu", "--cd", "/home/x"]);
    }

    #[test]
    fn wsl_project_other_distro_cds_home() {
        let spec = spec_in(&ShellKind::Wsl { distro: "Debian".into() }, &wsl_root(), &[]);
        assert_eq!(spec.args, vec!["-d", "Debian", "--cd", "~"]);
    }

    #[cfg(windows)]
    #[test]
    fn wsl_project_native_shell_cwd_is_unc() {
        let spec = spec_in(&ShellKind::Native, &wsl_root(), &[]);
        assert_eq!(spec.cwd, Path::new(r"\\wsl.localhost\Ubuntu\home\x"));
    }

    fn env<'a>(spec: &'a SpawnSpec, key: &str) -> Option<&'a str> {
        spec.env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn instance_is_forwarded_to_wsl() {
        let spec = spec_with(&ShellKind::Wsl { distro: "Ubuntu".into() }, r"C:\w\a", &[("FUDE_INSTANCE", "dev")]);
        assert_eq!(env(&spec, "WSLENV"), Some("COLORTERM/u:TERM_PROGRAM/u:FUDE_TERMINAL_ID/u:FUDE_INSTANCE/u"));
        let spec = spec_with(&ShellKind::Wsl { distro: "Ubuntu".into() }, r"C:\w\a", &[]);
        assert_eq!(env(&spec, "WSLENV"), Some("COLORTERM/u:TERM_PROGRAM/u:FUDE_TERMINAL_ID/u"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_mac_uses_shell_env_and_ctype() {
        let spec = spec_with(&ShellKind::Native, "/w/a", &[("SHELL", "/bin/bash")]);
        assert_eq!(spec.program, "/bin/bash");
        assert_eq!(spec.args, vec!["-l".to_string()]);
        assert_eq!(env(&spec, "LC_CTYPE"), Some("UTF-8"));
        let spec = spec_with(&ShellKind::Native, "/w/a", &[("LANG", "ja_JP.UTF-8")]);
        assert_eq!(spec.program, "/bin/zsh");
        assert_eq!(env(&spec, "LC_CTYPE"), None);
    }

    #[test]
    fn wsl_spec_uses_distro_and_cd() {
        let spec = spec_with(&ShellKind::Wsl { distro: "Ubuntu".into() }, r"C:\w\a", &[]);
        assert_eq!(spec.program, "wsl.exe");
        assert_eq!(spec.args, vec!["-d", "Ubuntu", "--cd", r"C:\w\a"]);
        assert_eq!(env(&spec, "WSLENV"), Some("COLORTERM/u:TERM_PROGRAM/u:FUDE_TERMINAL_ID/u"));
    }

    #[test]
    fn wslenv_is_appended() {
        let spec = spec_with(&ShellKind::Wsl { distro: "Ubuntu".into() }, r"C:\w\a", &[("WSLENV", "FOO/p")]);
        assert_eq!(env(&spec, "WSLENV"), Some("FOO/p:COLORTERM/u:TERM_PROGRAM/u:FUDE_TERMINAL_ID/u"));
    }

    #[test]
    fn terminal_id_is_exported() {
        let spec = spec_with(&ShellKind::Native, "/w/a", &[]);
        assert_eq!(env(&spec, "FUDE_TERMINAL_ID"), Some("p1"));
        let spec = spec_with(&ShellKind::Wsl { distro: "Ubuntu".into() }, "/w/a", &[]);
        assert_eq!(env(&spec, "FUDE_TERMINAL_ID"), Some("p1"));
    }

    #[test]
    fn common_env_is_set() {
        let spec = spec_with(&ShellKind::Native, "/w/a", &[]);
        assert_eq!(env(&spec, "TERM"), Some("xterm-256color"));
        assert_eq!(env(&spec, "COLORTERM"), Some("truecolor"));
        assert_eq!(env(&spec, "TERM_PROGRAM"), Some("fude"));
        assert_eq!(spec.cwd, Path::new("/w/a"));
    }

    #[test]
    fn native_shell_choice() {
        use fude_core::settings::NativeShell;
        let root = ProjectRoot::Local(PathBuf::from("/w"));
        let spec = |native: &NativeShell, vars: &[(&str, &str)]| {
            let map: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            spawn_spec(&ShellKind::Native, &root, "p1", native, &|k| map.get(k).cloned())
        };
        assert_eq!(spec(&NativeShell::Program("/bin/fish".into()), &[]).program, "/bin/fish");
        if cfg!(windows) {
            assert_eq!(spec(&NativeShell::Cmd, &[]).program, "cmd.exe");
            assert_eq!(spec(&NativeShell::Pwsh, &[]).program, "pwsh.exe");
            assert_eq!(spec(&NativeShell::Powershell, &[]).program, "powershell.exe");
            assert_eq!(spec(&NativeShell::Auto, &[("PATH", "")]).program, "powershell.exe");
        } else {
            assert_eq!(spec(&NativeShell::Auto, &[("SHELL", "/bin/bash")]).program, "/bin/bash");
        }
    }

    #[test]
    fn wsl_without_distro_omits_d() {
        let spec = spec_with(&ShellKind::Wsl { distro: String::new() }, "/w", &[]);
        assert!(!spec.args.iter().any(|a| a == "-d"), "{:?}", spec.args);
        assert_eq!(spec.program, "wsl.exe");
    }
}
