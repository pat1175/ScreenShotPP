//! Prévisualisation native : ffmpeg décode, egui ne reçoit que quelques frames.
use screenshotpp_core::record::{last_ffmpeg_time, parse_video_info, VideoInfo};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

#[derive(Default)]
struct ChildSlot(Mutex<Option<Child>>);
impl std::ops::Deref for ChildSlot {
    type Target = Mutex<Option<Child>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Drop for ChildSlot {
    fn drop(&mut self) {
        // L'Arc garantit une récupération unique, même si UI et thread finissent ensemble.
        if let Some(mut child) = self.0.get_mut().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
#[derive(Clone, Default)]
struct Process {
    child: Arc<ChildSlot>,
    cancelled: Arc<AtomicBool>,
}
impl Process {
    fn spawn(&self, command: &mut Command) -> Result<(), String> {
        let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if self.cancelled.load(Ordering::Acquire) {
            return Err("Decode cancelled".into());
        }
        *slot = Some(
            crate::record::no_console(command)
                .spawn()
                .map_err(|e| e.to_string())?,
        );
        Ok(())
    }
    fn stop(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(child) = self
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let _ = child.kill();
        }
    }
    fn finish(&self) -> Result<bool, String> {
        // Ne pas garder le mutex pendant wait : la fenêtre doit pouvoir tuer le process.
        loop {
            let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner());
            let Some(child) = slot.as_mut() else {
                return Ok(false);
            };
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                *slot = None;
                return Ok(status.success());
            }
            drop(slot);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
struct Job<T> {
    process: Process,
    receiver: Receiver<T>,
}
impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.process.stop();
    }
}

fn probe(file: &Path, process: &Process) -> Result<VideoInfo, String> {
    let ffmpeg = crate::record::resolve_ffmpeg()?;
    process.spawn(
        Command::new(&ffmpeg)
            .args(["-hide_banner", "-i"])
            .arg(file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
    )?;
    let stderr = process
        .child
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
        .unwrap()
        .stderr
        .take()
        .unwrap();
    let mut text = String::new();
    BufReader::new(stderr)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    let _ = process.finish()?; // Sans sortie, ffmpeg retourne normalement 1.
    let mut info =
        parse_video_info(&text).ok_or_else(|| format!("Could not read video metadata: {text}"))?;
    if info.duration.is_none() {
        process.spawn(
            Command::new(&ffmpeg)
                .args(["-hide_banner", "-i"])
                .arg(file)
                .args(["-map", "0:v:0", "-an", "-f", "null", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped()),
        )?;
        let stderr = process
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
            .unwrap()
            .stderr
            .take()
            .unwrap();
        let mut time = None;
        for chunk in BufReader::new(stderr).split(b'\r') {
            let chunk = chunk.map_err(|e| e.to_string())?;
            if let Some(t) = last_ffmpeg_time(&String::from_utf8_lossy(&chunk)) {
                time = Some(t);
            }
        }
        if !process.finish()? {
            return Err("Could not decode recording duration".into());
        }
        info.duration = time.filter(|t| *t > 0.0);
    }
    if info.duration.is_none() {
        return Err("Recording has no readable duration".into());
    }
    Ok(info)
}

struct Frame {
    time: f64,
    rgba: Vec<u8>,
}
enum Decoded {
    Frame(Frame),
    End,
    Error(String),
}
const FPS: f64 = 24.0;
fn send_frame(
    tx: &SyncSender<Decoded>,
    process: &Process,
    mut frame: Decoded,
    ctx: &egui::Context,
) -> bool {
    loop {
        if process.cancelled.load(Ordering::Acquire) {
            return false;
        }
        match tx.try_send(frame) {
            Ok(()) => {
                ctx.request_repaint();
                return true;
            }
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(value)) => {
                frame = value;
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
fn decoder(
    file: PathBuf,
    time: f64,
    single: bool,
    size: [usize; 2],
    process: Process,
    tx: SyncSender<Decoded>,
    recycled: Receiver<Vec<u8>>,
    ctx: egui::Context,
) {
    let result = (|| -> Result<(), String> {
        let ffmpeg = crate::record::resolve_ffmpeg()?;
        let mut command = Command::new(ffmpeg);
        let filter = if single {
            format!("scale={}:-2", size[0])
        } else {
            format!("fps={FPS},scale={}:-2", size[0])
        };
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-ss",
                &format!("{time:.6}"),
                "-i",
            ])
            .arg(file)
            .args(["-map", "0:v:0", "-an", "-vf", &filter]);
        if single {
            command.args(["-frames:v", "1"]);
        }
        command
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        process.spawn(&mut command)?;
        let (stdout, stderr) = {
            let mut slot = process.child.lock().unwrap_or_else(|e| e.into_inner());
            let child = slot.as_mut().unwrap();
            (child.stdout.take().unwrap(), child.stderr.take().unwrap())
        };
        // stderr est drainé indépendamment pour ne jamais bloquer le flux RGBA.
        let errors = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = BufReader::new(stderr).read_to_string(&mut text);
            text
        });
        let mut reader = BufReader::new(stdout);
        let mut index = 0;
        loop {
            let mut rgba = recycled
                .try_recv()
                .unwrap_or_else(|_| vec![0; size[0] * size[1] * 4]);
            match reader.read_exact(&mut rgba) {
                Ok(()) => {
                    if !send_frame(
                        &tx,
                        &process,
                        Decoded::Frame(Frame {
                            time: time + f64::from(index) / FPS,
                            rgba,
                        }),
                        &ctx,
                    ) {
                        break;
                    }
                    index += 1;
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.to_string()),
            }
        }
        let success = process.finish()?;
        let errors = errors.join().unwrap_or_default();
        if !success && !process.cancelled.load(Ordering::Acquire) {
            return Err(format!("Video decode failed: {errors}"));
        }
        Ok(())
    })();
    if !process.cancelled.load(Ordering::Acquire) {
        let event = match result {
            Ok(()) => Decoded::End,
            Err(e) => Decoded::Error(e),
        };
        let _ = send_frame(&tx, &process, event, &ctx);
    }
}

pub struct Video {
    file: PathBuf,
    probe: Option<Job<Result<VideoInfo, String>>>,
    decode: Option<Job<Decoded>>,
    recycled: Option<SyncSender<Vec<u8>>>,
    pending_frame: Option<Frame>,
    seek_pending: Option<(f64, Instant)>,
    pub info: Option<VideoInfo>,
    pub texture: Option<egui::TextureHandle>,
    pub time: f64,
    pub playing: bool,
    clock: Option<(Instant, f64)>,
    decode_size: [usize; 2],
    single: bool,
}
impl Video {
    pub fn new(file: PathBuf) -> Self {
        let (tx, receiver) = mpsc::channel();
        let process = Process::default();
        let worker = process.clone();
        let source = file.clone();
        std::thread::spawn(move || {
            let _ = tx.send(probe(&source, &worker));
        });
        Self {
            file,
            probe: Some(Job { process, receiver }),
            decode: None,
            recycled: None,
            pending_frame: None,
            seek_pending: None,
            info: None,
            texture: None,
            time: 0.0,
            playing: false,
            clock: None,
            decode_size: [0, 0],
            single: false,
        }
    }
    fn start_decode(&mut self, ctx: &egui::Context, single: bool) {
        let Some(info) = self.info else {
            return;
        };
        let width = info.width.min(960).max(2) as usize;
        // scale=W:-2 arrondit la hauteur à l'entier pair le plus proche.
        let height = ((f64::from(info.height) * width as f64 / f64::from(info.width) / 2.0).round()
            as usize
            * 2)
        .max(2);
        self.decode_size = [width, height];
        let (tx, receiver) = mpsc::sync_channel(3);
        let (recycle_tx, recycled) = mpsc::sync_channel(4);
        self.recycled = Some(recycle_tx);
        let process = Process::default();
        let worker = process.clone();
        let file = self.file.clone();
        let time = self
            .time
            .min((info.duration.unwrap_or_default() - 1.0 / FPS).max(0.0));
        let context = ctx.clone();
        let size = self.decode_size;
        std::thread::spawn(move || {
            decoder(file, time, single, size, worker, tx, recycled, context)
        });
        self.decode = Some(Job { process, receiver });
        self.single = single;
    }
    pub fn seek(&mut self, time: f64) {
        self.pause();
        self.time = time.max(0.0);
        self.seek_pending = Some((self.time, Instant::now()));
    }
    pub fn loading(&self) -> bool {
        self.probe.is_some()
    }
    pub fn pause(&mut self) {
        self.playing = false;
        self.clock = None;
        if let Some(job) = self.decode.as_ref() {
            job.process.stop();
        }
        self.pending_frame = None;
    }
    pub fn shutdown(&mut self) {
        self.pause();
        if let Some(job) = self.probe.as_ref() {
            job.process.stop();
            let _ = job.process.finish();
        }
        if let Some(job) = self.decode.as_ref() {
            let _ = job.process.finish();
        }
        self.probe = None;
        self.decode = None;
        self.seek_pending = None;
        self.recycled = None;
    }
    pub fn toggle(&mut self, ctx: &egui::Context, start: f64, end: f64) {
        if self.playing {
            self.seek(self.time);
            return;
        }
        if self.info.is_none() {
            return;
        }
        self.decode = None;
        self.pending_frame = None;
        self.seek_pending = None;
        if self.time < start || self.time >= end {
            self.time = start;
        }
        self.playing = true;
        // L'horloge démarre avec la première frame, pas au lancement de ffmpeg.
        self.clock = None;
        self.start_decode(ctx, false);
    }
    pub fn update(&mut self, ctx: &egui::Context, trim_end: f64) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(job) = self.probe.as_ref() {
            match job.receiver.try_recv() {
                Ok(result) => {
                    self.probe = None;
                    match result {
                        Ok(info) => {
                            self.info = Some(info);
                            self.seek_pending =
                                Some((0.0, Instant::now() - Duration::from_millis(120)));
                        }
                        Err(e) => errors.push(e),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.probe = None;
                    errors.push("Video probe stopped unexpectedly".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some((clock, origin)) = self.clock {
            self.time = origin + clock.elapsed().as_secs_f64();
        }
        if self.playing && trim_end > 0.0 && self.time >= trim_end {
            self.seek(trim_end);
        }
        if self.pending_frame.is_none() {
            if let Some(job) = self.decode.as_ref() {
                match job.receiver.try_recv() {
                    Ok(Decoded::Frame(frame)) => {
                        if self.playing && self.clock.is_none() {
                            self.clock = Some((Instant::now(), frame.time));
                            self.time = frame.time;
                        }
                        if self.seek_pending.is_none() {
                            self.pending_frame = Some(frame);
                        }
                    }
                    Ok(Decoded::End) | Err(mpsc::TryRecvError::Disconnected) => {
                        self.decode = None;
                        if self.seek_pending.is_none() {
                            self.playing = false;
                            self.clock = None;
                        }
                    }
                    Ok(Decoded::Error(e)) => {
                        self.decode = None;
                        self.playing = false;
                        self.clock = None;
                        errors.push(e);
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
        }
        if self
            .pending_frame
            .as_ref()
            .is_some_and(|frame| self.single || frame.time <= self.time + 0.002)
        {
            let frame = self.pending_frame.take().unwrap();
            // Une frame d'une recherche annulée ne remplace pas la dernière demande.
            if self.seek_pending.is_none() {
                let image = egui::ColorImage::from_rgba_unmultiplied(self.decode_size, &frame.rgba);
                if let Some(texture) = self.texture.as_mut() {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ctx.load_texture(
                        "recording-preview",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            if let Some(recycled) = self.recycled.as_ref() {
                let _ = recycled.try_send(frame.rgba);
            }
        }
        if self.seek_pending.is_some() && self.decode.is_none() {
            let (time, requested) = self.seek_pending.unwrap();
            if requested.elapsed() >= Duration::from_millis(100) {
                self.seek_pending = None;
                self.time = time;
                self.start_decode(ctx, true);
            }
        }
        if self.probe.is_some()
            || self.decode.is_some()
            || self.seek_pending.is_some()
            || self.playing
        {
            ctx.request_repaint_after(Duration::from_millis(15));
        }
        errors
    }
}
impl Drop for Video {
    fn drop(&mut self) {
        self.shutdown();
    }
}
