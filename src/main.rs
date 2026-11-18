// rclip: clips de juegos desde la bandeja del sistema.
// La captura y el encode los hace ffmpeg.exe (ddagrab + encoder por hardware);
// este proceso solo maneja la bandeja, las teclas, el audio del sistema y el watchdog.
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

/// Lógica pura (sin Windows): config, selección de segmentos, nombres de teclas, argumentos de ffmpeg.
mod logic {
    pub const DURATIONS: [u32; 5] = [15, 30, 60, 120, 300];
    pub const MOD_ALT: u32 = 1;
    pub const MOD_CONTROL: u32 = 2;
    pub const MOD_SHIFT: u32 = 4;
    pub const VK_F9: u32 = 0x78;
    pub const VK_F10: u32 = 0x79;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Hotkey {
        pub vk: u32,
        pub mods: u32,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct Config {
        pub secs: u32,
        pub fps: u32,
        pub clip: Hotkey,
        pub shot: Hotkey,
        pub folder: String,
        pub encoder: usize,
    }

    impl Config {
        pub fn defaults(folder: String) -> Self {
            Config {
                secs: 30,
                fps: 30,
                clip: Hotkey { vk: VK_F9, mods: 0 },
                shot: Hotkey { vk: VK_F10, mods: 0 },
                folder,
                encoder: 0,
            }
        }

        pub fn parse(text: &str, default_folder: String) -> Self {
            let mut c = Self::defaults(default_folder);
            for line in text.lines() {
                let Some((k, v)) = line.split_once('=') else { continue };
                let (k, v) = (k.trim(), v.trim());
                let n = v.parse::<u32>().ok();
                match (k, n) {
                    ("seconds", Some(n)) if (5..=600).contains(&n) => c.secs = n,
                    ("fps", Some(n)) if n == 30 || n == 60 => c.fps = n,
                    ("clip_key", Some(n)) if (1..=254).contains(&n) => c.clip.vk = n,
                    ("clip_mods", Some(n)) => c.clip.mods = n & 7,
                    ("shot_key", Some(n)) if (1..=254).contains(&n) => c.shot.vk = n,
                    ("shot_mods", Some(n)) => c.shot.mods = n & 7,
                    ("encoder", Some(n)) if (n as usize) < ENCODERS.len() => c.encoder = n as usize,
                    ("folder", _) if !v.is_empty() => c.folder = v.to_string(),
                    _ => {}
                }
            }
            if c.clip == c.shot {
                c.clip = Hotkey { vk: VK_F9, mods: 0 };
                c.shot = Hotkey { vk: VK_F10, mods: 0 };
            }
            c
        }

        pub fn serialize(&self) -> String {
            format!(
                "seconds={}\nfps={}\nclip_key={}\nclip_mods={}\nshot_key={}\nshot_mods={}\nfolder={}\nencoder={}\n",
                self.secs, self.fps, self.clip.vk, self.clip.mods, self.shot.vk, self.shot.mods, self.folder, self.encoder
            )
        }
    }

    /// "Ctrl+Alt+Shift+F9". `name` traduce teclas que no son F1..F24.
    pub fn fmt_hotkey(h: Hotkey, name: impl Fn(u32) -> String) -> String {
        let mut s = String::new();
        if h.mods & MOD_CONTROL != 0 {
            s.push_str("Ctrl+");
        }
        if h.mods & MOD_ALT != 0 {
            s.push_str("Alt+");
        }
        if h.mods & MOD_SHIFT != 0 {
            s.push_str("Shift+");
        }
        if (0x70..=0x87).contains(&h.vk) {
            s.push_str(&format!("F{}", h.vk - 0x6F));
        } else {
            let n = name(h.vk);
            if n.is_empty() {
                s.push_str(&format!("Tecla {}", h.vk));
            } else {
                s.push_str(&n);
            }
        }
        s
    }

    /// Segmentos de 2 s a juntar: los completos que cubren `secs` + el que se está escribiendo.
    pub fn segments_needed(secs: u32) -> usize {
        secs.div_ceil(2) as usize + 1
    }

    /// Cantidad de archivos que rota el muxer (margen para que no pise lo que estamos copiando).
    pub fn segment_wrap(secs: u32) -> u32 {
        secs / 2 + 5
    }

    /// files: (ruta, mtime). Devuelve los más nuevos, en orden cronológico.
    pub fn select_segments(mut files: Vec<(String, u128)>, secs: u32) -> Vec<String> {
        files.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        let skip = files.len().saturating_sub(segments_needed(secs));
        files.into_iter().skip(skip).map(|f| f.0).collect()
    }

    pub struct Enc {
        pub label: &'static str,
        pub vf: &'static str,
        pub codec: &'static str,
        pub opts: &'static [&'static str],
    }

    const NV: &[&str] = &["-preset", "p1", "-tune", "ll", "-rc", "cbr"];
    const AMF: &[&str] = &["-usage", "lowlatency", "-quality", "speed", "-rc", "cbr"];
    const QSV: &[&str] = &["-preset", "veryfast"];
    const GPU: &str = "scale_d3d11=width=-2:height=720:format=nv12";
    const CPU: &str = "hwdownload,format=bgra,scale=-2:720:flags=fast_bilinear,format=nv12";

    /// Orden de prueba: todo en GPU primero, después escalado en CPU, último x264.
    pub const ENCODERS: [Enc; 7] = [
        Enc { label: "NVENC", vf: GPU, codec: "h264_nvenc", opts: NV },
        Enc { label: "AMF", vf: GPU, codec: "h264_amf", opts: AMF },
        Enc {
            label: "QSV",
            vf: "scale_d3d11=width=-2:height=720:format=nv12,hwmap=derive_device=qsv,format=qsv",
            codec: "h264_qsv",
            opts: QSV,
        },
        Enc { label: "NVENC (escala CPU)", vf: CPU, codec: "h264_nvenc", opts: NV },
        Enc { label: "AMF (escala CPU)", vf: CPU, codec: "h264_amf", opts: AMF },
        Enc { label: "QSV (escala CPU)", vf: CPU, codec: "h264_qsv", opts: QSV },
        Enc {
            label: "x264 (CPU)",
            vf: CPU,
            codec: "libx264",
            opts: &["-preset", "ultrafast", "-tune", "zerolatency"],
        },
    ];

    pub fn bitrate_mbit(fps: u32) -> u32 {
        if fps >= 60 {
            9
        } else {
            6
        }
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    /// Grabación continua a segmentos .ts en `dir`. audio = (Hz, canales) de PCM f32 por stdin.
    pub fn record_args(enc: usize, fps: u32, secs: u32, audio: Option<(u32, u16)>, dir: &str) -> Vec<String> {
        let e = &ENCODERS[enc];
        let br = bitrate_mbit(fps);
        let mut a = s(&["-hide_banner", "-loglevel", "warning", "-nostats"]);
        if audio.is_none() {
            a.push("-nostdin".into());
        }
        a.extend(s(&["-f", "lavfi", "-i"]));
        a.push(format!("ddagrab=framerate={fps}:draw_mouse=1"));
        if let Some((rate, ch)) = audio {
            a.extend(s(&["-thread_queue_size", "1024", "-f", "f32le", "-ar"]));
            a.push(rate.to_string());
            a.push("-ac".into());
            a.push(ch.to_string());
            a.extend(s(&["-i", "pipe:0", "-map", "0:v", "-map", "1:a"]));
        } else {
            a.extend(s(&["-map", "0:v"]));
        }
        a.extend(s(&["-vf", e.vf, "-c:v", e.codec]));
        a.extend(s(e.opts));
        a.extend([
            "-b:v".into(),
            format!("{br}M"),
            "-maxrate".into(),
            format!("{br}M"),
            "-bufsize".into(),
            format!("{}M", br * 2),
            "-g".into(),
            (fps * 2).to_string(),
            "-bf".into(),
            "0".into(),
        ]);
        if audio.is_some() {
            a.extend(s(&["-c:a", "aac", "-b:a", "160k"]));
        }
        a.extend(s(&["-f", "segment", "-segment_time", "2", "-segment_wrap"]));
        a.push(segment_wrap(secs).to_string());
        a.extend(s(&["-segment_format", "mpegts"]));
        a.push(format!("{dir}\\seg%03d.ts"));
        a
    }

    /// Junta segmentos sin recodificar (copia de stream).
    pub fn clip_args(segs: &[String], out: &str) -> Vec<String> {
        let mut a = s(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"]);
        a.push(format!("concat:{}", segs.join("|")));
        a.extend(s(&["-map", "0", "-c", "copy", "-movflags", "+faststart", out]));
        a
    }

    /// Captura de pantalla completa en PNG (descarta los primeros cuadros, que pueden salir negros).
    pub fn shot_args(out: &str) -> Vec<String> {
        s(&[
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "ddagrab=framerate=30:draw_mouse=0",
            "-vf",
            "select=gte(n\\,3),hwdownload,format=bgra",
            "-frames:v",
            "1",
            "-update",
            "1",
            "-compression_level",
            "3",
            out,
        ])
    }

    pub fn stamp(y: u16, mo: u16, d: u16, h: u16, mi: u16, sec: u16) -> String {
        format!("{y:04}-{mo:02}-{d:02}_{h:02}-{mi:02}-{sec:02}")
    }

    /// Ícono: punto rojo con borde suavizado, BGRA (alfa sin premultiplicar).
    pub fn dot_pixels(size: usize) -> Vec<u32> {
        let c = size as f32 / 2.0;
        let r = size as f32 * 0.42;
        let mut px = Vec::with_capacity(size * size);
        for y in 0..size {
            for x in 0..size {
                let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
                let a = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
                let a = (a * 255.0) as u32;
                px.push(if a == 0 { 0 } else { (a << 24) | 0x00E5_2B2B });
            }
        }
        px
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn config_roundtrip() {
            let mut c = Config::defaults("C:\\x".into());
            c.secs = 120;
            c.fps = 60;
            c.clip = Hotkey { vk: 0x41, mods: MOD_CONTROL | MOD_SHIFT };
            c.encoder = 3;
            c.folder = "D:\\Mis clips\\rclip".into();
            assert_eq!(Config::parse(&c.serialize(), "C:\\y".into()), c);
        }

        #[test]
        fn config_defaults_and_garbage() {
            let d = Config::defaults("F".into());
            assert_eq!(Config::parse("", "F".into()), d);
            let c = Config::parse("fps=45\nseconds=9999\nencoder=99\nclip_key=0\nbasura\n=\nfolder=\n", "F".into());
            assert_eq!(c, d);
            // misma tecla para las dos acciones -> vuelve a los defaults
            let c = Config::parse("clip_key=65\nshot_key=65\n", "F".into());
            assert_eq!((c.clip.vk, c.shot.vk), (VK_F9, VK_F10));
            let c = Config::parse(" seconds = 300 \r\nfps=60\r\n", "F".into());
            assert_eq!((c.secs, c.fps), (300, 60));
        }

        #[test]
        fn hotkey_names() {
            let n = |_| String::from("A");
            assert_eq!(fmt_hotkey(Hotkey { vk: VK_F9, mods: 0 }, n), "F9");
            assert_eq!(fmt_hotkey(Hotkey { vk: 0x87, mods: 0 }, n), "F24");
            assert_eq!(fmt_hotkey(Hotkey { vk: 0x41, mods: 7 }, n), "Ctrl+Alt+Shift+A");
            assert_eq!(fmt_hotkey(Hotkey { vk: 0x42, mods: 0 }, |_| String::new()), "Tecla 66");
        }

        #[test]
        fn segment_selection() {
            assert_eq!(segments_needed(30), 16);
            assert_eq!(segments_needed(15), 9);
            assert_eq!(segment_wrap(300), 155);
            // rotación: los nombres no siguen el orden temporal, el mtime sí
            let files = vec![
                ("seg002.ts".to_string(), 50),
                ("seg000.ts".to_string(), 40),
                ("seg001.ts".to_string(), 45),
                ("seg003.ts".to_string(), 10),
                ("seg004.ts".to_string(), 20),
            ];
            assert_eq!(select_segments(files.clone(), 4), vec!["seg000.ts", "seg001.ts", "seg002.ts"]);
            assert_eq!(select_segments(files, 600).len(), 5);
            assert!(select_segments(vec![], 30).is_empty());
        }

        #[test]
        fn record_args_gpu_with_audio() {
            let a = record_args(0, 60, 30, Some((48000, 2)), "C:\\T\\rclip_buf").join(" ");
            assert!(a.contains("-f lavfi -i ddagrab=framerate=60:draw_mouse=1"));
            assert!(a.contains("-thread_queue_size 1024 -f f32le -ar 48000 -ac 2 -i pipe:0 -map 0:v -map 1:a"));
            assert!(a.contains("-vf scale_d3d11=width=-2:height=720:format=nv12 -c:v h264_nvenc -preset p1 -tune ll -rc cbr"));
            assert!(a.contains("-b:v 9M -maxrate 9M -bufsize 18M -g 120 -bf 0"));
            assert!(a.contains("-c:a aac -b:a 160k"));
            assert!(a.ends_with("-f segment -segment_time 2 -segment_wrap 20 -segment_format mpegts C:\\T\\rclip_buf\\seg%03d.ts"));
            assert!(!a.contains("-nostdin"));
            assert!(!a.contains("reset_timestamps"));
        }

        #[test]
        fn record_args_cpu_no_audio() {
            let a = record_args(6, 30, 60, None, "D").join(" ");
            assert!(a.contains("-nostdin"));
            assert!(a.contains("-map 0:v -vf hwdownload,format=bgra,scale=-2:720"));
            assert!(a.contains("-c:v libx264 -preset ultrafast"));
            assert!(a.contains("-b:v 6M"));
            assert!(a.contains("-g 60"));
            assert!(!a.contains("pipe:0"));
            assert!(!a.contains("-c:a"));
            for i in 0..ENCODERS.len() {
                assert!(record_args(i, 30, 30, None, "D").iter().any(|x| x == ENCODERS[i].codec));
            }
        }

        #[test]
        fn clip_and_shot_args() {
            let a = clip_args(&["C:\\b\\seg001.ts".into(), "C:\\b\\seg002.ts".into()], "C:\\o\\clip.mp4");
            let j = a.join(" ");
            assert!(j.contains("-i concat:C:\\b\\seg001.ts|C:\\b\\seg002.ts -map 0 -c copy -movflags +faststart C:\\o\\clip.mp4"));
            let s = shot_args("x.png");
            assert!(s.contains(&"select=gte(n\\,3),hwdownload,format=bgra".to_string()));
            assert_eq!(s.last().unwrap(), "x.png");
        }

        #[test]
        fn stamp_and_icon() {
            assert_eq!(stamp(2026, 1, 2, 3, 4, 5), "2026-01-02_03-04-05");
            let px = dot_pixels(16);
            assert_eq!(px.len(), 256);
            assert_eq!(px[0], 0); // esquina transparente
            assert_eq!(px[8 * 16 + 8] >> 24, 255); // centro opaco
        }
    }
}

/// Prueba real con ffmpeg.exe (en CI): grabación a segmentos con audio por pipe, y clip por copia.
/// Sin GPU ni escritorio en CI: se reemplaza ddagrab por una fuente de prueba. Requiere RCLIP_FFMPEG.
#[cfg(all(test, windows))]
mod win_tests {
    use crate::logic;
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant, UNIX_EPOCH};

    #[test]
    fn segments_audio_pipe_and_clip() {
        let Ok(ff) = std::env::var("RCLIP_FFMPEG") else {
            eprintln!("RCLIP_FFMPEG no definido: se saltea");
            return;
        };
        let dir = std::env::temp_dir().join("rclip_test_buf");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut args = logic::record_args(6, 30, 6, Some((48000, 2)), &dir.to_string_lossy());
        for a in args.iter_mut() {
            if a.starts_with("ddagrab=") {
                *a = "testsrc2=size=1920x1080:rate=30,realtime".into();
            } else if a.starts_with("hwdownload,format=bgra,") {
                *a = a.replacen("hwdownload,format=bgra,", "", 1);
            }
        }
        let (r, mut w) = crate::audio::small_pipe().expect("pipe");
        let mut child = Command::new(&ff).args(&args).stdin(Stdio::from(r)).stdout(Stdio::null()).spawn().unwrap();
        let t0 = Instant::now();
        let mut frames = 0u64;
        while t0.elapsed() < Duration::from_secs(7) {
            let mut buf = Vec::with_capacity(4800 * 2 * 4);
            for i in 0..4800u64 {
                let v = ((frames + i) as f32 * 440.0 * std::f32::consts::TAU / 48000.0).sin() * 0.3;
                buf.extend_from_slice(&v.to_le_bytes());
                buf.extend_from_slice(&v.to_le_bytes());
            }
            w.write_all(&buf).expect("ffmpeg dejó de leer el audio");
            frames += 4800;
            let due = Duration::from_micros(frames * 1_000_000 / 48000);
            if let Some(d) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(d);
            }
        }
        assert!(child.try_wait().unwrap().is_none(), "ffmpeg terminó antes de tiempo");
        let mut files = Vec::new();
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            let t = std::fs::metadata(&p).unwrap().modified().unwrap().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            files.push((p.to_string_lossy().into_owned(), t));
        }
        // el clip se arma con ffmpeg todavía grabando (como en uso real)
        let segs = logic::select_segments(files, 6);
        eprintln!("segmentos: {segs:?}");
        assert!(segs.len() >= 3, "pocos segmentos: {segs:?}");
        let out = std::env::temp_dir().join("rclip_test_clip.mp4").to_string_lossy().into_owned();
        let st = Command::new(&ff).args(logic::clip_args(&segs, &out)).status().unwrap();
        let _ = child.kill();
        let _ = child.wait();
        assert!(st.success(), "falló el clip");
        let probe = std::path::Path::new(&ff).with_file_name("ffprobe.exe");
        let o = Command::new(probe)
            .args(["-v", "error", "-show_entries", "stream=codec_name,width,height,duration", "-of", "default=nw=1", &out])
            .output()
            .unwrap();
        let info = String::from_utf8_lossy(&o.stdout).to_string();
        eprintln!("{info}");
        assert!(info.contains("codec_name=h264") && info.contains("codec_name=aac"), "{info}");
        assert!(info.contains("width=1280") && info.contains("height=720"), "{info}");
        let durs: Vec<f64> = info.lines().filter_map(|l| l.strip_prefix("duration=")).filter_map(|v| v.parse().ok()).collect();
        assert_eq!(durs.len(), 2, "{info}");
        assert!(durs.iter().all(|d| (5.0..=9.0).contains(d)), "duración rara: {durs:?}");
        assert!((durs[0] - durs[1]).abs() < 0.5, "audio y video desparejos: {durs:?}");
    }
}

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("rclip solo funciona en Windows");
}

#[cfg(windows)]
mod app {
    use crate::logic::{self, Config, Hotkey, DURATIONS, ENCODERS};
    use std::cell::RefCell;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
    use std::time::{Duration, Instant, UNIX_EPOCH};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::JobObjects::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::ProcessStatus::K32EmptyWorkingSet;
    use windows::Win32::System::Registry::*;
    use windows::Win32::System::SystemInformation::GetLocalTime;
    use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess};
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::Shell::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    const WM_TRAY: u32 = WM_APP + 1;
    const WM_DONE: u32 = WM_APP + 2;
    const WM_BIND: u32 = WM_APP + 3;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const BELOW_NORMAL: u32 = 0x0000_4000;
    const TIMER_WATCHDOG: usize = 1;
    const TIMER_BIND: usize = 2;
    const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const QUICK_DEATH: Duration = Duration::from_secs(5);

    static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
    static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
    static JOB: AtomicIsize = AtomicIsize::new(0);

    thread_local! {
        static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    }

    struct Rec {
        child: Option<Child>,
        enc: usize,
        started: Instant,
        good: bool,
        tried: usize,
        fails: u32,
        next_try: Option<Instant>,
        gave_up: bool,
        audio: bool,
    }

    struct App {
        hwnd: HWND,
        cfg: Config,
        cfg_path: PathBuf,
        ffmpeg: PathBuf,
        buf_dir: PathBuf,
        log_path: PathBuf,
        icon: HICON,
        rec: Rec,
        binding: u8,
        hook: Option<HHOOK>,
        trimmed: bool,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn copy_wide(dst: &mut [u16], s: &str) {
        let n = dst.len() - 1;
        let mut i = 0;
        for c in s.encode_utf16().take(n) {
            dst[i] = c;
            i += 1;
        }
        dst[i] = 0;
    }

    fn env_path(var: &str, fallback: &str) -> PathBuf {
        std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(fallback))
    }

    fn log_line(path: &Path, msg: &str) {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let t = now_stamp();
            let _ = writeln!(f, "[rclip {t}] {msg}");
        }
    }

    fn now_stamp() -> String {
        let t = unsafe { GetLocalTime() };
        logic::stamp(t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
    }

    fn log_stdio(path: &Path) -> Stdio {
        match OpenOptions::new().create(true).append(true).open(path) {
            Ok(f) => Stdio::from(f),
            Err(_) => Stdio::null(),
        }
    }

    /// Lanza un proceso sin ventana y lo mete en el Job (muere si rclip muere).
    fn spawn_hidden(cmd: &mut Command, flags: u32) -> std::io::Result<Child> {
        let child = cmd.creation_flags(CREATE_NO_WINDOW | flags).spawn()?;
        let job = JOB.load(Ordering::Relaxed);
        if job != 0 {
            unsafe {
                let _ = AssignProcessToJobObject(HANDLE(job as _), HANDLE(child.as_raw_handle() as _));
            }
        }
        Ok(child)
    }

    fn post_text(text: String) {
        let hwnd = HWND(MAIN_HWND.load(Ordering::Relaxed) as _);
        let p = Box::into_raw(Box::new(text));
        unsafe {
            if PostMessageW(Some(hwnd), WM_DONE, WPARAM(0), LPARAM(p as isize)).is_err() {
                drop(Box::from_raw(p));
            }
        }
    }

    fn key_name(vk: u32) -> String {
        unsafe {
            let sc = MapVirtualKeyW(vk, MAPVK_VK_TO_VSC);
            let ext = matches!(vk, 0x21..=0x2E | 0x5B | 0x5C | 0x6F | 0x90 | 0xA3 | 0xA5);
            let lp = ((sc as i32) << 16) | if ext { 1 << 24 } else { 0 };
            let mut buf = [0u16; 64];
            let n = GetKeyNameTextW(lp, &mut buf);
            String::from_utf16_lossy(&buf[..n.max(0) as usize])
        }
    }

    fn hk(h: Hotkey) -> String {
        logic::fmt_hotkey(h, key_name)
    }

    impl App {
        fn nid(&self) -> NOTIFYICONDATAW {
            NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: 1,
                ..Default::default()
            }
        }

        fn tip(&self) -> String {
            let state = if self.rec.child.is_some() && self.rec.good {
                format!("grabando ({})", ENCODERS[self.rec.enc].label)
            } else if self.rec.gave_up {
                "sin grabar (error)".to_string()
            } else {
                "iniciando…".to_string()
            };
            let audio = if self.rec.child.is_some() && !self.rec.audio { ", sin audio" } else { "" };
            format!("rclip – {state}{audio}\n720p {} fps · últimos {} s", self.cfg.fps, self.cfg.secs)
        }

        fn add_icon(&self) {
            let mut n = self.nid();
            n.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            n.uCallbackMessage = WM_TRAY;
            n.hIcon = self.icon;
            copy_wide(&mut n.szTip, &self.tip());
            unsafe {
                let _ = Shell_NotifyIconW(NIM_ADD, &n);
            }
        }

        fn update_tip(&self) {
            let mut n = self.nid();
            n.uFlags = NIF_TIP;
            copy_wide(&mut n.szTip, &self.tip());
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
            }
        }

        fn notify(&self, msg: &str, warn: bool) {
            let mut n = self.nid();
            n.uFlags = NIF_INFO;
            copy_wide(&mut n.szInfoTitle, "rclip");
            copy_wide(&mut n.szInfo, msg);
            n.dwInfoFlags = (if warn { NIIF_WARNING } else { NIIF_INFO }) | NIIF_NOSOUND | NIIF_RESPECT_QUIET_TIME;
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
            }
        }

        fn save_cfg(&self) {
            if let Some(dir) = self.cfg_path.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = fs::write(&self.cfg_path, self.cfg.serialize());
        }

        fn register_keys(&self) {
            let mut bad = Vec::new();
            for (id, h) in [(1, self.cfg.clip), (2, self.cfg.shot)] {
                unsafe {
                    let _ = UnregisterHotKey(Some(self.hwnd), id);
                    let mods = HOT_KEY_MODIFIERS(h.mods | MOD_NOREPEAT.0);
                    if RegisterHotKey(Some(self.hwnd), id, mods, h.vk).is_err() {
                        bad.push(hk(h));
                    }
                }
            }
            if !bad.is_empty() {
                self.notify(&format!("No pude usar la tecla {} (¿la usa otro programa?). Cambiala desde el menú.", bad.join(" / ")), true);
            }
        }

        // ---------- grabación ----------

        fn clear_buf(&self) {
            if let Ok(rd) = fs::read_dir(&self.buf_dir) {
                for e in rd.flatten() {
                    let _ = fs::remove_file(e.path());
                }
            }
            let _ = fs::create_dir_all(&self.buf_dir);
        }

        fn stop_rec(&mut self) {
            if let Some(mut c) = self.rec.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }

        fn spawn_rec(&mut self) {
            self.clear_buf();
            let enc = self.rec.enc;
            // audio: captura WASAPI + pipe chico hacia el stdin de ffmpeg; si algo falla, solo video
            let audio = match crate::audio::start() {
                Some(a) => crate::audio::small_pipe().map(|p| (a, p)),
                None => None,
            };
            let fmt = audio.as_ref().map(|(a, _)| (a.rate, a.channels));
            let dir = self.buf_dir.to_string_lossy().into_owned();
            let args = logic::record_args(enc, self.cfg.fps, self.cfg.secs, fmt, &dir);
            let audio_desc = fmt.map_or("no".to_string(), |f| format!("{} Hz, {} canales", f.0, f.1));
            log_line(&self.log_path, &format!("iniciando {} {} fps (audio: {audio_desc})", ENCODERS[enc].label, self.cfg.fps));
            let mut cmd = Command::new(&self.ffmpeg);
            cmd.args(&args).stdout(Stdio::null()).stderr(log_stdio(&self.log_path));
            let feed = match audio {
                Some((a, (read_end, write_end))) => {
                    cmd.stdin(Stdio::from(read_end));
                    Some((a, write_end))
                }
                None => {
                    cmd.stdin(Stdio::null());
                    None
                }
            };
            self.rec.started = Instant::now();
            self.rec.next_try = None;
            self.rec.audio = feed.is_some();
            match spawn_hidden(&mut cmd, 0) {
                Ok(child) => {
                    self.rec.child = Some(child);
                    if let Some((a, w)) = feed {
                        a.feed(w);
                    }
                }
                Err(e) => {
                    log_line(&self.log_path, &format!("no se pudo lanzar ffmpeg: {e}"));
                    self.rec.child = None;
                }
            }
            drop(cmd); // cierra nuestra copia del extremo de lectura del pipe
            self.update_tip();
        }

        fn restart_rec(&mut self) {
            self.stop_rec();
            self.rec.tried = 0;
            self.rec.good = false;
            self.rec.gave_up = false;
            self.rec.fails = 0;
            self.spawn_rec();
        }

        /// Watchdog: cada 2 s.
        fn tick(&mut self) {
            let now = Instant::now();
            if let Some(c) = self.rec.child.as_mut() {
                let status = match c.try_wait() {
                    Ok(None) => {
                        if !self.rec.good && self.rec.started.elapsed() >= QUICK_DEATH {
                            self.rec.good = true;
                            self.rec.fails = 0;
                            if self.cfg.encoder != self.rec.enc {
                                self.cfg.encoder = self.rec.enc;
                                self.save_cfg();
                            }
                            log_line(&self.log_path, &format!("grabando con {}", ENCODERS[self.rec.enc].label));
                            self.update_tip();
                            if !self.trimmed {
                                self.trimmed = true;
                                unsafe {
                                    let _ = K32EmptyWorkingSet(GetCurrentProcess());
                                }
                            }
                        }
                        return;
                    }
                    Ok(Some(s)) => format!("{s}"),
                    Err(e) => format!("{e}"),
                };
                let ran = self.rec.started.elapsed();
                log_line(&self.log_path, &format!("ffmpeg terminó ({status}) después de {} s", ran.as_secs()));
                self.rec.child = None;
                if ran >= QUICK_DEATH {
                    self.rec.good = true;
                }
                if self.rec.good {
                    // encoder que ya anduvo: reintentar el mismo (cambio de resolución, pantalla bloqueada…)
                    self.rec.fails = if ran < QUICK_DEATH { (self.rec.fails + 1).min(4) } else { 0 };
                    self.rec.next_try = Some(now + Duration::from_secs(2u64 << self.rec.fails));
                } else {
                    self.rec.tried += 1;
                    if self.rec.tried >= ENCODERS.len() {
                        self.rec.gave_up = true;
                        log_line(&self.log_path, "ningún encoder funcionó");
                        self.notify("No pude iniciar la grabación con ningún encoder. Mirá %TEMP%\\rclip.log", true);
                        self.update_tip();
                        return;
                    }
                    self.rec.enc = (self.rec.enc + 1) % ENCODERS.len();
                    self.rec.next_try = Some(now);
                }
                self.update_tip();
            }
            if self.rec.child.is_none() && !self.rec.gave_up && self.rec.next_try.is_none_or(|t| now >= t) {
                self.spawn_rec();
            }
        }

        // ---------- acciones ----------

        fn out_dir(&self) -> Option<PathBuf> {
            let d = PathBuf::from(&self.cfg.folder);
            fs::create_dir_all(&d).ok().map(|_| d)
        }

        fn save_clip(&self) {
            if self.rec.child.is_none() {
                self.notify("No está grabando todavía. Mirá %TEMP%\\rclip.log", true);
                return;
            }
            let mut files = Vec::new();
            if let Ok(rd) = fs::read_dir(&self.buf_dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "ts") {
                        // fs::metadata abre el archivo: el mtime del que se está escribiendo sale al día
                        if let Ok(m) = fs::metadata(&p) {
                            let t = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
                            if m.len() > 0 {
                                files.push((p.to_string_lossy().into_owned(), t));
                            }
                        }
                    }
                }
            }
            let segs = logic::select_segments(files, self.cfg.secs);
            if segs.is_empty() {
                self.notify("Todavía no hay video grabado.", true);
                return;
            }
            let Some(dir) = self.out_dir() else {
                self.notify("No pude crear la carpeta de clips.", true);
                return;
            };
            let name = format!("clip_{}.mp4", now_stamp());
            let out = dir.join(&name).to_string_lossy().into_owned();
            let args = logic::clip_args(&segs, &out);
            let (ffmpeg, log) = (self.ffmpeg.clone(), self.log_path.clone());
            std::thread::spawn(move || {
                let mut cmd = Command::new(ffmpeg);
                cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_stdio(&log));
                let ok = spawn_hidden(&mut cmd, BELOW_NORMAL).and_then(|mut c| c.wait()).is_ok_and(|s| s.success());
                post_text(if ok { format!("Clip guardado: {name}") } else { "No se pudo guardar el clip. Mirá %TEMP%\\rclip.log".into() });
            });
        }

        fn screenshot(&self) {
            let Some(dir) = self.out_dir() else {
                self.notify("No pude crear la carpeta de clips.", true);
                return;
            };
            let name = format!("shot_{}.png", now_stamp());
            let out = dir.join(&name).to_string_lossy().into_owned();
            let args = logic::shot_args(&out);
            let (ffmpeg, log) = (self.ffmpeg.clone(), self.log_path.clone());
            std::thread::spawn(move || {
                let mut cmd = Command::new(ffmpeg);
                cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_stdio(&log));
                let ok = spawn_hidden(&mut cmd, BELOW_NORMAL).and_then(|mut c| c.wait()).is_ok_and(|s| s.success());
                post_text(if ok { format!("Captura guardada: {name}") } else { "No se pudo sacar la captura. Mirá %TEMP%\\rclip.log".into() });
            });
        }

        fn start_bind(&mut self, which: u8) {
            if self.hook.is_none() {
                let hmod = unsafe { GetModuleHandleW(None) }.ok().map(|h| HINSTANCE(h.0));
                match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_hook), hmod, 0) } {
                    Ok(h) => self.hook = Some(h),
                    Err(_) => {
                        self.notify("No pude escuchar el teclado.", true);
                        return;
                    }
                }
            }
            self.binding = which;
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_BIND, 15_000, None);
            }
            let what = if which == 1 { "guardar clip" } else { "captura" };
            self.notify(&format!("Apretá la tecla nueva para {what} (podés sumar Ctrl/Alt/Shift). Esc cancela."), false);
        }

        fn finish_bind(&mut self, vk: u32, mods: u32) {
            if let Some(h) = self.hook.take() {
                unsafe {
                    let _ = UnhookWindowsHookEx(h);
                    let _ = KillTimer(Some(self.hwnd), TIMER_BIND);
                }
            }
            let which = std::mem::take(&mut self.binding);
            if which == 0 {
                return;
            }
            if vk == 0 {
                self.notify("Cambio de tecla cancelado.", false);
                return;
            }
            let new = Hotkey { vk, mods };
            let other = if which == 1 { self.cfg.shot } else { self.cfg.clip };
            if new == other {
                self.notify(&format!("{} ya está usada para la otra acción.", hk(new)), true);
                return;
            }
            if which == 1 {
                self.cfg.clip = new;
            } else {
                self.cfg.shot = new;
            }
            self.save_cfg();
            self.register_keys();
            let what = if which == 1 { "Guardar clip" } else { "Captura" };
            self.notify(&format!("{what}: {}", hk(new)), false);
        }

        fn build_menu(&self) -> HMENU {
            unsafe {
                let m = CreatePopupMenu().unwrap_or_default();
                let add = |menu: HMENU, id: usize, text: &str, checked: bool| {
                    let t = wide(text);
                    let f = if checked { MF_STRING | MF_CHECKED } else { MF_STRING };
                    let _ = AppendMenuW(menu, f, id, PCWSTR(t.as_ptr()));
                };
                let sep = |menu: HMENU| {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                };
                add(m, 1, &format!("Guardar clip\t{}", hk(self.cfg.clip)), false);
                add(m, 2, &format!("Captura\t{}", hk(self.cfg.shot)), false);
                sep(m);
                let dur = CreatePopupMenu().unwrap_or_default();
                for (i, s) in DURATIONS.iter().enumerate() {
                    let label = if *s < 60 { format!("{s} s") } else { format!("{} min", s / 60) };
                    add(dur, 10 + i, &label, self.cfg.secs == *s);
                }
                let t = wide("Duración");
                let _ = AppendMenuW(m, MF_POPUP, dur.0 as usize, PCWSTR(t.as_ptr()));
                let q = CreatePopupMenu().unwrap_or_default();
                add(q, 20, "720p 30 fps", self.cfg.fps == 30);
                add(q, 21, "720p 60 fps", self.cfg.fps == 60);
                let t = wide("Calidad");
                let _ = AppendMenuW(m, MF_POPUP, q.0 as usize, PCWSTR(t.as_ptr()));
                add(m, 30, "Tecla de clip…", false);
                add(m, 31, "Tecla de captura…", false);
                sep(m);
                add(m, 40, "Abrir carpeta", false);
                add(m, 41, "Iniciar con Windows", autostart_enabled());
                sep(m);
                add(m, 50, "Salir", false);
                m
            }
        }

        /// Devuelve true si hay que salir.
        fn command(&mut self, id: usize) -> bool {
            match id {
                1 => self.save_clip(),
                2 => self.screenshot(),
                10..=14 => {
                    let s = DURATIONS[id - 10];
                    if s != self.cfg.secs {
                        self.cfg.secs = s;
                        self.save_cfg();
                        self.restart_rec();
                    }
                }
                20 | 21 => {
                    let fps = if id == 20 { 30 } else { 60 };
                    if fps != self.cfg.fps {
                        self.cfg.fps = fps;
                        self.save_cfg();
                        self.restart_rec();
                    }
                }
                30 => self.start_bind(1),
                31 => self.start_bind(2),
                40 => {
                    if let Some(d) = self.out_dir() {
                        let _ = Command::new("explorer.exe").arg(d).spawn();
                    }
                }
                41 => set_autostart(!autostart_enabled()),
                50 => return true,
                _ => {}
            }
            false
        }

        fn shutdown(&mut self) {
            self.stop_rec();
            self.clear_buf();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &self.nid());
            }
        }
    }

    // ---------- autoarranque (HKCU\...\Run) ----------

    fn exe_cmdline() -> String {
        let exe = std::env::current_exe().unwrap_or_default();
        format!("\"{}\"", exe.display())
    }

    fn autostart_value() -> Option<String> {
        unsafe {
            let mut buf = [0u16; 1024];
            let mut len = (buf.len() * 2) as u32;
            let r = RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, w!("rclip"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut len));
            if r.is_ok() {
                let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
                Some(String::from_utf16_lossy(&buf[..n]))
            } else {
                None
            }
        }
    }

    fn autostart_enabled() -> bool {
        autostart_value().is_some()
    }

    fn set_autostart(on: bool) {
        unsafe {
            if on {
                let v = wide(&exe_cmdline());
                let _ = RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("rclip"), REG_SZ.0, Some(v.as_ptr() as _), (v.len() * 2) as u32);
            } else {
                let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("rclip"));
            }
        }
    }

    // ---------- ventana y mensajes ----------

    fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
        APP.with(|a| match a.try_borrow_mut() {
            Ok(mut g) => g.as_mut().map(f),
            Err(_) => None,
        })
    }

    unsafe extern "system" fn kb_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        let msg = wp.0 as u32;
        if code == HC_ACTION as i32 && (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN) {
            let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
            let vk = k.vkCode;
            let is_mod = matches!(vk, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5);
            if !is_mod {
                let down = |v: VIRTUAL_KEY| GetAsyncKeyState(v.0 as i32) < 0;
                let mut mods = 0;
                if down(VK_CONTROL) {
                    mods |= logic::MOD_CONTROL;
                }
                if down(VK_MENU) {
                    mods |= logic::MOD_ALT;
                }
                if down(VK_SHIFT) {
                    mods |= logic::MOD_SHIFT;
                }
                let vk = if vk == VK_ESCAPE.0 as u32 { 0 } else { vk };
                let hwnd = HWND(MAIN_HWND.load(Ordering::Relaxed) as _);
                let _ = PostMessageW(Some(hwnd), WM_BIND, WPARAM(vk as usize), LPARAM(mods as isize));
                return LRESULT(1); // nos comemos la tecla
            }
        }
        CallNextHookEx(None, code, wp, lp)
    }

    fn show_menu(hwnd: HWND) {
        let Some(menu) = with_app(|a| a.build_menu()) else { return };
        let quit = unsafe {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(hwnd);
            let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, None, hwnd, None);
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            let id = cmd.0 as usize;
            id != 0 && with_app(|a| a.command(id)).unwrap_or(false)
        };
        if quit {
            with_app(|a| a.shutdown());
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_TRAY => {
                let m = (lp.0 as u32) & 0xFFFF;
                if m == WM_RBUTTONUP || m == WM_LBUTTONUP {
                    show_menu(hwnd);
                }
                LRESULT(0)
            }
            WM_HOTKEY => {
                with_app(|a| match wp.0 {
                    1 => a.save_clip(),
                    2 => a.screenshot(),
                    _ => {}
                });
                LRESULT(0)
            }
            WM_TIMER => {
                if wp.0 == TIMER_WATCHDOG {
                    with_app(|a| a.tick());
                } else if wp.0 == TIMER_BIND {
                    with_app(|a| a.finish_bind(0, 0));
                }
                LRESULT(0)
            }
            WM_DONE => {
                let text = Box::from_raw(lp.0 as *mut String);
                with_app(|a| a.notify(&text, text.starts_with("No ")));
                LRESULT(0)
            }
            WM_BIND => {
                with_app(|a| a.finish_bind(wp.0 as u32, lp.0 as u32));
                LRESULT(0)
            }
            WM_ENDSESSION if wp.0 != 0 => {
                with_app(|a| a.shutdown());
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                with_app(|a| a.add_icon());
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    unsafe fn make_icon() -> HICON {
        let size = GetSystemMetrics(SM_CXSMICON).clamp(16, 64);
        let px = logic::dot_pixels(size as usize);
        let color = CreateBitmap(size, size, 1, 32, Some(px.as_ptr() as _));
        let mask_bits = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
        let mask = CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr() as _));
        let ii = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii).unwrap_or_default();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }

    fn message_box(text: &str) {
        let t = wide(text);
        unsafe {
            MessageBoxW(None, PCWSTR(t.as_ptr()), w!("rclip"), MB_OK | MB_ICONWARNING);
        }
    }

    fn make_job() {
        unsafe {
            let Ok(job) = CreateJobObjectW(None, PCWSTR::null()) else { return };
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .is_ok()
            {
                // el handle queda abierto hasta que el proceso termina: ahí Windows mata a los hijos
                JOB.store(job.0 as isize, Ordering::Relaxed);
            }
        }
    }

    pub fn run() {
        unsafe {
            let _mutex = CreateMutexW(None, true, w!("Local\\rclip_single_instance"));
            if GetLastError() == ERROR_ALREADY_EXISTS {
                message_box("rclip ya está abierto (mirá la bandeja del sistema, al lado del reloj).");
                return;
            }

            let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
            let ffmpeg = exe_dir.join("ffmpeg.exe");
            if !ffmpeg.exists() {
                message_box("Falta ffmpeg.exe al lado de rclip.exe. Descomprimí el zip completo en una carpeta.");
                return;
            }
            let temp = std::env::temp_dir();
            let log_path = temp.join("rclip.log");
            if fs::metadata(&log_path).is_ok_and(|m| m.len() > 1_000_000) {
                let _ = File::create(&log_path);
            }
            log_line(&log_path, "inicio");
            let cfg_path = env_path("APPDATA", ".").join("rclip").join("config.ini");
            let default_folder = env_path("USERPROFILE", ".").join("Videos").join("rclip").to_string_lossy().into_owned();
            let cfg = match fs::read_to_string(&cfg_path) {
                Ok(t) => Config::parse(&t, default_folder),
                Err(_) => Config::defaults(default_folder),
            };
            let _ = fs::create_dir_all(&cfg.folder);
            make_job();

            // si el autoarranque apunta a otra ruta (movieron la carpeta), lo actualizamos
            if autostart_value().is_some_and(|v| v != exe_cmdline()) {
                set_autostart(true);
            }

            let hinst = GetModuleHandleW(None).unwrap_or_default();
            let class = w!("rclip_tray");
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
            let Ok(hwnd) = CreateWindowExW(WS_EX_TOOLWINDOW, class, w!("rclip"), WS_POPUP, 0, 0, 0, 0, None, None, Some(hinst.into()), None) else {
                return;
            };
            MAIN_HWND.store(hwnd.0 as isize, Ordering::Relaxed);

            let enc = cfg.encoder;
            let app = App {
                hwnd,
                cfg,
                cfg_path,
                ffmpeg,
                buf_dir: temp.join("rclip_buf"),
                log_path,
                icon: make_icon(),
                rec: Rec {
                    child: None,
                    enc,
                    started: Instant::now(),
                    good: false,
                    tried: 0,
                    fails: 0,
                    next_try: None,
                    gave_up: false,
                    audio: false,
                },
                binding: 0,
                hook: None,
                trimmed: false,
            };
            APP.with(|a| *a.borrow_mut() = Some(app));
            with_app(|a| {
                a.add_icon();
                a.register_keys();
                a.spawn_rec();
            });
            SetTimer(Some(hwnd), TIMER_WATCHDOG, 2000, None);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            with_app(|a| a.shutdown());
        }
    }
}

/// Audio del sistema: loopback WASAPI del dispositivo de salida por defecto -> PCM f32 por un pipe a ffmpeg.
#[cfg(windows)]
mod audio {
    use std::fs::File;
    use std::io::Write;
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};
    use windows::core::PWSTR;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;
    use windows::Win32::System::Pipes::CreatePipe;

    const LOOPBACK: u32 = 0x0002_0000;
    const AUTOCONVERT: u32 = 0x8000_0000;
    const SRC_QUALITY: u32 = 0x0800_0000;
    const SILENT: u32 = 0x2;
    const BUFFER_HNS: i64 = 2_000_000; // 200 ms
    const FLOAT_GUID: [u8; 16] = [3, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71];

    pub struct Audio {
        pub rate: u32,
        pub channels: u16,
        tx: mpsc::Sender<File>,
    }

    impl Audio {
        /// Entrega el extremo de escritura del pipe; recién ahí arranca a mandar audio.
        pub fn feed(self, pipe: File) {
            let _ = self.tx.send(pipe);
        }
    }

    /// Pipe con buffer chico (4 KB ≈ 10 ms) para saber cuándo ffmpeg empieza a leer.
    pub fn small_pipe() -> Option<(OwnedHandle, File)> {
        unsafe {
            let (mut r, mut w) = (HANDLE::default(), HANDLE::default());
            CreatePipe(&mut r, &mut w, None, 4096).ok()?;
            Some((OwnedHandle::from_raw_handle(r.0 as _), File::from_raw_handle(w.0 as _)))
        }
    }

    /// Inicializa la captura en un hilo propio. None = sin audio (se graba solo video).
    pub fn start() -> Option<Audio> {
        let (ftx, frx) = mpsc::sync_channel(1);
        let (tx, rx) = mpsc::channel::<File>();
        thread::spawn(move || unsafe { run(ftx, rx) });
        match frx.recv_timeout(Duration::from_secs(3)) {
            Ok(Some((rate, channels))) => Some(Audio { rate, channels, tx }),
            _ => None,
        }
    }

    struct Cap {
        client: IAudioClient,
        cap: IAudioCaptureClient,
        id: String,
        rate: u32,
        ch: u16,
    }

    unsafe fn default_id(en: &IMMDeviceEnumerator) -> Option<String> {
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let p: PWSTR = dev.GetId().ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s
    }

    /// want = formato que ya le dijimos a ffmpeg (al reconectar un dispositivo nuevo).
    unsafe fn open(en: &IMMDeviceEnumerator, want: Option<(u32, u16)>) -> windows::core::Result<Cap> {
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let p = dev.GetId()?;
        let id = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as _));

        // 1) pedir 48 kHz estéreo float y que Windows convierta
        let (rate, ch) = want.unwrap_or((48000, 2));
        let wf = WAVEFORMATEX {
            wFormatTag: 3u16,
            nChannels: ch,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * ch as u32 * 4,
            nBlockAlign: ch * 4,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
        if client.Initialize(AUDCLNT_SHAREMODE_SHARED, LOOPBACK | AUTOCONVERT | SRC_QUALITY, BUFFER_HNS, 0, &wf, None).is_ok() {
            let cap = client.GetService()?;
            client.Start()?;
            return Ok(Cap { client, cap, id, rate, ch });
        }

        // 2) formato nativo del mezclador (si es float 32)
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
        let mix = client.GetMixFormat()?;
        let m = *mix;
        let (tag, bits, mrate, mch) = (m.wFormatTag, m.wBitsPerSample, m.nSamplesPerSec, m.nChannels);
        let float = bits == 32
            && (tag == 3u16
                || (tag == 0xFFFE && std::ptr::read_unaligned((mix as *const u8).add(24) as *const [u8; 16]) == FLOAT_GUID));
        let ok = float && want.is_none_or(|w| w == (mrate, mch));
        let r = if ok { client.Initialize(AUDCLNT_SHAREMODE_SHARED, LOOPBACK, BUFFER_HNS, 0, mix, None) } else { Err(windows::core::Error::empty()) };
        CoTaskMemFree(Some(mix as _));
        r?;
        let cap = client.GetService()?;
        client.Start()?;
        Ok(Cap { client, cap, id, rate: mrate, ch: mch })
    }

    /// Lee todo lo disponible y lo agrega a `out`. Err = dispositivo inválido.
    unsafe fn drain(c: &Cap, out: &mut Vec<f32>) -> windows::core::Result<()> {
        loop {
            if c.cap.GetNextPacketSize()? == 0 {
                return Ok(());
            }
            let mut data = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            c.cap.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
            let n = frames as usize * c.ch as usize;
            if flags & SILENT != 0 || data.is_null() {
                out.resize(out.len() + n, 0.0);
            } else {
                out.extend_from_slice(std::slice::from_raw_parts(data as *const f32, n));
            }
            c.cap.ReleaseBuffer(frames)?;
        }
    }

    fn write_f32(pipe: &mut File, s: &[f32]) -> bool {
        let bytes = unsafe { std::slice::from_raw_parts(s.as_ptr() as *const u8, std::mem::size_of_val(s)) };
        pipe.write_all(bytes).is_ok()
    }

    unsafe fn run(ftx: mpsc::SyncSender<Option<(u32, u16)>>, rx: mpsc::Receiver<File>) {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
            Ok(e) => e,
            Err(_) => {
                let _ = ftx.send(None);
                return;
            }
        };
        let cap = match open(&en, None) {
            Ok(c) => c,
            Err(_) => {
                let _ = ftx.send(None);
                return;
            }
        };
        let (rate, ch) = (cap.rate, cap.ch);
        let _ = ftx.send(Some((rate, ch)));
        let Ok(mut pipe) = rx.recv() else { return };
        let chs = ch as usize;
        let ms = |n: u64| rate as u64 * n / 1000;

        // Pre-roll: el pipe tiene ~4 KB; la 2da escritura de 10 ms bloquea hasta que ffmpeg empieza a leer.
        let silence10 = vec![0f32; ms(10) as usize * chs];
        for _ in 0..2 {
            if !write_f32(&mut pipe, &silence10) {
                return;
            }
        }
        let mut cap_opt = Some(cap);
        let mut pending: Vec<f32> = Vec::with_capacity(ms(100) as usize * chs);
        if let Some(c) = &cap_opt {
            let _ = drain(c, &mut pending); // lo capturado antes de arrancar se descarta
        }
        pending.clear();
        let t0 = Instant::now();
        let mut written: u64 = 0;
        let mut last_check = Instant::now();

        loop {
            thread::sleep(Duration::from_millis(10));

            // cambio de dispositivo por defecto / dispositivo inválido
            if last_check.elapsed() >= Duration::from_secs(1) {
                last_check = Instant::now();
                let changed = match (&cap_opt, default_id(&en)) {
                    (Some(c), Some(id)) => c.id != id,
                    (None, _) => true,
                    _ => false,
                };
                if changed {
                    if let Some(c) = cap_opt.take() {
                        let _ = c.client.Stop();
                    }
                    cap_opt = open(&en, Some((rate, ch))).ok();
                }
            }
            if let Some(c) = &cap_opt {
                if drain(c, &mut pending).is_err() {
                    cap_opt = None;
                }
            }

            // mantener el audio pegado al reloj de pared
            let target = t0.elapsed().as_micros() as u64 * rate as u64 / 1_000_000;
            let have = written + (pending.len() / chs) as u64;
            if have + ms(40) < target {
                // loopback no manda nada cuando hay silencio: rellenamos
                pending.resize(pending.len() + (target - have) as usize * chs, 0.0);
            } else if have > target + ms(50) {
                let drop = ((have - target) as usize * chs).min(pending.len());
                pending.drain(..drop);
            }
            if !pending.is_empty() {
                if !write_f32(&mut pipe, &pending) {
                    break; // ffmpeg terminó
                }
                written += (pending.len() / chs) as u64;
                pending.clear();
            }
        }
        if let Some(c) = cap_opt {
            let _ = c.client.Stop();
        }
    }
}
