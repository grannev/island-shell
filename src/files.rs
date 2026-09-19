//! Cancellable filename search and asynchronous, type-aware opening.
//! Search never reads file contents; opening probes only the selected file.
use std::io::Read;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct FileHit {
    pub path: PathBuf,
    pub name: String,
    pub subtitle: String,
    pub directory: bool,
}

#[derive(Clone)]
struct Request {
    generation: u64,
    query: String,
}

#[derive(Default)]
pub struct Reply {
    pub files: Vec<FileHit>,
    pub note: Option<String>,
}

pub struct Search {
    generation: Arc<AtomicU64>,
    worker: crate::background::Worker<Request, Reply>,
}

impl Search {
    pub fn new() -> Self {
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        let root = std::env::var_os("HOME").map(PathBuf::from);
        let worker = crate::background::Worker::new("file-search", move |request: Request| {
            let Some(root) = &root else {
                return Reply {
                    note: Some("Не задан домашний каталог".into()), ..Reply::default()
                };
            };
            search(root, &request.query, || current.load(Ordering::Relaxed) != request.generation)
        });
        Self { generation, worker }
    }
    pub fn set_waker(&self, tx: smithay_client_toolkit::reexports::calloop::channel::Sender<()>) {
        self.worker.set_waker(tx);
    }
    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
    pub fn request(&self, query: String) -> Option<u64> {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.worker.request(Request { generation, query }).then_some(generation)
    }
    pub fn drain(&self) -> Vec<(u64, Reply)> {
        self.worker.drain().into_iter().map(|(r, reply)| (r.generation, reply)).collect()
    }
}

/// Prefix is optional; ordinary queries still search both apps and files.
pub fn query(text: &str) -> (&str, bool) {
    let text = text.trim();
    for prefix in ["file:", "файл:"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            return (rest.trim(), true);
        }
    }
    (text, false)
}

fn search(root: &Path, query: &str, canceled: impl Fn() -> bool) -> Reply {
    if canceled() || query.chars().count() < 2 {
        return Reply::default();
    }
    let pattern = if let Some(rest) = query.strip_prefix("~/") {
        root.join(rest).to_string_lossy().into_owned()
    } else {
        query.to_string()
    };
    let mut child = None;
    for program in ["fdfind", "fd"] {
        let mut cmd = Command::new(program);
        cmd.args([
            "--fixed-strings",
            "--ignore-case",
            "--absolute-path",
            "--print0",
            "--type",
            "f",
            "--type",
            "d",
            "--type",
            "l",
            "--max-results",
            "80",
            "--threads",
            "2",
            "--exclude",
            "node_modules",
            "--exclude",
            "target",
            "--exclude",
            "__pycache__",
        ]);
        if pattern.contains('/') {
            cmd.arg("--full-path");
        }
        match cmd
            .arg("--")
            .arg(&pattern)
            .arg(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(process) => {
                child = Some(process);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                return Reply {
                    note: Some("Не удалось начать поиск файлов".into()), ..Reply::default()
                };
            }
        }
    }
    let Some(mut child) = child else {
        return Reply { note: Some("Для поиска файлов нужен fd-find".into()), ..Reply::default() };
    };
    let stdout = child.stdout.take().expect("piped search output");
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.take(1024 * 1024).read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    let mut partial = false;
    let mut failed = false;
    loop {
        if canceled() || started.elapsed() > Duration::from_secs(3) {
            partial = true;
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                failed = !status.success();
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                failed = true;
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    let bytes = reader.join().unwrap_or_default();
    if canceled() {
        return Reply::default();
    }
    let mut files: Vec<_> = bytes
        .split(|b| *b == 0)
        .filter(|b| !b.is_empty())
        .filter_map(|bytes| {
            let path = PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()));
            let metadata = path.metadata().ok()?;
            if !metadata.is_file() && !metadata.is_dir() {
                return None;
            }
            let directory = metadata.is_dir();
            let name = path.file_name()?.to_string_lossy().into_owned();
            let parent = path.parent().unwrap_or(root);
            let parent = parent
                .strip_prefix(root)
                .map(|p| format!("~/{}", p.to_string_lossy()))
                .unwrap_or_else(|_| parent.to_string_lossy().into_owned());
            let subtitle = format!("{} · {parent}", if directory { "Каталог" } else { "Файл" });
            Some(FileHit { path, name, subtitle, directory })
        })
        .collect();
    let needle = Path::new(query).file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    files.sort_by_cached_key(|f| {
        let name = f.name.to_lowercase();
        (name != needle, !name.starts_with(&needle), f.path.components().count(), name, f.path.clone())
    });
    files.dedup_by(|a, b| a.path == b.path);
    let note = if failed {
        Some("Поиск файлов завершился с ошибкой".into())
    } else if partial || files.len() == 80 {
        Some("Часть результатов · уточните имя".into())
    } else {
        None
    };
    Reply { files, note }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Opener {
    Directory,
    Text,
    Pdf,
    Office,
    Default,
}

fn extension_opener(path: &Path) -> Option<Opener> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "pdf" => Some(Opener::Pdf),
        "doc" | "docx" | "docm" | "dot" | "dotx" | "odt" | "ott" | "rtf" | "xls" | "xlsx" | "xlsm" | "ods" | "ots"
        | "ppt" | "pptx" | "pptm" | "odp" | "otp" | "odg" => Some(Opener::Office),
        // These formats can contain text internally but are visual documents.
        "svg" | "eps" | "ps" => Some(Opener::Default),
        _ => None,
    }
}

fn mime_opener(mime: &str) -> Opener {
    match mime.trim() {
        "application/pdf" => Opener::Pdf,
        "application/json"
        | "application/xml"
        | "application/javascript"
        | "application/x-javascript"
        | "application/x-shellscript"
        | "application/x-empty"
        | "inode/x-empty" => Opener::Text,
        "text/rtf" | "application/rtf" | "application/msword" => Opener::Office,
        mime if mime.starts_with("application/vnd.oasis.opendocument.")
            || mime.starts_with("application/vnd.openxmlformats-officedocument.")
            || mime.starts_with("application/vnd.ms-") =>
        {
            Opener::Office
        }
        mime if mime.starts_with("text/") => Opener::Text,
        _ => Opener::Default,
    }
}

/// Run only off the UI thread. Limit probing and never inspect special files.
fn detect_opener(path: &Path) -> std::io::Result<Opener> {
    let metadata = path.metadata()?;
    if metadata.is_dir() {
        return Ok(Opener::Directory);
    }
    if !metadata.is_file() {
        return Err(std::io::Error::other("selected path is not a regular file or directory"));
    }
    if let Some(opener) = extension_opener(path) {
        return Ok(opener);
    }
    let output = Command::new("file")
        .args(["--brief", "--mime-type", "--dereference", "--parameter", "bytes=65536", "--"])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    Ok(match output {
        Ok(output) if output.status.success() => mime_opener(&String::from_utf8_lossy(&output.stdout)),
        _ => Opener::Default,
    })
}

/// Pass paths as single arguments, never through a shell.
fn open_command(path: &Path, opener: Opener) -> Command {
    let mut cmd = match opener {
        Opener::Directory => {
            let mut cmd = Command::new("alacritty");
            cmd.arg("--working-directory").arg(path);
            cmd
        }
        Opener::Text => {
            let mut cmd = Command::new("alacritty");
            if let Some(parent) = path.parent() {
                cmd.arg("--working-directory").arg(parent);
            }
            cmd.args(["-e", "nvim", "--"]).arg(path);
            cmd
        }
        other => {
            let mut cmd = Command::new(match other {
                Opener::Pdf => "evince",
                Opener::Office => "libreoffice",
                _ => "xdg-open",
            });
            // Search produces absolute paths, so leading '-' cannot be an option.
            cmd.arg(path);
            cmd
        }
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::inherit());
    cmd
}

pub fn open(path: &Path) -> bool {
    let path = path.to_owned();
    // Metadata, MIME detection and process startup must not delay closing motion.
    let job = std::thread::Builder::new().name("file-open".into()).spawn(move || {
        let result = (|| -> std::io::Result<()> {
            let path = std::path::absolute(path)?;
            let opener = detect_opener(&path)?;
            let command = open_command(&path, opener);
            let mut child = match crate::launch::spawn(&command) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && opener != Opener::Default => {
                    eprintln!("island-shell: {:?} not installed; using system association", command.get_program());
                    crate::launch::spawn(&open_command(&path, Opener::Default))?
                }
                child => child?,
            };
            let status = child.wait()?;
            if !status.success() {
                eprintln!("island-shell: file opener exited with {status}");
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("island-shell: open file: {e}");
        }
    });
    match job {
        Ok(_) => true,
        Err(e) => {
            eprintln!("island-shell: open file: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_prefix_is_optional_and_preserves_spaces() {
        assert_eq!(query("  отчёт 2026  "), ("отчёт 2026", false));
        assert_eq!(query("file: отчёт 2026"), ("отчёт 2026", true));
        assert_eq!(query("файл:README"), ("README", true));
        assert_eq!(query("firefox"), ("firefox", false));
    }

    #[test]
    fn file_names_are_arguments_never_shell_code() {
        let path = Path::new("/tmp/a name; $(touch nope).txt");
        let command = open_command(path, Opener::Default);
        assert_eq!(command.get_program(), "xdg-open");
        assert_eq!(command.get_args().collect::<Vec<_>>(), vec![path.as_os_str()]);
        let command = open_command(path, Opener::Text);
        assert_eq!(command.get_program(), "alacritty");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["--working-directory", "/tmp", "-e", "nvim", "--"]
                .map(std::ffi::OsStr::new)
                .into_iter()
                .chain([path.as_os_str()])
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn directories_and_documents_have_explicit_openers() {
        let path = Path::new("/tmp/Мои документы");
        let command = open_command(path, Opener::Directory);
        assert_eq!(command.get_program(), "alacritty");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![std::ffi::OsStr::new("--working-directory"), path.as_os_str()]
        );
        for (name, opener, program) in [
            ("Report.PDF", Opener::Pdf, "evince"),
            ("Report.docx", Opener::Office, "libreoffice"),
            ("Report.ODT", Opener::Office, "libreoffice"),
            ("Report.xlsx", Opener::Office, "libreoffice"),
            ("image.svg", Opener::Default, "xdg-open"),
        ] {
            let path = Path::new("/tmp").join(name);
            assert_eq!(extension_opener(&path), Some(opener));
            let command = open_command(&path, opener);
            assert_eq!(command.get_program(), program);
            assert_eq!(command.get_args().collect::<Vec<_>>(), vec![path.as_os_str()]);
        }
    }

    #[test]
    fn mime_detection_distinguishes_text_from_binary_and_office() {
        for mime in
            ["text/plain\n", "text/x-rust", "text/x-python", "application/json", "application/xml", "inode/x-empty"]
        {
            assert_eq!(mime_opener(mime), Opener::Text, "{mime}");
        }
        for mime in ["image/png", "image/svg+xml", "application/zip", "application/octet-stream", "audio/mpeg"] {
            assert_eq!(mime_opener(mime), Opener::Default, "{mime}");
        }
        assert_eq!(mime_opener("application/pdf"), Opener::Pdf);
        assert_eq!(mime_opener("text/rtf"), Opener::Office);
        assert_eq!(mime_opener("application/vnd.oasis.opendocument.text"), Opener::Office);
    }

    #[test]
    fn canceled_and_empty_queries_do_not_start_a_search() {
        assert!(search(Path::new("/not/a/directory"), "test", || true).files.is_empty());
        assert!(search(Path::new("/not/a/directory"), "a", || false).files.is_empty());
    }

    #[test]
    #[ignore = "Requires fd-find; searches only a private temporary fixture"]
    fn real_search_handles_unicode_spaces_literal_symbols_and_hidden_files() {
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("island-file-test-{}-{unique}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _fixture = Fixture(root.clone());
        assert_eq!(detect_opener(&root).unwrap(), Opener::Directory);
        std::fs::create_dir(root.join("Документы")).unwrap();
        for name in ["Отчёт [2026].txt", "Отчёт другой.txt", ".Отчёт секрет.txt"] {
            std::fs::write(root.join("Документы").join(name), "fixture").unwrap();
        }
        let found = search(&root, "отчёт [2026]", || false);
        assert!(found.note.is_none(), "{:?}", found.note);
        assert_eq!(found.files.len(), 1);
        assert_eq!(found.files[0].name, "Отчёт [2026].txt");
        assert_eq!(detect_opener(&found.files[0].path).unwrap(), Opener::Text);
        let binary = root.join("binary.txt");
        std::fs::write(&binary, [0, 1, 2, 3, 0, 255]).unwrap();
        assert_eq!(detect_opener(&binary).unwrap(), Opener::Default);
        assert_eq!(found.files[0].subtitle, "Файл · ~/Документы");
        let found = search(&root, "отчёт", || false);
        assert_eq!(found.files.len(), 2);
        let found = search(&root, "Документы", || false);
        assert_eq!(found.files.len(), 1);
        assert!(found.files[0].directory);
        assert_eq!(found.files[0].subtitle, "Каталог · ~/");
        let found = search(&root, "~/Документы/Отчёт [2026]", || false);
        assert_eq!(found.files.len(), 1);
    }
}
