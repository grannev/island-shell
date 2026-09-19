//! User applications must outlive the island's service and its cgroup.
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

fn executable(program: &std::ffi::OsStr) -> bool {
    let runnable = |path: &Path| path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if Path::new(program).components().count() > 1 {
        return runnable(Path::new(program));
    }
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| runnable(&dir.join(program))))
}

fn scope_command(command: &Command) -> Command {
    let mut scoped = Command::new("systemd-run");
    // A new scope, not merely setsid: KillMode=control-group also kills
    // detached sessions in the island's cgroup. Preserve the GUI environment
    // and literal '$' in filenames/desktop commands; never use shell quoting.
    scoped.args(["--user", "--scope", "--collect", "--quiet", "--expand-environment=no", "--"]);
    scoped.arg(command.get_program()).args(command.get_args());
    if let Some(dir) = command.get_current_dir() {
        scoped.current_dir(dir);
    }
    for (name, value) in command.get_envs() {
        match value {
            Some(value) => {
                scoped.env(name, value);
            }
            None => {
                scoped.env_remove(name);
            }
        }
    }
    scoped.process_group(0).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::inherit());
    scoped
}

/// Call on a background thread. The returned child is reaped by the caller.
/// Fail closed if scopes are unavailable; falling back to direct spawn would
/// silently reintroduce loss of the user's terminal/editor on service restart.
pub fn spawn(command: &Command) -> io::Result<Child> {
    if !executable(command.get_program()) {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("{:?} not installed", command.get_program())));
    }
    scope_command(command).spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_preserves_literal_arguments_environment_and_directory() {
        let mut original = Command::new("alacritty");
        original
            .args(["-e", "nvim", "--", "/tmp/$HOME; 'notes'.txt"])
            .env("ISLAND_TEST", "literal $VALUE")
            .env_remove("ISLAND_REMOVED")
            .current_dir("/tmp");
        let scoped = scope_command(&original);
        assert_eq!(scoped.get_program(), "systemd-run");
        assert_eq!(
            scoped.get_args().collect::<Vec<_>>(),
            [
                "--user",
                "--scope",
                "--collect",
                "--quiet",
                "--expand-environment=no",
                "--",
                "alacritty",
                "-e",
                "nvim",
                "--",
                "/tmp/$HOME; 'notes'.txt",
            ]
            .map(std::ffi::OsStr::new)
        );
        assert_eq!(scoped.get_current_dir(), Some(Path::new("/tmp")));
        assert_eq!(scoped.get_envs().collect::<Vec<_>>(), original.get_envs().collect::<Vec<_>>());
    }

    #[test]
    fn missing_program_is_reported_before_starting_a_scope() {
        assert_eq!(
            spawn(&Command::new("/nonexistent/island-test-opener")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    #[ignore = "Helper for the isolated service lifetime test"]
    fn scope_parent_fixture() {
        let Some(root) = std::env::var_os("ISLAND_SCOPE_TEST_ROOT") else { return };
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "printf '%s' \"$$\" > \"$1/pid\"; cat /proc/self/cgroup > \"$1/cgroup\"; exec sleep 60",
                "fixture",
            ])
            .arg(root);
        let mut child = spawn(&command).unwrap();
        let _ = child.wait();
    }

    #[test]
    #[ignore = "Creates a private user service and scope; stops only those test units"]
    fn scoped_process_survives_parent_service_stop() {
        let unique = format!(
            "island-lifetime-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        );
        let root = std::env::temp_dir().join(&unique);
        std::fs::create_dir(&root).unwrap();
        let parent = format!("{unique}.service");
        struct Cleanup {
            root: std::path::PathBuf,
            parent: String,
            scope: Option<String>,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for unit in std::iter::once(&self.parent).chain(self.scope.iter()) {
                    let _ = Command::new("systemctl").args(["--user", "stop", unit]).status();
                }
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }
        let mut cleanup = Cleanup { root: root.clone(), parent: parent.clone(), scope: None };
        assert!(
            Command::new("systemd-run")
                .args(["--user", "--collect", "--quiet", "--service-type=exec", "--unit"])
                .arg(&parent)
                .arg(format!("--setenv=ISLAND_SCOPE_TEST_ROOT={}", root.display()))
                .arg(std::env::current_exe().unwrap())
                .args(["--exact", "launch::tests::scope_parent_fixture", "--ignored", "--nocapture"])
                .status()
                .unwrap()
                .success()
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let cgroup = loop {
            if let Ok(cgroup) = std::fs::read_to_string(root.join("cgroup")) {
                if cgroup.contains(".scope") {
                    break cgroup;
                }
            }
            assert!(std::time::Instant::now() < deadline, "scope did not start");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        let scope = cgroup.trim().rsplit('/').next().unwrap().to_string();
        assert!(scope.starts_with("run-") && scope.ends_with(".scope"));
        cleanup.scope = Some(scope);
        assert!(!cgroup.contains(&parent));
        let pid = std::fs::read_to_string(root.join("pid")).unwrap().parse::<u32>().unwrap();
        assert!(Command::new("systemctl").args(["--user", "stop", &parent]).status().unwrap().success());
        std::thread::sleep(std::time::Duration::from_millis(150));
        let after =
            std::fs::read_to_string(format!("/proc/{pid}/cgroup")).expect("application must survive service stop");
        assert_eq!(after, cgroup);
    }
}
