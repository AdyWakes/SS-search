//! ONNX Runtime bootstrap. The runtime library is bundled with the app and
//! loaded by absolute path (never from the system search path, which on
//! Windows can contain an old `onnxruntime.dll` in System32).

use anyhow::{anyhow, bail, Context, Result};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static ENV: OnceLock<std::result::Result<ort::environment::Environment, String>> = OnceLock::new();
static DYLIB: OnceLock<PathBuf> = OnceLock::new();

/// File name of the ONNX Runtime shared library on this platform.
pub fn dylib_name() -> &'static str {
    if cfg!(windows) {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

/// Initialises ONNX Runtime once. `candidates` are tried in order, after the
/// `ORT_DYLIB_PATH` environment variable.
pub fn init(candidates: &[PathBuf]) -> Result<()> {
    let r = ENV.get_or_init(|| {
        let mut tried = Vec::new();
        let env_path = std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from);
        for p in env_path.iter().chain(candidates.iter()) {
            tried.push(p.display().to_string());
            if !p.is_file() {
                continue;
            }
            match ort::init_from(p) {
                Ok(b) => match b.with_name("shotfind").build() {
                    Ok(env) => {
                        let _ = DYLIB.set(p.clone());
                        tracing::info!("ONNX Runtime loaded from {}", p.display());
                        return Ok(env);
                    }
                    Err(e) => return Err(format!("ONNX Runtime init failed: {e}")),
                },
                Err(e) => return Err(format!("could not load {}: {e}", p.display())),
            }
        }
        Err(format!("ONNX Runtime library not found (tried: {})", tried.join(", ")))
    });
    r.as_ref().map(|_| ()).map_err(|e| anyhow!("{e}"))
}

pub fn is_ready() -> bool {
    ENV.get().is_some_and(|r| r.is_ok())
}

pub fn loaded_from() -> Option<&'static Path> {
    DYLIB.get().map(|p| p.as_path())
}

/// Opens an inference session with a bounded thread count (we are a background app).
pub fn session(model: &Path, threads: usize) -> Result<Session> {
    let env = match ENV.get() {
        Some(Ok(env)) => env,
        _ => bail!("ONNX Runtime is not initialised"),
    };
    let mut b = Session::builder(env)
        .map_err(|e| anyhow!("{e}"))?
        .with_optimization_level(GraphOptimizationLevel::All)
        .map_err(|e| anyhow!("{e}"))?
        .with_intra_threads(threads.max(1))
        .map_err(|e| anyhow!("{e}"))?
        .with_inter_threads(1)
        .map_err(|e| anyhow!("{e}"))?
        // OCR inputs change shape every call; ORT's per-shape memory pattern
        // cache would otherwise grow by hundreds of MB.
        .with_memory_pattern(false)
        .map_err(|e| anyhow!("{e}"))?
        // Without the arena, memory goes back to the OS after each image instead
        // of staying at the high-water mark of the largest screenshot seen.
        .with_execution_providers([ort::ep::CPU::default()
            .with_arena_allocator(std::env::var_os("SHOTFIND_ORT_ARENA").is_some())
            .build()])
        .map_err(|e| anyhow!("{e}"))?;
    if threads > 1 {
        // ORT's own pool threads run at background priority like ours.
        b = b.with_thread_manager(BackgroundThreads).map_err(|e| anyhow!("{e}"))?;
    }
    b.commit_from_file(model).with_context(|| format!("loading model {}", model.display()))
}

/// Spawns ONNX Runtime's intra-op pool threads at background priority (on
/// Windows, threads start at normal priority regardless of their creator).
struct BackgroundThreads;

impl ort::environment::ThreadManager for BackgroundThreads {
    type Thread = std::thread::JoinHandle<()>;

    fn create(&self, work: impl FnOnce() + Send + 'static) -> ort::Result<Self::Thread> {
        std::thread::Builder::new()
            .name("shotfind-ort".into())
            .spawn(move || {
                lower_thread_priority();
                work()
            })
            .map_err(|e| ort::Error::new(e.to_string()))
    }

    fn join(thread: Self::Thread) -> ort::Result<()> {
        thread.join().map_err(|_| ort::Error::new("ORT worker thread panicked"))
    }
}

/// Lowest scheduling priority for the current thread. On Windows this is
/// background mode, which also lowers I/O and memory priority.
pub fn lower_thread_priority() {
    #[cfg(windows)]
    {
        use thread_priority::windows::{set_current_thread_priority as set_win, WinAPIThreadPriority};
        if set_win(WinAPIThreadPriority::BackgroundModeBegin.into()).is_ok() {
            return;
        }
    }
    let _ = thread_priority::set_current_thread_priority(thread_priority::ThreadPriority::Min);
}
