//! Logique pure de l'enregistrement vidéo : construction des arguments ffmpeg
//! (capture par OS, export trim/crop/vitesse, GIF), machine à états du
//! raccourci, parseurs de sortie ffmpeg, et pastille "REC" du tray.
//! Aucun I/O ici — le côté Tauri ne fait que spawner ffmpeg avec ces args.

/// Rectangle en pixels, relatif à la sortie capturée (moniteur ou frame vidéo).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct RecordOptions {
    pub fps: u32,
    pub cursor: bool,
}

/// H.264/yuv420p exige des dimensions paires : arrondit vers le bas, minimum 2.
pub fn even_dim(v: u32) -> u32 {
    (v & !1).max(2)
}

/// Réglages d'encodage communs (enregistrement et export MP4).
fn h264_encode_args() -> Vec<String> {
    [
        "-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Args ffmpeg pour capturer une région d'un moniteur Windows via ddagrab
/// (Desktop Duplication — même API que WGC, pas de fenêtres noires GPU).
/// `output_idx` est l'index DXGI de la sortie ; offsets relatifs à cette sortie.
pub fn windows_capture_args(
    output_idx: u32,
    region: Region,
    opts: &RecordOptions,
    out_path: &str,
) -> Vec<String> {
    let (w, h) = (even_dim(region.width), even_dim(region.height));
    let filter = format!(
        "ddagrab=output_idx={}:framerate={}:offset_x={}:offset_y={}:video_size={}x{}:draw_mouse={},hwdownload,format=bgra",
        output_idx, opts.fps, region.x, region.y, w, h, if opts.cursor { 1 } else { 0 }
    );
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-init_hw_device".into(),
        "d3d11va".into(),
        "-filter_complex".into(),
        filter,
    ];
    args.extend(h264_encode_args());
    // MP4 fragmenté : lisible même si le process meurt en cours d'enregistrement.
    args.extend(["-movflags".into(), "+frag_keyframe+empty_moov".into()]);
    args.push(out_path.into());
    args
}

/// Args ffmpeg pour capturer un écran macOS via avfoundation puis recadrer la
/// région (avfoundation capture l'écran entier, en pixels physiques/Retina).
/// `device_idx` est l'index du périphérique "Capture screen N" d'avfoundation.
pub fn macos_capture_args(
    device_idx: u32,
    region: Region,
    opts: &RecordOptions,
    out_path: &str,
) -> Vec<String> {
    let (w, h) = (even_dim(region.width), even_dim(region.height));
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-f".into(),
        "avfoundation".into(),
        "-capture_cursor".into(),
        if opts.cursor { "1" } else { "0" }.into(),
        "-framerate".into(),
        opts.fps.to_string(),
        "-i".into(),
        format!("{device_idx}:none"),
        "-vf".into(),
        format!("crop={}:{}:{}:{}", w, h, region.x, region.y),
    ];
    args.extend(h264_encode_args());
    args.extend(["-movflags".into(), "+frag_keyframe+empty_moov".into()]);
    args.push(out_path.into());
    args
}

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub trim_start: f64,
    pub trim_end: f64,
    pub crop: Option<Region>,
    pub speed: f64, // 1.0, 2.0, 4.0
    pub gif: bool,
    pub preset: ExportPreset,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExportPreset {
    Mp4High,
    Mp4Small,
    GifSmooth,
    GifSmall,
}

impl ExportPreset {
    pub fn from_str(s: &str, gif: bool) -> Self {
        match s {
            "mp4-small" => ExportPreset::Mp4Small,
            "gif-smooth" => ExportPreset::GifSmooth,
            "gif-small" => ExportPreset::GifSmall,
            "mp4-high" => ExportPreset::Mp4High,
            _ if gif => ExportPreset::GifSmooth,
            _ => ExportPreset::Mp4High,
        }
    }
}

fn h264_export_args(preset: ExportPreset) -> Vec<String> {
    let crf = match preset {
        ExportPreset::Mp4Small => "28",
        _ => "20",
    };
    [
        "-c:v", "libx264", "-preset", "veryfast", "-crf", crf, "-pix_fmt", "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn gif_fps(preset: ExportPreset) -> u32 {
    match preset {
        ExportPreset::GifSmall => 10,
        _ => 20,
    }
}

/// Chaîne de filtres commune (crop puis vitesse), vide si aucun des deux.
fn filter_prefix(opts: &ExportOptions) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(c) = opts.crop {
        parts.push(format!(
            "crop={}:{}:{}:{}",
            even_dim(c.width),
            even_dim(c.height),
            c.x,
            c.y
        ));
    }
    if opts.speed != 1.0 {
        parts.push(format!("setpts=PTS/{}", opts.speed));
    }
    parts
}

/// Args ffmpeg pour l'export du mini-éditeur : trim (-ss/-to), crop, vitesse,
/// sortie MP4 (+faststart) ou GIF (palette deux passes en un seul appel).
pub fn export_args(input: &str, opts: &ExportOptions, output: &str) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-ss".into(),
        opts.trim_start.to_string(),
        "-to".into(),
        opts.trim_end.to_string(),
        "-i".into(),
        input.into(),
    ];
    let prefix = filter_prefix(opts);
    if opts.gif {
        let mut chain = prefix;
        chain.push(format!("fps={}", gif_fps(opts.preset)));
        if opts.preset == ExportPreset::GifSmall {
            chain.push("scale='min(960,iw)':-2".into());
        }
        let filter = format!(
            "{},split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer",
            chain.join(",")
        );
        args.extend(["-filter_complex".into(), filter]);
    } else {
        if !prefix.is_empty() {
            args.extend(["-vf".into(), prefix.join(",")]);
        }
        args.extend(h264_export_args(opts.preset));
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    args
}

/// Décision du raccourci d'enregistrement selon l'état courant.
/// `busy` = sélection ou export déjà en cours (on ignore le déclenchement) ;
/// un enregistrement actif peut TOUJOURS être arrêté.
#[derive(Debug)]
pub enum ShortcutAction {
    StartSelection,
    Stop,
    Ignore,
}

pub fn on_record_shortcut(recording: bool, busy: bool) -> ShortcutAction {
    if recording {
        ShortcutAction::Stop
    } else if busy {
        ShortcutAction::Ignore
    } else {
        ShortcutAction::StartSelection
    }
}

/// Parse la liste de périphériques avfoundation (stderr de
/// `ffmpeg -f avfoundation -list_devices true -i ""`) et renvoie les écrans :
/// (index de périphérique, numéro d'écran) pour chaque ligne "[N] Capture screen M".
pub fn parse_avfoundation_screens(stderr: &str) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        if let Some(pos) = line.find("Capture screen ") {
            let screen_no = line[pos + "Capture screen ".len()..]
                .trim()
                .parse::<u32>()
                .ok();
            // L'index de périphérique est le dernier "[N]" avant le nom.
            let device_idx = line[..pos]
                .rfind('[')
                .and_then(|i| line[i + 1..].split(']').next())
                .and_then(|s| s.parse::<u32>().ok());
            if let (Some(d), Some(s)) = (device_idx, screen_no) {
                out.push((d, s));
            }
        }
    }
    out
}

/// Extrait le temps de progression (en secondes) d'une ligne stderr ffmpeg
/// ("... time=00:01:02.50 ...").
pub fn parse_ffmpeg_time(line: &str) -> Option<f64> {
    let idx = line.find("time=")?;
    let rest = &line[idx + 5..];
    let token = rest.split_whitespace().next()?;
    let mut parts = token.split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// Pastille rouge (disque plein sur fond transparent) pour l'icône de tray en
/// enregistrement. RGBA, `size`×`size`.
pub fn recording_badge_rgba(size: u32) -> Vec<u8> {
    let mut px = vec![0u8; (size * size * 4) as usize];
    let c = (size as f32 - 1.0) / 2.0;
    let r = size as f32 * 0.42;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - c;
            let dy = y as f32 - c;
            if (dx * dx + dy * dy).sqrt() <= r {
                let i = ((y * size + x) * 4) as usize;
                px[i] = 220;
                px[i + 1] = 40;
                px[i + 2] = 40;
                px[i + 3] = 255;
            }
        }
    }
    px
}

/// Contenu du fichier-liste pour le démultiplexeur `concat` de ffmpeg, à partir
/// des chemins de segments. Chaque ligne `file '<chemin>'` ; les apostrophes du
/// chemin sont échappées selon la convention concat (`'` → `'\''`).
pub fn concat_list_contents(paths: &[String]) -> String {
    let mut out = String::new();
    for p in paths {
        let escaped = p.replace('\'', "'\\''");
        out.push_str(&format!("file '{escaped}'\n"));
    }
    out
}

/// Args ffmpeg pour concaténer sans réencodage les segments d'un enregistrement
/// (pause/reprise) via le démultiplexeur `concat` : `-c copy`, sortie faststart.
pub fn concat_args(list_path: &str, out_path: &str) -> Vec<String> {
    vec![
        "-y".into(),
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list_path.into(),
        "-c".into(),
        "copy".into(),
        "-movflags".into(),
        "+faststart".into(),
        out_path.into(),
    ]
}

/// Métadonnées extraites de stderr, sans dépendre de ffprobe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub duration: Option<f64>,
}

pub fn parse_video_info(stderr: &str) -> Option<VideoInfo> {
    let duration = stderr.lines().find_map(|line| {
        let value = line.split_once("Duration:")?.1.split(',').next()?.trim();
        let mut parts = value.split(':');
        let hours: f64 = parts.next()?.parse().ok()?;
        let minutes: f64 = parts.next()?.parse().ok()?;
        let seconds: f64 = parts.next()?.parse().ok()?;
        let seconds = hours * 3600.0 + minutes * 60.0 + seconds;
        (seconds.is_finite() && seconds > 0.0).then_some(seconds)
    });
    let (width, height) = stderr
        .lines()
        .filter(|line| line.contains("Video:"))
        .find_map(|line| {
            line.split(|c: char| c.is_whitespace() || c == ',')
                .find_map(|token| {
                    let (w, h) = token.split_once('x')?;
                    let w: u32 = w.parse().ok()?;
                    let h: u32 = h.parse().ok()?;
                    (w > 0 && h > 0).then_some((w, h))
                })
        })?;
    Some(VideoInfo {
        width,
        height,
        duration,
    })
}

/// Dernier timestamp de décodage, utile pour les MP4 fragmentés sans durée.
pub fn last_ffmpeg_time(stderr: &str) -> Option<f64> {
    stderr
        .split(['\r', '\n'])
        .filter_map(parse_ffmpeg_time)
        .filter(|time| time.is_finite() && *time >= 0.0)
        .last()
}

pub const MIN_TRIM_GAP: f64 = 0.1;

/// Même règle que clampTrim : en cas de croisement, la borne basse gagne.
pub fn clamp_trim(start: f64, end: f64, duration: f64) -> (f64, f64) {
    let duration = duration.max(0.0);
    let mut s = start.clamp(0.0, duration);
    let mut e = end.clamp(0.0, duration);
    if e < s + MIN_TRIM_GAP {
        s = s.min(e);
        e = duration.min(s + MIN_TRIM_GAP);
        if e < s + MIN_TRIM_GAP {
            s = (e - MIN_TRIM_GAP).max(0.0);
        }
    }
    (s, e)
}

pub fn effective_duration(start: f64, end: f64, speed: f64) -> f64 {
    (end - start) / speed
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayCrop {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Borne le rectangle d'affichage avant conversion, avec arrondi inférieur.
pub fn clamp_crop(rect: DisplayCrop, width: f64, height: f64) -> DisplayCrop {
    let x = rect.x.floor().clamp(0.0, width);
    let y = rect.y.floor().clamp(0.0, height);
    DisplayCrop {
        x,
        y,
        width: rect.width.floor().max(0.0).min(width - x),
        height: rect.height.floor().max(0.0).min(height - y),
    }
}

pub fn display_to_video(
    rect: DisplayCrop,
    display_w: f64,
    display_h: f64,
    video_w: u32,
    video_h: u32,
) -> Region {
    let fx = f64::from(video_w) / display_w;
    let fy = f64::from(video_h) / display_h;
    let crop = clamp_crop(
        DisplayCrop {
            x: rect.x * fx,
            y: rect.y * fy,
            width: rect.width * fx,
            height: rect.height * fy,
        },
        f64::from(video_w),
        f64::from(video_h),
    );
    Region {
        x: crop.x as u32,
        y: crop.y as u32,
        width: crop.width as u32,
        height: crop.height as u32,
    }
}

#[cfg(test)]
mod editor_tests {
    use super::*;

    #[test]
    fn trim_keeps_minimum_gap() {
        assert_eq!(clamp_trim(5.0, 4.0, 10.0), (4.0, 4.1));
        assert_eq!(clamp_trim(-1.0, 20.0, 10.0), (0.0, 10.0));
        assert_eq!(clamp_trim(2.0, 8.0, 10.0), (2.0, 8.0));
        assert_eq!(clamp_trim(10.0, 10.0, 10.0), (9.9, 10.0));
        assert_eq!(clamp_trim(0.0, 0.05, 0.05), (0.0, 0.05));
        assert_eq!(effective_duration(2.0, 8.0, 2.0), 3.0);
        assert_eq!(effective_duration(0.0, 10.0, 1.0), 10.0);
    }

    #[test]
    fn crop_bounds_and_floors() {
        assert_eq!(
            clamp_crop(
                DisplayCrop {
                    x: -5.0,
                    y: 2.7,
                    width: 2000.0,
                    height: 50.0
                },
                640.0,
                480.0
            ),
            DisplayCrop {
                x: 0.0,
                y: 2.0,
                width: 640.0,
                height: 50.0
            }
        );
        assert_eq!(
            display_to_video(
                DisplayCrop {
                    x: 10.0,
                    y: 20.0,
                    width: 100.0,
                    height: 50.0
                },
                640.0,
                360.0,
                1280,
                720
            ),
            Region {
                x: 20,
                y: 40,
                width: 200,
                height: 100
            }
        );
        assert_eq!(
            clamp_crop(
                DisplayCrop {
                    x: 900.0,
                    y: 900.0,
                    width: 10.0,
                    height: 10.0
                },
                640.0,
                480.0
            ),
            DisplayCrop {
                x: 640.0,
                y: 480.0,
                width: 0.0,
                height: 0.0
            }
        );
    }

    #[test]
    fn probe_reads_video_not_audio_or_pixel_format() {
        let text = "Duration: 00:01:02.50, start: 0.000000, bitrate: 500 kb/s\nStream #0:0: Audio: aac, 48000 Hz\nStream #0:1: Video: h264 (High) (avc1 / 0x31637661), yuv420p(progressive), 1920x1080 [SAR 1:1 DAR 16:9], 30 fps";
        assert_eq!(
            parse_video_info(text),
            Some(VideoInfo {
                width: 1920,
                height: 1080,
                duration: Some(62.5)
            })
        );
        assert_eq!(
            parse_video_info(
                "Duration: N/A, bitrate: N/A\nStream #0:0: Video: h264, yuv420p, 640x480, 30 fps"
            ),
            Some(VideoInfo {
                width: 640,
                height: 480,
                duration: None
            })
        );
        assert_eq!(
            parse_video_info("Invalid data found when processing input"),
            None
        );
        assert_eq!(
            last_ffmpeg_time("frame=1 time=00:00:00.10\rframe=2 time=00:00:04.25\n"),
            Some(4.25)
        );
        assert_eq!(last_ffmpeg_time("time=N/A"), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn even_dim_rounds_down_to_even_with_min_2() {
        assert_eq!(even_dim(7), 6);
        assert_eq!(even_dim(8), 8);
        assert_eq!(even_dim(1), 2);
        assert_eq!(even_dim(0), 2);
    }

    #[test]
    fn concat_list_escapes_quotes_and_lists_each_file() {
        let paths = vec![
            "/tmp/rec/seg0.mp4".to_string(),
            "/tmp/o'brien/seg1.mp4".to_string(),
        ];
        assert_eq!(
            concat_list_contents(&paths),
            "file '/tmp/rec/seg0.mp4'\nfile '/tmp/o'\\''brien/seg1.mp4'\n"
        );
    }

    #[test]
    fn concat_args_copy_without_reencode() {
        let args = concat_args("list.txt", "out.mp4");
        let joined = args.join(" ");
        assert!(joined.contains("-f concat"));
        assert!(joined.contains("-safe 0"));
        assert!(joined.contains("-i list.txt"));
        assert!(joined.contains("-c copy"));
        assert!(joined.contains("+faststart"));
        assert_eq!(args.last().unwrap(), "out.mp4");
    }

    #[test]
    fn windows_args_use_ddagrab_with_region_fps_and_cursor() {
        let args = windows_capture_args(
            1,
            Region {
                x: 10,
                y: 20,
                width: 641,
                height: 480,
            },
            &RecordOptions {
                fps: 30,
                cursor: true,
            },
            "out.mp4",
        );
        let joined = args.join(" ");
        assert!(joined.contains("ddagrab=output_idx=1"));
        assert!(joined.contains("framerate=30"));
        assert!(joined.contains("offset_x=10:offset_y=20"));
        // 641 arrondi pair → 640
        assert!(joined.contains("video_size=640x480"));
        assert!(joined.contains("draw_mouse=1"));
        assert!(joined.contains("hwdownload"));
        assert!(joined.contains("-movflags"));
        assert!(joined.contains("+frag_keyframe+empty_moov"));
        assert_eq!(args.last().unwrap(), "out.mp4");
    }

    #[test]
    fn windows_args_hide_cursor_when_disabled() {
        let args = windows_capture_args(
            0,
            Region {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            &RecordOptions {
                fps: 30,
                cursor: false,
            },
            "o.mp4",
        );
        assert!(args.join(" ").contains("draw_mouse=0"));
    }

    #[test]
    fn macos_args_use_avfoundation_and_pixel_crop() {
        let args = macos_capture_args(
            3,
            Region {
                x: 4,
                y: 8,
                width: 300,
                height: 201,
            },
            &RecordOptions {
                fps: 30,
                cursor: true,
            },
            "out.mp4",
        );
        let joined = args.join(" ");
        assert!(joined.contains("-f avfoundation"));
        assert!(joined.contains("-capture_cursor 1"));
        assert!(joined.contains("-framerate 30"));
        assert!(joined.contains("-i 3:none"));
        // crop=w:h:x:y en pixels, hauteur 201 arrondie → 200
        assert!(joined.contains("crop=300:200:4:8"));
        assert_eq!(args.last().unwrap(), "out.mp4");
    }

    #[test]
    fn export_args_mp4_with_trim_crop_and_speed() {
        let opts = ExportOptions {
            trim_start: 1.5,
            trim_end: 9.0,
            crop: Some(Region {
                x: 2,
                y: 4,
                width: 500,
                height: 301,
            }),
            speed: 2.0,
            gif: false,
            preset: ExportPreset::Mp4High,
        };
        let args = export_args("in.mp4", &opts, "out.mp4");
        let joined = args.join(" ");
        assert!(joined.contains("-ss 1.5"));
        assert!(joined.contains("-to 9"));
        assert!(joined.contains("crop=500:300:2:4"));
        assert!(joined.contains("setpts=PTS/2"));
        assert!(joined.contains("+faststart"));
        assert!(!joined.contains("palettegen"));
    }

    #[test]
    fn export_args_mp4_without_filters_omits_vf() {
        let opts = ExportOptions {
            trim_start: 0.0,
            trim_end: 5.0,
            crop: None,
            speed: 1.0,
            gif: false,
            preset: ExportPreset::Mp4High,
        };
        let args = export_args("in.mp4", &opts, "out.mp4");
        assert!(!args.contains(&"-vf".to_string()));
    }

    #[test]
    fn export_args_gif_uses_two_pass_palette_at_20fps() {
        let opts = ExportOptions {
            trim_start: 0.0,
            trim_end: 3.0,
            crop: None,
            speed: 1.0,
            gif: true,
            preset: ExportPreset::GifSmooth,
        };
        let args = export_args("in.mp4", &opts, "out.gif");
        let joined = args.join(" ");
        assert!(joined.contains("fps=20"));
        assert!(joined.contains("palettegen"));
        assert!(joined.contains("paletteuse"));
        assert_eq!(args.last().unwrap(), "out.gif");
    }

    #[test]
    fn export_args_small_gif_caps_size_and_fps() {
        let opts = ExportOptions {
            trim_start: 0.0,
            trim_end: 3.0,
            crop: None,
            speed: 1.0,
            gif: true,
            preset: ExportPreset::GifSmall,
        };
        let joined = export_args("in.mp4", &opts, "out.gif").join(" ");
        assert!(joined.contains("fps=10"));
        assert!(joined.contains("scale='min(960,iw)':-2"));
    }

    #[test]
    fn shortcut_action_follows_state() {
        assert!(matches!(
            on_record_shortcut(false, false),
            ShortcutAction::StartSelection
        ));
        assert!(matches!(
            on_record_shortcut(true, false),
            ShortcutAction::Stop
        ));
        assert!(matches!(
            on_record_shortcut(false, true),
            ShortcutAction::Ignore
        ));
        // "recording" prime sur "busy" : on peut toujours arrêter.
        assert!(matches!(
            on_record_shortcut(true, true),
            ShortcutAction::Stop
        ));
    }

    #[test]
    fn parses_avfoundation_screen_devices() {
        let stderr = r#"[AVFoundation indev @ 0x7f8] AVFoundation video devices:
[AVFoundation indev @ 0x7f8] [0] FaceTime HD Camera
[AVFoundation indev @ 0x7f8] [1] Capture screen 0
[AVFoundation indev @ 0x7f8] [2] Capture screen 1
[AVFoundation indev @ 0x7f8] AVFoundation audio devices:
[AVFoundation indev @ 0x7f8] [0] MacBook Pro Microphone"#;
        assert_eq!(parse_avfoundation_screens(stderr), vec![(1, 0), (2, 1)]);
    }

    #[test]
    fn parses_ffmpeg_progress_time() {
        let line =
            "frame=  120 fps= 30 q=23.0 size=     512KiB time=00:01:02.50 bitrate= 900kbits/s";
        assert_eq!(parse_ffmpeg_time(line), Some(62.5));
        assert_eq!(parse_ffmpeg_time("no time here"), None);
    }

    #[test]
    fn recording_badge_is_rgba_square_with_red_center() {
        let size = 16u32;
        let px = recording_badge_rgba(size);
        assert_eq!(px.len(), (size * size * 4) as usize);
        // Centre : rouge opaque. Coin : transparent.
        let c = ((8 * size + 8) * 4) as usize;
        assert_eq!(&px[c..c + 4], &[220, 40, 40, 255]);
        assert_eq!(px[3], 0);
    }
}
