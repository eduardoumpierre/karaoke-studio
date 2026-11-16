//! Inicia e encerra o music-engine (Python) e entrega à UI a porta e o token.
//!
//! Protocolo (ver services/music-engine/app/__main__.py):
//!   - o token vai na variável KARAOKE_TOKEN; o backend escolhe a porta;
//!   - o backend imprime {"event":"ready","port":N} no stdout quando está pronto;
//!   - fechar o stdin do backend o faz encerrar (funciona mesmo se este processo morrer).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use rand::RngCore;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendConfig {
    pub base_url: String,
    pub token: String,
}

#[derive(Clone, Debug)]
enum Status {
    Starting,
    Ready(BackendConfig),
    Failed(String),
}

pub struct Shared {
    status: Mutex<Status>,
    changed: Condvar,
}

impl Shared {
    fn new() -> Self {
        Shared {
            status: Mutex::new(Status::Starting),
            changed: Condvar::new(),
        }
    }

    fn set(&self, status: Status) {
        *self.status.lock().unwrap() = status;
        self.changed.notify_all();
    }

    fn set_if_starting(&self, status: Status) {
        let mut guard = self.status.lock().unwrap();
        if matches!(*guard, Status::Starting) {
            *guard = status;
            self.changed.notify_all();
        }
    }

    /// Bloqueia até o backend ficar pronto, falhar ou estourar o tempo.
    pub fn wait_ready(&self, timeout: Duration) -> Result<BackendConfig, String> {
        let deadline = Instant::now() + timeout;
        let mut guard = self.status.lock().unwrap();
        loop {
            match &*guard {
                Status::Ready(cfg) => return Ok(cfg.clone()),
                Status::Failed(msg) => return Err(msg.clone()),
                Status::Starting => {}
            }
            let left = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| "O backend não ficou pronto a tempo.".to_string())?;
            let (g, _) = self.changed.wait_timeout(guard, left).unwrap();
            guard = g;
        }
    }
}

/// Onde está o Python do backend e a pasta do serviço.
pub struct Launch {
    pub python: PathBuf,
    pub engine_dir: PathBuf,
}

impl Launch {
    /// Modo de desenvolvimento: usa o .venv do repositório. Pode ser trocado por
    /// KARAOKE_PYTHON e KARAOKE_ENGINE_DIR.
    pub fn detect() -> Result<Launch, String> {
        let engine_dir = match std::env::var_os("KARAOKE_ENGINE_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../services/music-engine"),
        };
        let python = match std::env::var_os("KARAOKE_PYTHON") {
            Some(p) => PathBuf::from(p),
            None if cfg!(windows) => engine_dir.join(".venv/Scripts/python.exe"),
            None => engine_dir.join(".venv/bin/python"),
        };
        if !python.exists() {
            return Err(format!(
                "Python do backend não encontrado em {}. Rode `uv sync` em services/music-engine.",
                python.display()
            ));
        }
        Ok(Launch { python, engine_dir })
    }
}

pub struct Sidecar {
    shared: Arc<Shared>,
    child: Mutex<Option<Child>>,
}

impl Sidecar {
    /// Nunca falha: se não der para iniciar, o erro é entregue a quem chamar `wait_ready`.
    pub fn start(data_dir: PathBuf) -> Sidecar {
        let shared = Arc::new(Shared::new());
        let sidecar = Sidecar {
            shared: shared.clone(),
            child: Mutex::new(None),
        };
        match Launch::detect().and_then(|launch| spawn(&launch, &data_dir)) {
            Ok((mut child, token)) => {
                let stdout = child.stdout.take();
                let stderr = child.stderr.take();
                if let Some(out) = stdout {
                    let shared = shared.clone();
                    std::thread::spawn(move || read_stdout(out, token, shared));
                }
                if let Some(err) = stderr {
                    std::thread::spawn(move || {
                        for line in BufReader::new(err).lines().map_while(Result::ok) {
                            log::info!(target: "music-engine", "{line}");
                        }
                    });
                }
                *sidecar.child.lock().unwrap() = Some(child);
            }
            Err(msg) => {
                log::error!("não foi possível iniciar o backend: {msg}");
                shared.set(Status::Failed(msg));
            }
        }
        sidecar
    }

    pub fn shared(&self) -> Arc<Shared> {
        self.shared.clone()
    }

    /// Fecha o stdin (pedido educado de encerramento) e mata o processo se demorar.
    pub fn shutdown(&self) {
        let Some(mut child) = self.child.lock().unwrap().take() else {
            return;
        };
        drop(child.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                log::info!("backend encerrado.");
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        log::warn!("backend não encerrou a tempo; forçando.");
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn spawn(launch: &Launch, data_dir: &Path) -> Result<(Child, String), String> {
    let token = new_token();
    let mut cmd = Command::new(&launch.python);
    cmd.args(["-m", "app"])
        .current_dir(&launch.engine_dir)
        .env("KARAOKE_TOKEN", &token)
        .env("KARAOKE_DATA_DIR", data_dir)
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("Falha ao iniciar o backend: {e}"))?;
    log::info!("backend iniciado (pid {})", child.id());
    Ok((child, token))
}

fn read_stdout(out: std::process::ChildStdout, token: String, shared: Arc<Shared>) {
    for line in BufReader::new(out).lines().map_while(Result::ok) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            if v["event"] == "ready" {
                if let Some(port) = v["port"].as_u64() {
                    log::info!("backend pronto na porta {port}");
                    shared.set(Status::Ready(BackendConfig {
                        base_url: format!("http://127.0.0.1:{port}"),
                        token: token.clone(),
                    }));
                    continue;
                }
            }
        }
        log::info!(target: "music-engine", "{line}");
    }
    shared.set_if_starting(Status::Failed(
        "O backend encerrou antes de ficar pronto. Veja o log em music-engine.log.".to_string(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_ready_returns_config_once_ready() {
        let shared = Arc::new(Shared::new());
        let s2 = shared.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            s2.set(Status::Ready(BackendConfig {
                base_url: "http://127.0.0.1:1".into(),
                token: "t".into(),
            }));
        });
        let cfg = shared.wait_ready(Duration::from_secs(5)).unwrap();
        assert_eq!(cfg.base_url, "http://127.0.0.1:1");
        t.join().unwrap();
    }

    #[test]
    fn wait_ready_reports_failure_and_timeout() {
        let failed = Shared::new();
        failed.set(Status::Failed("deu ruim".into()));
        assert_eq!(
            failed.wait_ready(Duration::from_secs(1)).unwrap_err(),
            "deu ruim"
        );
        let starting = Shared::new();
        assert!(starting.wait_ready(Duration::from_millis(50)).is_err());
    }

    #[test]
    fn config_serializes_in_camel_case() {
        let cfg = BackendConfig {
            base_url: "u".into(),
            token: "t".into(),
        };
        assert_eq!(
            serde_json::to_string(&cfg).unwrap(),
            r#"{"baseUrl":"u","token":"t"}"#
        );
    }

    #[test]
    fn tokens_are_long_and_unique() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    /// Teste de ponta a ponta com o backend de verdade. Só roda se KARAOKE_PYTHON estiver definido.
    #[test]
    fn starts_and_stops_the_real_backend() {
        if std::env::var_os("KARAOKE_PYTHON").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("ks-sidecar-test-{}", std::process::id()));
        let sidecar = Sidecar::start(dir);
        let cfg = sidecar
            .shared()
            .wait_ready(Duration::from_secs(30))
            .expect("backend deveria subir");
        assert!(cfg.base_url.starts_with("http://127.0.0.1:"));
        assert_eq!(cfg.token.len(), 64);
        sidecar.shutdown();
    }
}
