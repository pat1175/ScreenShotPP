//! Enregistrement vidéo : pilotage de ffmpeg (spawn, arrêt propre via 'q' sur
//! stdin, segments par pause, concaténation), export du mini-éditeur.
//! Toute la construction d'arguments est dans `screenshotpp_core::record`.

use screenshotpp_core::record as core_record;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};

/// Sur Windows, empêche l'ouverture d'une fenêtre de console (terminal noir)
/// lors du spawn de ffmpeg via le flag CREATE_NO_WINDOW. No-op ailleurs.
pub fn no_console(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Session d'enregistrement en cours : région ciblée, options, et segments
/// produits (un segment par intervalle démarrer→pause ; concaténés à l'arrêt).
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
struct RecordSession {
    region: core_record::Region,
    opts: core_record::RecordOptions,
    dir: PathBuf,
    base: String,
    /// Index de capture passé à ffmpeg : output_idx DXGI (Windows) ou
    /// périphérique avfoundation (macOS). Résolu une fois au démarrage.
    capture_index: u32,
    segments: Vec<PathBuf>,
    paused: bool,
}

#[derive(Default)]
struct Inner {
    /// Process ffmpeg du segment courant (None quand en pause ou arrêté).
    child: Option<Child>,
    session: Option<RecordSession>,
    /// Dernier fichier finalisé, servi au mini-éditeur.
    last_file: Option<PathBuf>,
    export_child: Option<Arc<Mutex<Child>>>,
    exported_files: Vec<PathBuf>,
}

/// Poignée partagée (clonable) sur l'état d'enregistrement : utilisée par la
/// boucle principale, le HUD, le mini-éditeur et les threads ffmpeg.
#[derive(Clone, Default)]
pub struct Recording(Arc<Mutex<Inner>>);

/// Résout le binaire ffmpeg embarqué, placé à côté de l'exécutable de l'app.
pub fn resolve_ffmpeg() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("Exécutable sans dossier parent")?;
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let path = dir.join(name);
    if path.exists() {
        Ok(path)
    } else {
        Err(format!(
            "ffmpeg introuvable ({}). Placez ffmpeg à côté de l'exécutable.",
            path.display()
        ))
    }
}

/// Dossier des enregistrements temporaires (<données app>/recordings).
pub fn recordings_dir() -> Result<PathBuf, String> {
    let dir = crate::paths::data_dir()?.join("recordings");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Index du périphérique avfoundation "Capture screen N" pour l'écran N (macOS).
#[cfg(target_os = "macos")]
fn avfoundation_device_for_screen(ffmpeg: &Path, screen_no: u32) -> Result<u32, String> {
    let out = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-f",
            "avfoundation",
            "-list_devices",
            "true",
            "-i",
            "",
        ])
        .output()
        .map_err(|e| format!("Énumération avfoundation échouée: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    core_record::parse_avfoundation_screens(&stderr)
        .into_iter()
        .find(|(_, s)| *s == screen_no)
        .map(|(d, _)| d)
        .ok_or_else(|| format!("Aucun périphérique avfoundation pour l'écran {screen_no}"))
}

/// Construit les args ffmpeg de capture d'un segment vers `out_path` selon l'OS.
fn segment_args(session: &RecordSession, out_path: &str) -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        Ok(core_record::windows_capture_args(
            session.capture_index,
            session.region,
            &session.opts,
            out_path,
        ))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(core_record::macos_capture_args(
            session.capture_index,
            session.region,
            &session.opts,
            out_path,
        ))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (session, out_path);
        Err("Enregistrement non pris en charge sur cette plateforme".into())
    }
}

/// Lance ffmpeg pour le prochain segment de la session ; renvoie le process et
/// le chemin du fichier segment (indexé par le nombre de segments déjà présents).
fn spawn_segment(ffmpeg: &Path, session: &RecordSession) -> Result<(Child, PathBuf), String> {
    let idx = session.segments.len();
    let out = session.dir.join(format!("{}-seg{}.mp4", session.base, idx));
    let out_path = out.to_string_lossy().to_string();
    let args = segment_args(session, &out_path)?;
    let child = no_console(&mut Command::new(ffmpeg))
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Lancement de ffmpeg échoué: {e}"))?;
    Ok((child, out))
}

/// Finalise proprement un process ffmpeg : 'q' sur stdin, puis kill après 5 s
/// (le MP4 fragmenté reste lisible même tué).
fn finalize_child(mut child: Child) {
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(b"q");
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
}

impl Recording {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Vrai tant qu'une session est active (y compris en pause).
    pub fn is_active(&self) -> bool {
        self.lock().session.is_some()
    }

    pub fn last_file(&self) -> Option<PathBuf> {
        self.lock().last_file.clone()
    }

    /// Démarre l'enregistrement de la région (pixels, relative au moniteur
    /// `monitor_index`) : crée la session et lance le premier segment ffmpeg.
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        allow(unreachable_code, unused_variables)
    )]
    pub fn start(
        &self,
        monitor_index: usize,
        region: core_record::Region,
        settings: &crate::settings::Settings,
    ) -> Result<(), String> {
        let opts = core_record::RecordOptions {
            fps: settings.record_fps.max(1),
            cursor: settings.record_cursor,
        };
        let ffmpeg = resolve_ffmpeg()?;
        let dir = recordings_dir()?;
        let base = format!("recording-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));

        #[cfg(windows)]
        let capture_index: u32 = monitor_index as u32;
        #[cfg(target_os = "macos")]
        let capture_index: u32 = avfoundation_device_for_screen(&ffmpeg, monitor_index as u32)?;
        #[cfg(not(any(windows, target_os = "macos")))]
        let capture_index: u32 = {
            let _ = (&ffmpeg, &dir, &base, &opts, region, monitor_index);
            return Err("Enregistrement non pris en charge sur cette plateforme".into());
        };

        let mut session = RecordSession {
            region,
            opts,
            dir,
            base,
            capture_index,
            segments: Vec::new(),
            paused: false,
        };
        let (child, seg) = spawn_segment(&ffmpeg, &session)?;
        session.segments.push(seg);

        let mut inner = self.lock();
        inner.child = Some(child);
        inner.session = Some(session);
        Ok(())
    }

    /// Met en pause : finalise le segment courant (concaténé à l'arrêt).
    /// Idempotent si déjà en pause. Peut bloquer jusqu'à 5 s (hors verrou).
    pub fn pause(&self) -> Result<(), String> {
        let child = {
            let mut inner = self.lock();
            match inner.session {
                None => return Err("Aucun enregistrement en cours".into()),
                Some(ref s) if s.paused => return Ok(()),
                _ => {}
            }
            let child = inner.child.take();
            if let Some(s) = inner.session.as_mut() {
                s.paused = true;
            }
            child
        };
        if let Some(c) = child {
            finalize_child(c);
        }
        Ok(())
    }

    /// Reprend après une pause : lance un nouveau segment ffmpeg.
    pub fn resume(&self) -> Result<(), String> {
        let ffmpeg = resolve_ffmpeg()?;
        let mut inner = self.lock();
        let (child, seg) = match inner.session {
            None => return Err("Aucun enregistrement en cours".into()),
            Some(ref s) if !s.paused => return Ok(()),
            Some(ref s) => spawn_segment(&ffmpeg, s)?,
        };
        if let Some(s) = inner.session.as_mut() {
            s.segments.push(seg);
            s.paused = false;
        }
        inner.child = Some(child);
        Ok(())
    }

    /// Arrête : finalise le segment courant et concatène si nécessaire. Le
    /// fichier produit devient `last_file`. Bloquant (à appeler hors UI).
    pub fn stop(&self) -> Result<(), String> {
        let (child, session) = {
            let mut inner = self.lock();
            (inner.child.take(), inner.session.take())
        };
        let session = session.ok_or("Aucun enregistrement en cours")?;
        if let Some(c) = child {
            finalize_child(c);
        }
        // Un seul segment : c'est directement le résultat. Sinon concaténation
        // sans réencodage (les segments partagent les mêmes paramètres).
        let result = if session.segments.len() <= 1 {
            session
                .segments
                .first()
                .cloned()
                .ok_or("Aucun segment enregistré")?
        } else {
            concat_segments(&session)?
        };
        let mut inner = self.lock();
        inner.last_file = Some(result);
        inner.exported_files.clear();
        Ok(())
    }

    /// Supprime le fichier temporaire (abandon depuis l'éditeur).
    pub fn discard(&self) {
        if let Some(f) = self.lock().last_file.take() {
            let _ = std::fs::remove_file(f);
        }
    }

    /// Exporte le dernier enregistrement. Bloquant ; `progress` reçoit les
    /// secondes de sortie traitées par ffmpeg.
    pub fn export(
        &self,
        options: &core_record::ExportOptions,
        output_path: &Path,
        progress: impl Fn(f64),
    ) -> Result<(), String> {
        use std::io::{BufRead, BufReader};
        let input = self
            .lock()
            .last_file
            .clone()
            .ok_or("Aucun enregistrement disponible")?;
        let ffmpeg = resolve_ffmpeg()?;
        let args = core_record::export_args(
            &input.to_string_lossy(),
            options,
            &output_path.to_string_lossy(),
        );
        let mut child = no_console(&mut Command::new(&ffmpeg))
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Lancement de l'export ffmpeg échoué: {e}"))?;
        let stderr = child.stderr.take();
        let export_child = Arc::new(Mutex::new(child));
        self.lock().export_child = Some(export_child.clone());
        if let Some(stderr) = stderr {
            // ffmpeg écrit la progression avec des \r ; on lit donc par blocs CR.
            for chunk in BufReader::new(stderr).split(b'\r') {
                let Ok(bytes) = chunk else { break };
                if let Some(secs) = core_record::parse_ffmpeg_time(&String::from_utf8_lossy(&bytes))
                {
                    progress(secs);
                }
            }
        }
        let status = export_child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .wait()
            .map_err(|e| e.to_string());
        self.lock().export_child = None;
        match status {
            Ok(s) if s.success() => {
                let exported = normalized_existing_path(output_path);
                let mut inner = self.lock();
                if !inner.exported_files.iter().any(|p| p == &exported) {
                    inner.exported_files.push(exported);
                }
                Ok(())
            }
            Ok(s) => {
                let _ = std::fs::remove_file(output_path);
                Err(format!("Export ffmpeg terminé en erreur ({s})"))
            }
            Err(e) => {
                let _ = std::fs::remove_file(output_path);
                Err(e)
            }
        }
    }

    /// Interrompt l'export en cours (l'appel `export` renvoie alors une erreur).
    pub fn cancel_export(&self) {
        let child = self.lock().export_child.clone();
        if let Some(child) = child {
            let _ = child.lock().unwrap_or_else(|e| e.into_inner()).kill();
        }
    }

    /// Supprime un export produit par cette session (MP4/GIF uniquement).
    pub fn delete_export(&self, path: &Path) -> Result<(), String> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if ext != "mp4" && ext != "gif" {
            return Err("Unsupported export type".into());
        }
        let target = normalized_existing_path(path);
        let mut inner = self.lock();
        if !inner
            .exported_files
            .iter()
            .any(|p| normalized_existing_path(p) == target)
        {
            return Err("Unknown recording export".into());
        }
        if path.exists() {
            std::fs::remove_file(&target).map_err(|e| e.to_string())?;
        }
        inner
            .exported_files
            .retain(|p| normalized_existing_path(p) != target);
        Ok(())
    }
}

fn normalized_existing_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Concatène les segments d'une session (démultiplexeur `concat`, `-c copy`).
/// En cas d'échec, retombe sur le premier segment pour ne rien perdre.
fn concat_segments(session: &RecordSession) -> Result<PathBuf, String> {
    let ffmpeg = resolve_ffmpeg()?;
    let list = session.dir.join(format!("{}-list.txt", session.base));
    let paths: Vec<String> = session
        .segments
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    std::fs::write(&list, core_record::concat_list_contents(&paths)).map_err(|e| e.to_string())?;
    let out = session.dir.join(format!("{}.mp4", session.base));
    let args = core_record::concat_args(&list.to_string_lossy(), &out.to_string_lossy());
    let status = no_console(&mut Command::new(&ffmpeg))
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Concaténation ffmpeg échouée: {e}"))?;
    let _ = std::fs::remove_file(&list);
    if status.success() {
        for seg in &session.segments {
            let _ = std::fs::remove_file(seg);
        }
        Ok(out)
    } else {
        // Repli : au moins le premier segment reste exploitable.
        Ok(session.segments[0].clone())
    }
}
