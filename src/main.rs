// Jugadón: clips de juegos desde la bandeja del sistema.
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
    pub const VK_F8: u32 = 0x77;
    pub const BG_OFF: u32 = 0;
    pub const BG_GAMES: u32 = 1;
    pub const BG_ALWAYS: u32 = 2;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Hotkey {
        pub vk: u32,
        pub mods: u32,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct Config {
        pub secs: u32,
        pub fps: u32,
        /// alto del video: 0 = automático (720 con la placa, 480 si comprime el procesador), 480 o 720
        pub height: u32,
        pub clip: Hotkey,
        pub shot: Hotkey,
        pub folder: String,
        pub encoder: usize,
        /// clips en segundo plano: BG_OFF, BG_GAMES (solo mientras hay un juego) o BG_ALWAYS
        pub bg: u32,
        /// tecla para empezar/frenar una grabación manual
        pub rec: Hotkey,
        pub mic: bool,
        pub sound: bool,
        /// comprimir solo con la placa de video (NVENC/AMF/QSV); nunca x264 en el procesador
        pub gpu_only: bool,
    }

    impl Config {
        pub fn defaults(folder: String) -> Self {
            Config {
                secs: 30,
                fps: 30,
                height: 0,
                clip: Hotkey { vk: VK_F9, mods: 0 },
                shot: Hotkey { vk: VK_F10, mods: 0 },
                folder,
                encoder: 0,
                bg: BG_GAMES,
                rec: Hotkey { vk: VK_F8, mods: 0 },
                mic: true,
                sound: true,
                gpu_only: true,
            }
        }

        pub fn parse(text: &str, default_folder: String) -> Self {
            let mut c = Self::defaults(default_folder);
            // configs anteriores a la versión 2 tenían el micrófono apagado por defecto: se ignora ese valor
            let v2 = text.lines().any(|l| l.trim() == "version=2");
            for line in text.lines() {
                let Some((k, v)) = line.split_once('=') else { continue };
                let (k, v) = (k.trim(), v.trim());
                let n = v.parse::<u32>().ok();
                match (k, n) {
                    ("seconds", Some(n)) if (5..=600).contains(&n) => c.secs = n,
                    ("fps", Some(n)) if n == 30 || n == 60 => c.fps = n,
                    // "quality" reemplaza a "height" (las configs viejas pasan a automático)
                    ("quality", Some(n)) if n == 0 || n == 480 || n == 720 => c.height = n,
                    ("clip_key", Some(n)) if (1..=254).contains(&n) => c.clip.vk = n,
                    ("clip_mods", Some(n)) => c.clip.mods = n & 7,
                    ("shot_key", Some(n)) if (1..=254).contains(&n) => c.shot.vk = n,
                    ("shot_mods", Some(n)) => c.shot.mods = n & 7,
                    ("encoder", Some(n)) if (n as usize) < ENCODERS.len() => c.encoder = n as usize,
                    ("folder", _) if !v.is_empty() => c.folder = v.to_string(),
                    ("bg", Some(n)) if n <= BG_ALWAYS => c.bg = n,
                    ("only_games", Some(0)) => c.bg = BG_ALWAYS, // config de la versión 0.3
                    ("rec_key", Some(n)) if (1..=254).contains(&n) => c.rec.vk = n,
                    ("rec_mods", Some(n)) => c.rec.mods = n & 7,
                    ("mic", Some(n)) if n <= 1 && v2 => c.mic = n == 1,
                    ("sound", Some(n)) if n <= 1 => c.sound = n == 1,
                    ("gpu_only", Some(n)) if n <= 1 => c.gpu_only = n == 1,
                    _ => {}
                }
            }
            if c.clip == c.shot || c.rec == c.clip || c.rec == c.shot {
                c.clip = Hotkey { vk: VK_F9, mods: 0 };
                c.shot = Hotkey { vk: VK_F10, mods: 0 };
                c.rec = Hotkey { vk: VK_F8, mods: 0 };
            }
            c
        }

        pub fn serialize(&self) -> String {
            format!(
                "version=2\nseconds={}\nfps={}\nquality={}\nclip_key={}\nclip_mods={}\nshot_key={}\nshot_mods={}\nfolder={}\nencoder={}\nbg={}\nrec_key={}\nrec_mods={}\nmic={}\nsound={}\ngpu_only={}\n",
                self.secs,
                self.fps,
                self.height,
                self.clip.vk,
                self.clip.mods,
                self.shot.vk,
                self.shot.mods,
                self.folder,
                self.encoder,
                self.bg,
                self.rec.vk,
                self.rec.mods,
                self.mic as u8,
                self.sound as u8,
                self.gpu_only as u8
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
        /// captura por GDI (para drivers viejos sin Desktop Duplication, ej. Intel HD 2000/3000)
        pub gdi: bool,
        pub vf: &'static str,
        pub codec: &'static str,
        pub opts: &'static [&'static str],
    }

    const NV: &[&str] = &["-preset", "p1", "-tune", "ll", "-rc", "cbr"];
    const AMF: &[&str] = &["-usage", "lowlatency", "-quality", "speed", "-rc", "cbr"];
    const QSV: &[&str] = &["-preset", "veryfast"];
    // x264 (sin placa que codifique): el preset más liviano y solo 2 hilos, para dejarle CPU al juego
    // los hilos se agregan según la resolución (1 hilo a 480p, 2 a 720p)
    const X264: &[&str] = &["-preset", "ultrafast", "-tune", "zerolatency"];
    // a 720p como máximo (una ventana chica no se agranda)
    const GPU: &str = "scale_d3d11=width=-2:height=min(720\\,trunc(ih/2)*2):format=nv12";
    const CPU: &str = "hwdownload,format=bgra,scale=-2:min(720\\,trunc(ih/2)*2):flags=fast_bilinear,format=nv12";

    /// Orden de prueba: todo en GPU primero, después escalado en CPU, último x264.
    /// ("720" en vf se reemplaza por el alto elegido.)
    pub const ENCODERS: [Enc; 9] = [
        Enc { gdi: false, label: "NVENC", vf: GPU, codec: "h264_nvenc", opts: NV },
        Enc { gdi: false, label: "AMF", vf: GPU, codec: "h264_amf", opts: AMF },
        Enc {
            gdi: false,
            label: "QSV",
            vf: "scale_d3d11=width=-2:height=min(720\\,trunc(ih/2)*2):format=nv12,hwmap=derive_device=qsv,format=qsv",
            codec: "h264_qsv",
            opts: QSV,
        },
        Enc { gdi: false, label: "NVENC (escala CPU)", vf: CPU, codec: "h264_nvenc", opts: NV },
        Enc { gdi: false, label: "AMF (escala CPU)", vf: CPU, codec: "h264_amf", opts: AMF },
        Enc { gdi: false, label: "QSV (escala CPU)", vf: CPU, codec: "h264_qsv", opts: QSV },
        // la placa escala (barato) y la CPU solo comprime un video chico
        Enc { gdi: false, label: "x264 (escala GPU)", vf: "scale_d3d11=width=-2:height=min(720\\,trunc(ih/2)*2):format=nv12,hwdownload,format=nv12", codec: "libx264", opts: X264 },
        Enc { gdi: false, label: "x264 (CPU)", vf: CPU, codec: "libx264", opts: X264 },
        // último recurso: driver sin Desktop Duplication (WDDM 1.1). Captura por GDI, todo en CPU.
        Enc { gdi: true, label: "x264 (captura GDI)", vf: "scale=-2:min(720\\,trunc(ih/2)*2):flags=fast_bilinear,format=nv12", codec: "libx264", opts: X264 },
    ];

    /// ¿Se puede usar este encoder? Con gpu_only, x264 (procesador) queda afuera.
    pub fn allowed(enc: usize, gpu_only: bool) -> bool {
        !gpu_only || ENCODERS[enc].codec != "libx264"
    }

    /// Encoder con el que arrancar: el que anduvo la última vez, si está permitido.
    pub fn first_encoder(saved: usize, gpu_only: bool) -> usize {
        if saved < ENCODERS.len() && allowed(saved, gpu_only) {
            saved
        } else {
            0
        }
    }

    /// Siguiente encoder a probar cuando `enc` falló. `tried` = cuántos se probaron antes de `enc`.
    /// Devuelve (encoder, nuevo tried) o None si no queda ninguno.
    pub fn next_encoder(enc: usize, tried: usize, gpu_only: bool) -> Option<(usize, usize)> {
        let n = ENCODERS.len();
        let (mut e, mut t) = (enc, tried);
        loop {
            t += 1;
            if t >= n {
                return None;
            }
            e = (e + 1) % n;
            if allowed(e, gpu_only) {
                return Some((e, t));
            }
        }
    }

    /// Alto y fps reales. Automático (0): 720p con la placa, 480p si comprime el procesador.
    /// x264 (procesador) nunca pasa de 30 fps.
    pub fn effective(enc: usize, height: u32, fps: u32) -> (u32, u32) {
        let cpu = ENCODERS[enc].codec == "libx264";
        let h = match height {
            0 if cpu => 480,
            0 => 720,
            h => h,
        };
        if cpu {
            (h, fps.min(30))
        } else {
            (h, fps)
        }
    }

    pub fn bitrate_kbit(height: u32, fps: u32) -> u32 {
        match (height >= 720, fps >= 60) {
            (true, true) => 9000,
            (true, false) => 6000,
            (false, true) => 4000,
            (false, false) => 2500,
        }
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    /// Dónde escribe ffmpeg: segmentos rotativos (clips en segundo plano) o un archivo (grabación manual).
    pub enum Out<'a> {
        Segments { secs: u32, dir: &'a str },
        File(&'a str),
    }

    /// Qué se graba: la pantalla principal o una sola ventana (por su HWND).
    /// La ventana se graba con Windows Graphics Capture: sale solo esa ventana,
    /// aunque esté tapada o en segundo plano.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Source {
        Screen,
        Window(u64),
    }

    /// Argumentos de grabación. audio = (Hz, canales) de PCM f32 por stdin.
    pub fn record_args(enc: usize, height: u32, fps: u32, src: Source, audio: Option<(u32, u16)>, out: Out) -> Vec<String> {
        let e = &ENCODERS[enc];
        let (height, fps) = effective(enc, height, fps);
        let br = bitrate_kbit(height, fps);
        let vf = e.vf.replace("720", &height.to_string());
        // un solo hilo de filtros: todo el trabajo pesado está en la GPU
        let mut a = s(&["-hide_banner", "-loglevel", "warning", "-nostats", "-filter_threads", "1"]);
        if audio.is_none() {
            a.push("-nostdin".into());
        }
        if e.gdi {
            a.extend(["-f".into(), "gdigrab".into(), "-framerate".into(), fps.to_string(), "-draw_mouse".into(), "1".into()]);
            a.extend(s(&["-i", "desktop"]));
        } else {
            a.extend(s(&["-f", "lavfi", "-i"]));
            a.push(match src {
                Source::Screen => format!("ddagrab=framerate={fps}:draw_mouse=1"),
                Source::Window(h) => format!("gfxcapture=hwnd={h}:max_framerate={fps}:capture_cursor=1"),
            });
        }
        if let Some((rate, ch)) = audio {
            // sin probesize/analyzeduration ffmpeg lee ~5 s de audio antes de arrancar el video (desfase)
            a.extend(s(&["-thread_queue_size", "128", "-probesize", "32", "-analyzeduration", "0", "-f", "f32le", "-ar"]));
            a.push(rate.to_string());
            a.push("-ac".into());
            a.push(ch.to_string());
            a.extend(s(&["-i", "pipe:0", "-map", "0:v", "-map", "1:a"]));
        } else {
            a.extend(s(&["-map", "0:v"]));
        }
        a.extend(["-vf".into(), vf]);
        if matches!(src, Source::Window(_)) && !e.gdi {
            // la captura de ventana manda cuadros solo cuando la ventana cambia: video a fps fijos
            a.extend(["-fps_mode".into(), "cfr".into(), "-r".into(), fps.to_string()]);
        }
        a.extend(s(&["-c:v", e.codec]));
        a.extend(s(e.opts));
        if e.codec == "libx264" {
            a.extend(s(&["-threads", if height >= 720 { "2" } else { "1" }]));
        }
        a.extend([
            "-b:v".into(),
            format!("{br}k"),
            "-maxrate".into(),
            format!("{br}k"),
            "-bufsize".into(),
            format!("{}k", br * 2),
            "-g".into(),
            (fps * 2).to_string(),
            "-bf".into(),
            "0".into(),
        ]);
        if audio.is_some() {
            // codificador AAC "fast": mucho menos CPU que el de fábrica, misma calidad para juegos/voz
            a.extend(s(&["-c:a", "aac", "-aac_coder", "fast", "-b:a", "128k"]));
        }
        match out {
            Out::Segments { secs, dir } => {
                a.extend(s(&["-f", "segment", "-segment_time", "2", "-segment_wrap"]));
                a.push(segment_wrap(secs).to_string());
                a.extend(s(&["-segment_format", "mpegts"]));
                a.push(format!("{dir}\\seg%03d.ts"));
            }
            // .ts: si algo se corta, lo grabado hasta ahí sigue sirviendo
            Out::File(path) => a.extend(s(&["-y", "-f", "mpegts", path])),
        }
        a
    }

    /// Junta segmentos sin recodificar (copia de stream).
    pub fn clip_args(segs: &[String], out: &str) -> Vec<String> {
        let mut a = s(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"]);
        a.push(format!("concat:{}", segs.join("|")));
        a.extend(s(&["-map", "0", "-c", "copy", "-movflags", "+faststart", out]));
        a
    }

    /// Pasa una grabación .ts a .mp4 sin recodificar.
    pub fn remux_args(input: &str, out: &str) -> Vec<String> {
        s(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i", input, "-map", "0", "-c", "copy", "-movflags", "+faststart", out])
    }

    /// "1:05:09" / "3:07"
    pub fn fmt_elapsed(secs: u64) -> String {
        let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
        if h > 0 {
            format!("{h}:{m:02}:{s:02}")
        } else {
            format!("{m}:{s:02}")
        }
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
            "1",
            out,
        ])
    }

    /// Captura por GDI (si el driver no tiene Desktop Duplication).
    pub fn shot_args_gdi(out: &str) -> Vec<String> {
        s(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-f", "gdigrab", "-i", "desktop", "-frames:v", "1", "-update", "1", "-compression_level", "1", out])
    }

    pub fn stamp(y: u16, mo: u16, d: u16, h: u16, mi: u16, sec: u16) -> String {
        format!("{y:04}-{mo:02}-{d:02}_{h:02}-{mi:02}-{sec:02}")
    }

    /// Para la ventana de clips: (tipo, fecha legible) a partir del nombre del archivo.
    pub fn describe_file(name: &str) -> (&'static str, String) {
        let kind = if name.starts_with("clip_") {
            "Clip"
        } else if name.starts_with("grabacion") {
            "Grabación"
        } else if name.starts_with("shot_") {
            "Captura"
        } else if name.ends_with(".png") {
            "Imagen"
        } else {
            "Video"
        };
        // ..._AAAA-MM-DD_HH-MM-SS.ext
        let stem = name.rsplit_once('.').map_or(name, |x| x.0);
        let date = stem
            .char_indices()
            .filter(|(_, c)| c.is_ascii_digit())
            .map(|(i, _)| i)
            .find(|&i| {
                let t = stem.get(i..i + 19).unwrap_or("").as_bytes();
                t.len() == 19 && t[4] == b'-' && t[7] == b'-' && t[10] == b'_' && t[13] == b'-' && t[16] == b'-'
            })
            .map(|i| {
                let t = &stem[i..i + 19];
                format!("{}/{}/{} {}:{}", &t[8..10], &t[5..7], &t[0..4], &t[11..13], &t[14..16])
            })
            .unwrap_or_default();
        (kind, date)
    }

    pub fn fmt_size(bytes: u64) -> String {
        if bytes >= 1 << 30 {
            format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
        } else {
            format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
        }
    }

    /// Suma `q` (multiplicado por `gain`) sobre `out` y saca de `q` lo usado.
    pub fn mix_into(out: &mut [f32], q: &mut Vec<f32>, gain: f32) {
        let n = out.len().min(q.len());
        for (o, s) in out.iter_mut().zip(&q[..n]) {
            *o += s * gain;
        }
        q.drain(..n);
    }

    /// Si la cola se atrasó más de `max` muestras, tira lo más viejo.
    pub fn trim_queue(q: &mut Vec<f32>, max: usize) {
        if q.len() > max {
            q.drain(..q.len() - max);
        }
    }

    /// ¿La ventana (izq, arriba, der, abajo) tapa todo el monitor? = juego a pantalla completa.
    pub fn covers(win: (i32, i32, i32, i32), mon: (i32, i32, i32, i32)) -> bool {
        mon.2 > mon.0 && win.0 <= mon.0 && win.1 <= mon.1 && win.2 >= mon.2 && win.3 >= mon.3
    }

    /// Sonido corto de "clip guardado" (WAV 16 bit mono en memoria, ~7 KB).
    pub fn clip_sound_wav() -> Vec<u8> {
        let rate = 22050u32;
        let mut pcm: Vec<i16> = Vec::new();
        for (freq, ms) in [(880.0f32, 70u32), (1318.5, 110)] {
            let n = rate * ms / 1000;
            for i in 0..n {
                let t = i as f32 / rate as f32;
                let env = (i as f32 / 200.0).min(1.0) * (1.0 - i as f32 / n as f32).powf(1.5);
                pcm.push(((t * freq * std::f32::consts::TAU).sin() * env * 0.35 * 32767.0) as i16);
            }
        }
        let data_len = (pcm.len() * 2) as u32;
        let mut w = Vec::with_capacity(44 + data_len as usize);
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&rate.to_le_bytes());
        w.extend_from_slice(&(rate * 2).to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data_len.to_le_bytes());
        for v in pcm {
            w.extend_from_slice(&v.to_le_bytes());
        }
        w
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
            c.height = 480;
            c.clip = Hotkey { vk: 0x41, mods: MOD_CONTROL | MOD_SHIFT };
            c.encoder = 3;
            c.bg = BG_OFF;
            c.rec = Hotkey { vk: 0x52, mods: MOD_ALT };
            c.mic = false;
            c.sound = false;
            c.folder = "D:\\Mis clips\\Jugadon".into();
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
        fn new_options_default_and_parse() {
            let d = Config::defaults("F".into());
            assert!(d.gpu_only);
            assert!(!Config::parse("gpu_only=0\n", "F".into()).gpu_only);
            assert!(d.bg == BG_GAMES && d.mic && d.sound && d.rec.vk == VK_F8);
            let c = Config::parse("version=2\nbg=0\nmic=0\nsound=0\n", "F".into());
            assert!(c.bg == BG_OFF && !c.mic && !c.sound);
            let c = Config::parse("bg=7\nmic=x\n", "F".into());
            assert!(c.bg == BG_GAMES && c.mic);
            // config vieja (sin version=2) con mic=0: ahora el mic viene prendido
            assert!(Config::parse("mic=0\n", "F".into()).mic);
            // config vieja (0.3): only_games=0 -> siempre
            assert_eq!(Config::parse("only_games=0\n", "F".into()).bg, BG_ALWAYS);
            // tecla de grabar repetida -> defaults
            let c = Config::parse("rec_key=120\n", "F".into());
            assert_eq!((c.clip.vk, c.shot.vk, c.rec.vk), (VK_F9, VK_F10, VK_F8));
        }

        #[test]
        fn encoder_chain() {
            // solo placa: recorre NVENC, AMF, QSV y sus variantes, nunca x264
            let mut seen = vec![0];
            let (mut e, mut t) = (0, 0);
            while let Some((ne, nt)) = next_encoder(e, t, true) {
                seen.push(ne);
                (e, t) = (ne, nt);
            }
            assert!(seen.iter().all(|&i| ENCODERS[i].codec != "libx264"), "{seen:?}");
            assert_eq!(seen.len(), 6);
            // sin restricción: prueba los 9
            let mut n = 1;
            let (mut e, mut t) = (0, 0);
            while let Some((ne, nt)) = next_encoder(e, t, false) {
                n += 1;
                (e, t) = (ne, nt);
            }
            assert_eq!(n, ENCODERS.len());
            // arrancando de uno guardado en el medio, da la vuelta
            assert_eq!(next_encoder(5, 0, true), Some((0, 4)));
            assert_eq!(first_encoder(7, true), 0);
            assert_eq!(first_encoder(7, false), 7);
            assert_eq!(first_encoder(2, true), 2);
        }

        #[test]
        fn gallery_rows() {
            assert_eq!(describe_file("clip_2026-11-18_16-30-05.mp4"), ("Clip", "18/11/2026 16:30".to_string()));
            assert_eq!(describe_file("grabacion_recuperada_2026-01-02_03-04-05_0.mp4"), ("Grabación", "02/01/2026 03:04".to_string()));
            assert_eq!(describe_file("shot_2026-11-18_16-30-05.png").0, "Captura");
            assert_eq!(describe_file("mi video.mp4"), ("Video", String::new()));
            assert_eq!(fmt_size(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
            assert_eq!(fmt_size(3 << 30), "3.0 GB");
        }

        #[test]
        fn audio_mixing() {
            let mut out = vec![0.1f32; 4];
            let mut q = vec![1.0, 2.0];
            mix_into(&mut out, &mut q, 0.5);
            assert_eq!(out, vec![0.6, 1.1, 0.1, 0.1]);
            assert!(q.is_empty());
            let mut q: Vec<f32> = (0..10).map(|x| x as f32).collect();
            let mut out = vec![0.0; 3];
            mix_into(&mut out, &mut q, 1.0);
            assert_eq!((out, q.len()), (vec![0.0, 1.0, 2.0], 7));
            trim_queue(&mut q, 4);
            assert_eq!(q, vec![6.0, 7.0, 8.0, 9.0]);
            trim_queue(&mut q, 10);
            assert_eq!(q.len(), 4);
        }

        #[test]
        fn fullscreen_detection() {
            let mon = (0, 0, 1920, 1080);
            assert!(covers((0, 0, 1920, 1080), mon));
            assert!(covers((-8, -8, 1928, 1088), mon)); // ventana maximizada sin bordes
            assert!(!covers((0, 0, 1920, 1040), mon)); // ventana maximizada normal (deja ver la barra de tareas)
            assert!(!covers((100, 100, 800, 600), mon));
            assert!(!covers((0, 0, 0, 0), (0, 0, 0, 0)));
        }

        #[test]
        fn clip_sound() {
            let w = clip_sound_wav();
            assert_eq!(&w[0..4], b"RIFF");
            assert_eq!(&w[8..16], b"WAVEfmt ");
            assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
            assert!(w.len() < 10_000);
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
            let a = record_args(0, 720, 60, Source::Screen, Some((48000, 2)), Out::Segments { secs: 30, dir: "C:\\T\\jugadon_buf" }).join(" ");
            assert!(a.contains("-f lavfi -i ddagrab=framerate=60:draw_mouse=1"));
            assert!(a.starts_with("-hide_banner -loglevel warning -nostats -filter_threads 1 -f lavfi"));
            assert!(a.contains("-thread_queue_size 128 -probesize 32 -analyzeduration 0 -f f32le -ar 48000 -ac 2 -i pipe:0 -map 0:v -map 1:a"));
            assert!(a.contains("-vf scale_d3d11=width=-2:height=min(720\\,trunc(ih/2)*2):format=nv12 -c:v h264_nvenc -preset p1 -tune ll -rc cbr"));
            assert!(a.contains("-b:v 9000k -maxrate 9000k -bufsize 18000k -g 120 -bf 0"));
            assert!(a.contains("-c:a aac -aac_coder fast -b:a 128k"));
            assert!(a.ends_with("-f segment -segment_time 2 -segment_wrap 20 -segment_format mpegts C:\\T\\jugadon_buf\\seg%03d.ts"));
            assert!(!a.contains("-nostdin"));
            assert!(!a.contains("reset_timestamps"));
        }

        #[test]
        fn record_args_cpu_no_audio() {
            let a = record_args(7, 720, 30, Source::Screen, None, Out::Segments { secs: 60, dir: "D" }).join(" ");
            assert!(a.contains("-nostdin"));
            // x264 en CPU: 720p si se pide, máximo 30 fps, 2 hilos
            assert!(a.contains("-map 0:v -vf hwdownload,format=bgra,scale=-2:min(720\\,trunc(ih/2)*2)"));
            assert!(a.contains("-c:v libx264 -preset ultrafast -tune zerolatency -threads 2"));
            // automático con procesador: 480p, 1 hilo
            let a = record_args(7, 0, 30, Source::Screen, None, Out::File("x")).join(" ");
            assert!(a.contains("scale=-2:min(480") && a.contains("-threads 1") && a.contains("-b:v 2500k"));
            // automático con placa: 720p
            let a = record_args(0, 0, 30, Source::Screen, None, Out::File("x")).join(" ");
            assert!(a.contains("height=min(720") && a.contains("-b:v 6000k") && !a.contains("-threads"));
            assert_eq!(Config::defaults("F".into()).height, 0);
            assert_eq!(Config::parse("height=720\n", "F".into()).height, 0);
            assert_eq!(Config::parse("quality=720\n", "F".into()).height, 720);
            assert!(a.contains("-b:v 6000k"));
            assert!(a.contains("-g 60"));
            let a = record_args(6, 720, 60, Source::Screen, None, Out::File("x")).join(" ");
            assert!(a.contains("ddagrab=framerate=30:"));
            assert!(a.contains("scale_d3d11=width=-2:height=min(720\\,trunc(ih/2)*2):format=nv12,hwdownload,format=nv12"));
            // GDI: otra entrada, sin lavfi
            let a = record_args(8, 720, 30, Source::Screen, None, Out::File("x")).join(" ");
            assert!(a.contains("-f gdigrab -framerate 30 -draw_mouse 1 -i desktop -map 0:v -vf scale=-2:min(720"));
            assert!(!a.contains("lavfi"));
            // 480p elegido por el usuario, con encoder de placa
            let a = record_args(0, 480, 30, Source::Screen, None, Out::File("x")).join(" ");
            assert!(a.contains("height=min(480") && a.contains("-b:v 2500k") && a.contains("h264_nvenc"));
            assert!(!a.contains("pipe:0"));
            assert!(!a.contains("-c:a"));
            for i in 0..ENCODERS.len() {
                assert!(record_args(i, 720, 30, Source::Screen, None, Out::File("x.ts")).iter().any(|x| x == ENCODERS[i].codec));
            }
        }

        #[test]
        fn manual_recording_args() {
            let a = record_args(0, 720, 30, Source::Window(1234), None, Out::File("C:\\T\\rec.ts")).join(" ");
            assert!(a.contains("-i gfxcapture=hwnd=1234:max_framerate=30:capture_cursor=1 "));
            assert!(a.contains("format=nv12 -fps_mode cfr -r 30 -c:v h264_nvenc"));
            assert!(!a.contains("ddagrab"));
            assert!(!record_args(0, 720, 30, Source::Screen, None, Out::File("x")).join(" ").contains("-fps_mode"));
            assert!(a.ends_with("-y -f mpegts C:\\T\\rec.ts"));
            assert!(!a.contains("segment"));
            let r = remux_args("a.ts", "b.mp4").join(" ");
            assert!(r.contains("-i a.ts -map 0 -c copy -movflags +faststart b.mp4"));
            assert_eq!(fmt_elapsed(187), "3:07");
            assert_eq!(fmt_elapsed(3909), "1:05:09");
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
        let dir = std::env::temp_dir().join("jugadon_test_buf");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dirs = dir.to_string_lossy().into_owned();
        let mut args = logic::record_args(7, 720, 30, logic::Source::Screen, Some((48000, 2)), logic::Out::Segments { secs: 6, dir: &dirs });
        for a in args.iter_mut() {
            if a.starts_with("ddagrab=") {
                *a = "testsrc2=size=1920x1080:rate=30,realtime".into();
            } else if a.starts_with("hwdownload,format=bgra,") {
                *a = a.replacen("hwdownload,format=bgra,", "", 1);
            }
        }
        let (r, mut w) = crate::audio::small_pipe().expect("pipe");
        let mut child = Command::new(&ff).args(&args).stdin(Stdio::from(r)).stdout(Stdio::null()).spawn().unwrap();
        // igual que la app: el reloj arranca recién cuando ffmpeg empieza a leer (2da escritura bloquea)
        let silence = vec![0u8; 480 * 2 * 4];
        let t_pre = Instant::now();
        for _ in 0..2 {
            w.write_all(&silence).expect("pipe");
        }
        eprintln!("ffmpeg empezó a leer después de {} ms", t_pre.elapsed().as_millis());
        let t0 = Instant::now();
        let mut frames = 0u64;
        while t0.elapsed() < Duration::from_secs(8) {
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
        let out = std::env::temp_dir().join("jugadon_test_clip.mp4").to_string_lossy().into_owned();
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

#[cfg(all(test, windows))]
mod win_tests_manual {
    use crate::logic;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    /// Grabación manual: ffmpeg escribe un .ts, se corta de golpe (como al frenar) y se pasa a .mp4.
    #[test]
    fn manual_recording_kill_and_remux() {
        let Ok(ff) = std::env::var("RCLIP_FFMPEG") else {
            eprintln!("RCLIP_FFMPEG no definido: se saltea");
            return;
        };
        let tmp = std::env::temp_dir();
        let ts = tmp.join("jugadon_test_rec.ts").to_string_lossy().into_owned();
        let mp4 = tmp.join("jugadon_test_rec.mp4").to_string_lossy().into_owned();
        let mut args = logic::record_args(7, 720, 30, logic::Source::Window(0), None, logic::Out::File(&ts));
        for a in args.iter_mut() {
            if a.starts_with("gfxcapture=") {
                *a = "testsrc2=size=800x601:rate=30,realtime".into(); // alto impar a propósito
            } else if a.starts_with("hwdownload,format=bgra,") {
                *a = a.replacen("hwdownload,format=bgra,", "", 1);
            }
        }
        let mut child = Command::new(&ff).args(&args).stdout(Stdio::null()).spawn().unwrap();
        std::thread::sleep(Duration::from_secs(4));
        let _ = child.kill(); // frenar = matar ffmpeg; el .ts tiene que seguir sirviendo
        let _ = child.wait();
        let st = Command::new(&ff).args(logic::remux_args(&ts, &mp4)).status().unwrap();
        assert!(st.success(), "falló el remux");
        let probe = std::path::Path::new(&ff).with_file_name("ffprobe.exe");
        let o = Command::new(probe)
            .args(["-v", "error", "-show_entries", "stream=codec_name,height:format=duration", "-of", "default=nw=1", &mp4])
            .output()
            .unwrap();
        let info = String::from_utf8_lossy(&o.stdout).to_string();
        eprintln!("{info}");
        // 601 px de alto -> 600: una ventana chica no se agranda a 720 y queda par
        assert!(info.contains("codec_name=h264") && info.contains("height=600"), "{info}");
        let dur: f64 = info.lines().find_map(|l| l.strip_prefix("duration=")).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        assert!((1.5..=5.0).contains(&dur), "duración rara: {dur}");
    }
}

/// Captura real de pantalla por GDI (el respaldo para drivers viejos) con los mismos argumentos que la app.
/// En el servidor de CI puede no haber escritorio: en ese caso se saltea con un aviso.
#[cfg(all(test, windows))]
mod win_tests_gdi {
    use crate::logic;
    use std::os::windows::io::AsRawHandle;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn cpu_ms(child: &std::process::Child) -> f64 {
        use windows::Win32::Foundation::{FILETIME, HANDLE};
        use windows::Win32::System::Threading::GetProcessTimes;
        let (mut a, mut b, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        unsafe {
            let _ = GetProcessTimes(HANDLE(child.as_raw_handle() as _), &mut a, &mut b, &mut k, &mut u);
        }
        let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 10_000.0;
        t(k) + t(u)
    }

    #[test]
    fn gdi_screen_capture_720p30() {
        let Ok(ff) = std::env::var("RCLIP_FFMPEG") else { return };
        let gdi = logic::ENCODERS.iter().position(|e| e.gdi).unwrap();
        let ts = std::env::temp_dir().join("jugadon_test_gdi.ts").to_string_lossy().into_owned();
        let args = logic::record_args(gdi, 720, 30, logic::Source::Screen, None, logic::Out::File(&ts));
        let mut child = Command::new(&ff).args(&args).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
        let t0 = Instant::now();
        std::thread::sleep(Duration::from_secs(8));
        if let Some(st) = child.try_wait().unwrap() {
            let mut err = String::new();
            use std::io::Read;
            let _ = child.stderr.take().unwrap().read_to_string(&mut err);
            eprintln!("GDI no disponible en este servidor ({st}): {err}");
            return;
        }
        let cpu = cpu_ms(&child);
        let wall = t0.elapsed().as_millis() as f64;
        let _ = child.kill();
        let _ = child.wait();
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        eprintln!(
            "GDI + x264 720p30 en el servidor de CI ({cores} hilos): ffmpeg usó {:.0} % de un núcleo ({:.1} % del total)",
            cpu / wall * 100.0,
            cpu / wall * 100.0 / cores as f64
        );
        let probe = std::path::Path::new(&ff).with_file_name("ffprobe.exe");
        let o = Command::new(probe).args(["-v", "error", "-show_entries", "stream=codec_name,height", "-of", "default=nw=1", &ts]).output().unwrap();
        let info = String::from_utf8_lossy(&o.stdout).to_string();
        eprintln!("{info}");
        assert!(info.contains("codec_name=h264"), "{info}");
    }
}

/// Costo fijo de ffmpeg mientras graba, sin contar el video (que en una PC real lo comprime la placa):
/// audio por pipe + AAC + segmentos. Se mide con un video mínimo para aislarlo.
#[cfg(all(test, windows))]
mod win_tests_overhead {
    use std::io::Write;
    use std::os::windows::io::AsRawHandle;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    #[test]
    fn audio_and_mux_cpu_cost() {
        let Ok(ff) = std::env::var("RCLIP_FFMPEG") else { return };
        let dir = std::env::temp_dir().join("jugadon_test_overhead");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let out = format!("{}\\seg%03d.ts", dir.display());
        let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostats", "-filter_threads", "1", "-f", "lavfi", "-i", "color=black:s=64x64:r=30"]
            .iter()
            .map(|x| x.to_string())
            .collect();
        // mismos argumentos de audio y salida que la app
        let app = crate::logic::record_args(0, 720, 30, crate::logic::Source::Screen, Some((48000, 2)), crate::logic::Out::Segments { secs: 30, dir: "D" });
        let ai = app.iter().position(|x| x == "-thread_queue_size").unwrap();
        args.extend(app[ai..ai + 14].iter().cloned()); // -thread_queue_size ... -i pipe:0
        args.extend(["-map", "0:v", "-map", "1:a", "-c:v", "libx264", "-preset", "ultrafast", "-g", "60"].iter().map(|x| x.to_string()));
        let ca = app.iter().position(|x| x == "-c:a").unwrap();
        args.extend(app[ca..ca + 6].iter().cloned());
        args.extend(["-f", "segment", "-segment_time", "2", "-segment_wrap", "20", "-segment_format", "mpegts", &out].iter().map(|x| x.to_string()));
        eprintln!("args: {}", args.join(" "));
        let (r, mut w) = crate::audio::small_pipe().unwrap();
        let mut child = Command::new(&ff).args(&args).stdin(Stdio::from(r)).stdout(Stdio::null()).spawn().unwrap();
        let t0 = Instant::now();
        let mut frames = 0u64;
        let block = vec![0u8; 1920 * 2 * 4]; // 40 ms de audio, como el hilo de la app
        while t0.elapsed() < Duration::from_secs(10) {
            w.write_all(&block).unwrap();
            frames += 1920;
            if let Some(d) = Duration::from_micros(frames * 1_000_000 / 48000).checked_sub(t0.elapsed()) {
                std::thread::sleep(d);
            }
        }
        use windows::Win32::Foundation::{FILETIME, HANDLE};
        use windows::Win32::System::Threading::GetProcessTimes;
        let (mut a, mut b, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        unsafe {
            let _ = GetProcessTimes(HANDLE(child.as_raw_handle() as _), &mut a, &mut b, &mut k, &mut u);
        }
        let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 10_000.0;
        let cpu = t(k) + t(u);
        let wall = t0.elapsed().as_millis() as f64;
        let _ = child.kill();
        let _ = child.wait();
        eprintln!("Costo fijo de ffmpeg (audio AAC + segmentos, sin el video): {:.1} % de un núcleo", cpu / wall * 100.0);
    }
}

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Jugadón solo funciona en Windows");
}

#[cfg(windows)]
mod app {
    use crate::logic::{self, Config, Hotkey, Source, BG_ALWAYS, BG_GAMES, BG_OFF, DURATIONS, ENCODERS};
    use std::cell::RefCell;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
    use std::time::{Duration, Instant, UNIX_EPOCH};
    use windows::core::{w, PCWSTR, PWSTR};
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::JobObjects::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::ProcessStatus::K32EmptyWorkingSet;
    use windows::Win32::System::Registry::*;
    use windows::Win32::System::SystemInformation::GetLocalTime;
    use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess};
    use windows::Win32::UI::Controls::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::Shell::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    const WM_TRAY: u32 = WM_APP + 1;
    const WM_DONE: u32 = WM_APP + 2;
    const WM_BIND: u32 = WM_APP + 3;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // El juego siempre gana la CPU: grabar en "debajo de lo normal", guardar/capturar en "baja".
    const BELOW_NORMAL: u32 = 0x0000_4000;
    const IDLE: u32 = 0x0000_0040;
    const TIMER_WATCHDOG: usize = 1;
    const TIMER_BIND: usize = 2;
    /// chequeo 1,5 s después de cambiar de ventana (el juego suele pasar a pantalla completa un toque después)
    const TIMER_FG: usize = 3;
    const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const QUICK_DEATH: Duration = Duration::from_secs(5);
    /// Cuánto sigue grabando después de salir del juego (alt-tab corto no pierde el buffer).
    const GAME_GRACE: Duration = Duration::from_secs(60);
    const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;

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

    struct Manual {
        child: Option<Child>,
        enc: usize,
        tried: usize,
        started: Instant,
        t0: Instant,
        ts: PathBuf,
        src: Source,
        what: String,
        audio: bool,
    }

    struct Gallery {
        hwnd: HWND,
        list: HWND,
        buttons: Vec<HWND>,
        font: HFONT,
        big_font: HFONT,
        il: HIMAGELIST,
        files: Vec<PathBuf>,
        /// texto del encabezado (estado, cantidad de clips…)
        header: String,
        sub: String,
    }

    struct App {
        hwnd: HWND,
        cfg: Config,
        cfg_path: PathBuf,
        ffmpeg: PathBuf,
        buf_dir: PathBuf,
        log_path: PathBuf,
        icon_on: HICON,
        icon_off: HICON,
        icon_rec: HICON,
        /// grabación manual en curso (F8 o "Grabar ventana")
        manual: Option<Manual>,
        rec_dir: PathBuf,
        menu_windows: Vec<(HWND, String)>,
        /// ventana "Mis clips": solo existe mientras está abierta (cerrarla libera todo)
        gallery: Option<Gallery>,
        share_token: Option<i64>,
        /// intervalo actual del watchdog: 2 s grabando, 10 s en reposo
        tick_ms: u32,
        rec: Rec,
        binding: u8,
        hook: Option<HHOOK>,
        last_game: Option<Instant>,
        last_trim: Option<Instant>,
        last_file: Option<String>,
        sound: Vec<u8>,
    }

    /// Devuelve la RAM que no se está usando (en reposo queda casi en cero).
    fn trim_ram() {
        unsafe {
            let _ = K32EmptyWorkingSet(GetCurrentProcess());
        }
    }

    /// ¿Hay un juego (o cualquier app) a pantalla completa en el monitor principal?
    fn fullscreen_app() -> bool {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_invalid() || fg == GetShellWindow() || fg == GetDesktopWindow() {
                return false;
            }
            let mut cls = [0u16; 32];
            let n = GetClassNameW(fg, &mut cls).max(0) as usize;
            let cls = String::from_utf16_lossy(&cls[..n]);
            if cls == "WorkerW" || cls == "Progman" {
                return false;
            }
            let mut r = RECT::default();
            if GetWindowRect(fg, &mut r).is_err() {
                return false;
            }
            let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONULL);
            if mon.is_invalid() {
                return false;
            }
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(mon, &mut mi).as_bool() || mi.dwFlags & 1 == 0 {
                return false; // solo el monitor principal (es el que se graba)
            }
            let m = mi.rcMonitor;
            logic::covers((r.left, r.top, r.right, r.bottom), (m.left, m.top, m.right, m.bottom))
        }
    }

    /// Pone el archivo en el portapapeles como archivo (Ctrl+V en Discord, WhatsApp, el Explorador…).
    fn copy_file_to_clipboard(hwnd: HWND, path: &str) -> bool {
        use windows::Win32::System::DataExchange::*;
        use windows::Win32::System::Memory::*;
        let wpath: Vec<u16> = path.encode_utf16().chain([0, 0]).collect();
        let size = 20 + wpath.len() * 2; // DROPFILES (20 bytes) + ruta + doble cero
        unsafe {
            let Ok(h) = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, size) else { return false };
            let p = GlobalLock(h) as *mut u8;
            if p.is_null() {
                let _ = GlobalFree(Some(h));
                return false;
            }
            p.cast::<u32>().write_unaligned(20); // pFiles
            p.add(16).cast::<u32>().write_unaligned(1); // fWide
            std::ptr::copy_nonoverlapping(wpath.as_ptr() as *const u8, p.add(20), wpath.len() * 2);
            let _ = GlobalUnlock(h);
            if OpenClipboard(Some(hwnd)).is_err() {
                let _ = GlobalFree(Some(h));
                return false;
            }
            let _ = EmptyClipboard();
            let ok = SetClipboardData(15 /* CF_HDROP */, Some(HANDLE(h.0))).is_ok();
            let _ = CloseClipboard();
            if !ok {
                let _ = GlobalFree(Some(h));
            }
            ok
        }
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
            let _ = writeln!(f, "[jugadon {t}] {msg}");
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

    /// Lanza un proceso sin ventana y lo mete en el Job (muere si Jugadón muere).
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

    unsafe extern "system" fn enum_cb(hwnd: HWND, lp: LPARAM) -> windows::core::BOOL {
        use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
        let list = &mut *(lp.0 as *mut Vec<(HWND, String)>);
        if list.len() >= 20 || !IsWindowVisible(hwnd).as_bool() || hwnd.0 as isize == MAIN_HWND.load(Ordering::Relaxed) {
            return TRUE;
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 || GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return TRUE;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as _, 4);
        if cloaked != 0 {
            return TRUE;
        }
        let mut buf = [0u16; 128];
        let n = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
        let title = String::from_utf16_lossy(&buf[..n]);
        let title = title.trim();
        if !title.is_empty() && title != "Program Manager" {
            let short: String = title.chars().take(50).collect();
            list.push((hwnd, if short.len() < title.len() { format!("{short}…") } else { short }));
        }
        TRUE
    }

    /// Ventanas abiertas que se pueden grabar (para el submenú "Grabar ventana").
    fn list_windows() -> Vec<(HWND, String)> {
        let mut v: Vec<(HWND, String)> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(enum_cb), LPARAM(&mut v as *mut _ as isize));
        }
        v
    }

    /// Pasa la grabación .ts a .mp4 (prioridad baja) y avisa. Si falla, deja el .ts.
    fn remux_in_background(ffmpeg: PathBuf, log: PathBuf, ts: PathBuf, out: PathBuf, msg: String) {
        std::thread::spawn(move || {
            let args = logic::remux_args(&ts.to_string_lossy(), &out.to_string_lossy());
            let mut cmd = Command::new(ffmpeg);
            cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_stdio(&log));
            let ok = spawn_hidden(&mut cmd, IDLE).and_then(|mut c| c.wait()).is_ok_and(|s| s.success());
            if ok {
                let _ = fs::remove_file(&ts);
                post_text(format!("{msg}\nClic acá para verla."), Some(out.to_string_lossy().into_owned()));
            } else {
                post_text(format!("No pude pasar la grabación a mp4. Quedó en {}", ts.display()), None);
            }
        });
    }

    /// Aviso desde un hilo de trabajo: (texto, archivo creado).
    fn post_text(text: String, file: Option<String>) {
        let hwnd = HWND(MAIN_HWND.load(Ordering::Relaxed) as _);
        let p = Box::into_raw(Box::new((text, file)));
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

        /// ¿Tiene que estar corriendo el buffer de clips en segundo plano?
        fn want_rec(&self) -> bool {
            self.manual.is_none()
                && match self.cfg.bg {
                    BG_OFF => false,
                    BG_ALWAYS => true,
                    _ => self.last_game.is_some_and(|t| t.elapsed() < GAME_GRACE),
                }
        }

        fn status(&self) -> String {
            if let Some(m) = &self.manual {
                let a = if m.audio { "" } else { ", sin audio" };
                return format!("● Grabando {} – {}{a}", m.what, logic::fmt_elapsed(m.t0.elapsed().as_secs()));
            }
            if self.cfg.bg == BG_OFF {
                return "Clips en segundo plano: apagado".to_string();
            }
            if self.rec.child.is_some() && self.rec.good {
                let audio = if !self.rec.audio {
                    ", sin audio"
                } else if self.cfg.mic {
                    " + mic"
                } else {
                    ""
                };
                let cpu = if ENCODERS[self.rec.enc].codec == "libx264" { " – sin encoder de placa: usa CPU" } else { "" };
                format!("Grabando ({}{audio}){cpu}", ENCODERS[self.rec.enc].label)
            } else if self.rec.gave_up {
                if self.cfg.gpu_only {
                    "Sin grabar: la placa no puede comprimir (ver Opciones)".to_string()
                } else {
                    "Sin grabar: error (mirá jugadon.log)".to_string()
                }
            } else if self.rec.child.is_none() && !self.want_rec() {
                "En pausa: esperando un juego".to_string()
            } else {
                "Iniciando…".to_string()
            }
        }

        fn tip(&self) -> String {
            let enc = self.manual.as_ref().map_or(self.rec.enc, |m| m.enc);
            let (h, f) = logic::effective(enc, self.cfg.height, self.cfg.fps);
            format!("Jugadón – {}\n{h}p {f} fps · últimos {} s", self.status(), self.cfg.secs)
        }

        fn icon(&self) -> HICON {
            if self.manual.is_some() {
                self.icon_rec
            } else if self.rec.child.is_some() {
                self.icon_on
            } else {
                self.icon_off
            }
        }

        fn add_icon(&self) {
            let mut n = self.nid();
            n.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            n.uCallbackMessage = WM_TRAY;
            n.hIcon = self.icon();
            copy_wide(&mut n.szTip, &self.tip());
            unsafe {
                let _ = Shell_NotifyIconW(NIM_ADD, &n);
            }
        }

        fn update_tip(&self) {
            let mut n = self.nid();
            n.uFlags = NIF_TIP | NIF_ICON;
            n.hIcon = self.icon();
            copy_wide(&mut n.szTip, &self.tip());
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
            }
        }

        fn notify(&self, msg: &str, warn: bool) {
            let mut n = self.nid();
            n.uFlags = NIF_INFO;
            copy_wide(&mut n.szInfoTitle, "Jugadón");
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
            for (id, h) in [(1, self.cfg.clip), (2, self.cfg.shot), (3, self.cfg.rec)] {
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

        /// Lanza ffmpeg grabando (audio por pipe si se puede). Devuelve (proceso, ¿con audio?).
        fn launch(&self, enc: usize, src: Source, out: logic::Out) -> (Option<Child>, bool) {
            // audio: captura WASAPI + pipe chico hacia el stdin de ffmpeg; si algo falla, solo video
            let audio = match crate::audio::start(self.cfg.mic) {
                Some(a) => crate::audio::small_pipe().map(|p| (a, p)),
                None => None,
            };
            let fmt = audio.as_ref().map(|(a, _)| (a.rate, a.channels));
            let args = logic::record_args(enc, self.cfg.height, self.cfg.fps, src, fmt, out);
            let audio_desc = fmt.map_or("no".to_string(), |f| format!("{} Hz, {} canales", f.0, f.1));
            log_line(&self.log_path, &format!("iniciando {} {} fps (audio: {audio_desc}, fuente: {src:?})", ENCODERS[enc].label, self.cfg.fps));
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
            let has_audio = feed.is_some();
            let child = match spawn_hidden(&mut cmd, BELOW_NORMAL) {
                Ok(child) => {
                    if let Some((a, w)) = feed {
                        a.feed(w);
                    }
                    Some(child)
                }
                Err(e) => {
                    log_line(&self.log_path, &format!("no se pudo lanzar ffmpeg: {e}"));
                    None
                }
            };
            drop(cmd); // cierra nuestra copia del extremo de lectura del pipe
            (child, has_audio)
        }

        fn spawn_rec(&mut self) {
            self.clear_buf();
            let dir = self.buf_dir.to_string_lossy().into_owned();
            let (child, audio) = self.launch(self.rec.enc, Source::Screen, logic::Out::Segments { secs: self.cfg.secs, dir: &dir });
            self.rec.started = Instant::now();
            self.rec.next_try = None;
            self.rec.child = child;
            self.rec.audio = audio;
            self.update_tip();
        }

        // ---------- grabación manual ----------

        fn spawn_manual(&mut self) {
            let Some(m) = self.manual.as_ref() else { return };
            let (enc, src, ts) = (m.enc, m.src, m.ts.to_string_lossy().into_owned());
            let (child, audio) = self.launch(enc, src, logic::Out::File(&ts));
            if let Some(m) = self.manual.as_mut() {
                m.child = child;
                m.audio = audio;
                m.started = Instant::now();
            }
        }

        fn start_manual(&mut self, src: Source, what: String) {
            if self.manual.is_some() {
                return;
            }
            // mientras grabás a mano, el buffer de clips se pausa (no grabar dos veces lo mismo)
            self.stop_rec();
            self.clear_buf();
            let _ = fs::create_dir_all(&self.rec_dir);
            let ts = self.rec_dir.join(format!("grabacion_{}.ts", now_stamp()));
            let now = Instant::now();
            self.manual = Some(Manual { child: None, enc: logic::first_encoder(self.cfg.encoder, self.cfg.gpu_only), tried: 0, started: now, t0: now, ts, src, what, audio: false });
            self.spawn_manual();
            self.play_sound();
            let what = self.manual.as_ref().map(|m| m.what.clone()).unwrap_or_default();
            self.notify(&format!("Grabando {what}. Apretá {} para frenar y guardar.", hk(self.cfg.rec)), false);
            self.update_tip();
        }

        /// Frena la grabación manual y la guarda como mp4.
        fn finish_manual(&mut self, cut: bool) {
            let Some(mut m) = self.manual.take() else { return };
            if let Some(mut c) = m.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            self.update_tip();
            let len = fs::metadata(&m.ts).map(|x| x.len()).unwrap_or(0);
            let Some(dir) = self.out_dir() else {
                self.notify("No pude crear la carpeta de clips.", true);
                return;
            };
            if len == 0 {
                let _ = fs::remove_file(&m.ts);
                self.notify("No se grabó nada (mirá %TEMP%\\jugadon.log).", true);
                return;
            }
            self.play_sound();
            let name = format!("grabacion_{}.mp4", now_stamp());
            let dur = logic::fmt_elapsed(m.t0.elapsed().as_secs());
            let msg = if cut {
                format!("La grabación se cortó; se guardó lo grabado ({dur}): {name}")
            } else {
                format!("Grabación guardada ({dur}): {name}")
            };
            remux_in_background(self.ffmpeg.clone(), self.log_path.clone(), m.ts, dir.join(name), msg);
            self.last_trim = None;
        }

        fn toggle_manual_screen(&mut self) {
            if self.manual.is_some() {
                self.finish_manual(false);
            } else {
                self.start_manual(Source::Screen, "la pantalla".to_string());
            }
        }

        fn record_window(&mut self, idx: usize) {
            let Some((hwnd, title)) = self.menu_windows.get(idx).cloned() else { return };
            unsafe {
                if !IsWindow(Some(hwnd)).as_bool() {
                    self.notify("Esa ventana ya se cerró.", true);
                    return;
                }
                // minimizada no manda imagen: la restauramos (sin traerla adelante)
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
            }
            self.start_manual(Source::Window(hwnd.0 as usize as u64), format!("\"{title}\""));
        }

        /// Grabaciones que quedaron a medias (se cortó la luz, se colgó la PC…): se pasan a mp4.
        fn recover_recordings(&self) {
            let Ok(rd) = fs::read_dir(&self.rec_dir) else { return };
            let Some(dir) = self.out_dir() else { return };
            for (i, e) in rd.flatten().enumerate() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "ts") && e.metadata().is_ok_and(|m| m.len() > 0) {
                    let name = format!("grabacion_recuperada_{}_{i}.mp4", now_stamp());
                    let msg = format!("Recuperé una grabación que había quedado a medias: {name}");
                    remux_in_background(self.ffmpeg.clone(), self.log_path.clone(), p, dir.join(name), msg);
                }
            }
        }

        fn restart_rec(&mut self) {
            self.stop_rec();
            self.rec.tried = 0;
            self.rec.good = false;
            self.rec.gave_up = false;
            self.rec.fails = 0;
            self.rec.enc = logic::first_encoder(self.rec.enc, self.cfg.gpu_only);
            if self.want_rec() {
                self.spawn_rec();
            } else {
                self.update_tip();
            }
        }

        /// Watchdog + cambio de intervalo: 2 s mientras graba, 10 s en reposo (casi no despierta a la CPU).
        fn tick(&mut self) {
            self.tick_inner();
            let busy = self.rec.child.is_some() || self.manual.is_some() || (self.cfg.bg != BG_GAMES && self.want_rec() && !self.rec.gave_up);
            let ms = if busy { 2000 } else { 10_000 };
            if ms != self.tick_ms {
                self.tick_ms = ms;
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_WATCHDOG, ms, None);
                }
            }
        }

        fn tick_inner(&mut self) {
            let now = Instant::now();
            if let Some(m) = self.manual.as_mut() {
                if let Some(c) = m.child.as_mut() {
                    if let Ok(None) = c.try_wait() {
                        self.update_tip(); // muestra el tiempo grabado
                        return;
                    }
                }
                // ffmpeg de la grabación manual terminó solo
                match logic::next_encoder(m.enc, m.tried, self.cfg.gpu_only) {
                    Some((e, t)) if m.started.elapsed() < QUICK_DEATH => {
                        m.enc = e;
                        m.tried = t;
                        self.spawn_manual();
                    }
                    _ => self.finish_manual(true),
                }
                return;
            }
            if self.cfg.bg == BG_GAMES && fullscreen_app() {
                self.last_game = Some(now);
            }
            {
                if !self.want_rec() {
                    // al próximo juego se vuelve a intentar desde cero
                    self.rec.gave_up = false;
                    self.rec.tried = 0;
                    if self.rec.child.is_some() {
                        self.stop_rec();
                        self.clear_buf();
                        log_line(&self.log_path, "clips en segundo plano en pausa");
                        self.update_tip();
                        self.last_trim = None;
                    }
                    // en reposo: devolver la RAM cada tanto
                    if self.last_trim.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
                        self.last_trim = Some(now);
                        trim_ram();
                    }
                    return;
                }
            }
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
                            trim_ram();
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
                    match logic::next_encoder(self.rec.enc, self.rec.tried, self.cfg.gpu_only) {
                        Some((e, t)) => {
                            self.rec.enc = e;
                            self.rec.tried = t;
                            self.rec.next_try = Some(now);
                        }
                        None => {
                            self.rec.gave_up = true;
                            log_line(&self.log_path, "ningún encoder funcionó");
                            if self.cfg.gpu_only {
                                self.notify("Tu placa de video no pudo comprimir el video, así que no grabo (para no usar el procesador). Para grabar igual: menú → Opciones → desmarcá \"Usar solo la placa de video\".", true);
                            } else {
                                self.notify("No pude iniciar la grabación con ningún encoder. Mirá %TEMP%\\jugadon.log", true);
                            }
                            self.update_tip();
                            return;
                        }
                    }
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
                if self.manual.is_some() {
                    self.notify(&format!("Estás grabando a mano: apretá {} para frenar y guardar.", hk(self.cfg.rec)), false);
                } else if self.cfg.bg == BG_OFF {
                    self.notify(&format!("Los clips en segundo plano están apagados (menú → Clips en segundo plano). Para grabar a mano: {}.", hk(self.cfg.rec)), false);
                } else if !self.want_rec() {
                    self.notify("En pausa: los clips se graban solo mientras hay un juego en pantalla completa. Se cambia en el menú → Clips en segundo plano.", false);
                } else {
                    self.notify("No está grabando todavía. Mirá %TEMP%\\jugadon.log", true);
                }
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
            self.play_sound();
            let (ffmpeg, log) = (self.ffmpeg.clone(), self.log_path.clone());
            std::thread::spawn(move || {
                let mut cmd = Command::new(ffmpeg);
                cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_stdio(&log));
                let ok = spawn_hidden(&mut cmd, IDLE).and_then(|mut c| c.wait()).is_ok_and(|s| s.success());
                if ok {
                    post_text(format!("Clip guardado: {name}\nClic acá para verlo."), Some(out));
                } else {
                    post_text("No se pudo guardar el clip. Mirá %TEMP%\\jugadon.log".into(), None);
                }
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
            self.play_sound();
            let (ffmpeg, log) = (self.ffmpeg.clone(), self.log_path.clone());
            std::thread::spawn(move || {
                let ffmpeg2 = ffmpeg.clone();
                let mut cmd = Command::new(ffmpeg);
                cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_stdio(&log));
                let mut ok = spawn_hidden(&mut cmd, IDLE).and_then(|mut c| c.wait()).is_ok_and(|s| s.success());
                if !ok {
                    // driver viejo sin Desktop Duplication: por GDI
                    let mut cmd = Command::new(&ffmpeg2);
                    cmd.args(logic::shot_args_gdi(&out)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_stdio(&log));
                    ok = spawn_hidden(&mut cmd, IDLE).and_then(|mut c| c.wait()).is_ok_and(|s| s.success());
                }
                if ok {
                    post_text(format!("Captura guardada: {name}\nClic acá para verla."), Some(out));
                } else {
                    post_text("No se pudo sacar la captura. Mirá %TEMP%\\jugadon.log".into(), None);
                }
            });
        }

        fn play_sound(&self) {
            if self.cfg.sound {
                use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
                unsafe {
                    let _ = PlaySoundW(PCWSTR(self.sound.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
                }
            }
        }

        fn newest_clip(&self) -> Option<PathBuf> {
            fs::read_dir(&self.cfg.folder)
                .ok()?
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "mp4"))
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .max_by_key(|(t, _)| *t)
                .map(|(_, p)| p)
        }

        fn copy_last_clip(&self) {
            match self.newest_clip() {
                Some(p) if copy_file_to_clipboard(self.hwnd, &p.to_string_lossy()) => {
                    self.notify("Último clip copiado. Pegalo con Ctrl+V en Discord, WhatsApp o donde quieras.", false)
                }
                Some(_) => self.notify("No pude copiar el clip.", true),
                None => self.notify("Todavía no hay clips guardados.", false),
            }
        }

        fn show_last_file(&self) {
            if let Some(f) = &self.last_file {
                let _ = Command::new("explorer.exe").raw_arg(format!("/select,\"{f}\"")).spawn();
            }
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
            let what = ["", "guardar clip", "captura", "grabar/frenar"][which as usize];
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
            let keys = [self.cfg.clip, self.cfg.shot, self.cfg.rec];
            if keys.iter().enumerate().any(|(i, k)| i + 1 != which as usize && *k == new) {
                self.notify(&format!("{} ya está usada para otra acción.", hk(new)), true);
                return;
            }
            match which {
                1 => self.cfg.clip = new,
                2 => self.cfg.shot = new,
                _ => self.cfg.rec = new,
            }
            self.save_cfg();
            self.register_keys();
            let what = ["", "Guardar clip", "Captura", "Grabar/frenar"][which as usize];
            self.notify(&format!("{what}: {}", hk(new)), false);
        }

        fn build_menu(&mut self) -> HMENU {
            self.menu_windows = if self.manual.is_none() { list_windows() } else { Vec::new() };
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
                let t = wide(&self.status());
                let _ = AppendMenuW(m, MF_STRING | MF_GRAYED, 0, PCWSTR(t.as_ptr()));
                sep(m);
                add(m, 5, "Ver mis clips…", false);
                add(m, 1, &format!("Guardar clip\t{}", hk(self.cfg.clip)), false);
                add(m, 2, &format!("Captura\t{}", hk(self.cfg.shot)), false);
                if self.manual.is_some() {
                    add(m, 4, &format!("■ Frenar y guardar grabación\t{}", hk(self.cfg.rec)), false);
                } else {
                    add(m, 4, &format!("● Grabar pantalla completa\t{}", hk(self.cfg.rec)), false);
                    let w = CreatePopupMenu().unwrap_or_default();
                    if self.menu_windows.is_empty() {
                        let t = wide("(no hay ventanas)");
                        let _ = AppendMenuW(w, MF_STRING | MF_GRAYED, 0, PCWSTR(t.as_ptr()));
                    }
                    for (i, (_, title)) in self.menu_windows.iter().enumerate() {
                        add(w, 100 + i, &title.replace('&', "&&"), false);
                    }
                    let t = wide("● Grabar una ventana");
                    let _ = AppendMenuW(m, MF_POPUP, w.0 as usize, PCWSTR(t.as_ptr()));
                }
                add(m, 3, "Copiar último clip (para pegar en Discord)", false);
                sep(m);
                let bg = CreatePopupMenu().unwrap_or_default();
                add(bg, 46, "Solo mientras juego (recomendado)", self.cfg.bg == BG_GAMES);
                add(bg, 47, "Siempre", self.cfg.bg == BG_ALWAYS);
                add(bg, 45, "Apagado (solo grabo a mano)", self.cfg.bg == BG_OFF);
                let t = wide("Clips en segundo plano");
                let _ = AppendMenuW(m, MF_POPUP, bg.0 as usize, PCWSTR(t.as_ptr()));
                let dur = CreatePopupMenu().unwrap_or_default();
                for (i, s) in DURATIONS.iter().enumerate() {
                    let label = if *s < 60 { format!("{s} s") } else { format!("{} min", s / 60) };
                    add(dur, 10 + i, &label, self.cfg.secs == *s);
                }
                let t = wide("Duración del clip");
                let _ = AppendMenuW(m, MF_POPUP, dur.0 as usize, PCWSTR(t.as_ptr()));
                let q = CreatePopupMenu().unwrap_or_default();
                let (h, f) = (self.cfg.height, self.cfg.fps);
                add(q, 23, "Automática: lo justo para tu PC (recomendada)", h == 0);
                add(q, 22, "480p 30 fps (ahorro máximo)", h == 480 && f == 30);
                add(q, 20, "720p 30 fps", h == 720 && f == 30);
                add(q, 21, "720p 60 fps", h == 720 && f == 60);
                let t = wide("Calidad");
                let _ = AppendMenuW(m, MF_POPUP, q.0 as usize, PCWSTR(t.as_ptr()));
                let o = CreatePopupMenu().unwrap_or_default();
                add(o, 43, "Grabar micrófono", self.cfg.mic);
                add(o, 44, "Sonido al guardar", self.cfg.sound);
                add(o, 48, "Usar solo la placa de video (nunca el procesador)", self.cfg.gpu_only);
                sep(o);
                add(o, 30, "Tecla de clip…", false);
                add(o, 31, "Tecla de captura…", false);
                add(o, 32, "Tecla de grabar/frenar…", false);
                sep(o);
                add(o, 41, "Iniciar con Windows", autostart_enabled());
                let t = wide("Opciones");
                let _ = AppendMenuW(m, MF_POPUP, o.0 as usize, PCWSTR(t.as_ptr()));
                sep(m);
                add(m, 40, "Abrir carpeta", false);
                add(m, 50, "Salir", false);
                m
            }
        }

        /// Devuelve true si hay que salir.
        fn command(&mut self, id: usize) -> bool {
            match id {
                1 => self.save_clip(),
                2 => self.screenshot(),
                3 => self.copy_last_clip(),
                4 => self.toggle_manual_screen(),
                5 => self.open_gallery(),
                100..=119 => self.record_window(id - 100),
                45..=47 => {
                    let bg = [BG_OFF, BG_GAMES, BG_ALWAYS][id - 45];
                    if bg != self.cfg.bg {
                        self.cfg.bg = bg;
                        self.save_cfg();
                        self.notify(
                            match bg {
                                BG_OFF => "Clips en segundo plano apagados: no se graba nada hasta que apretes grabar.",
                                BG_GAMES => "Los clips se graban solo mientras hay un juego en pantalla completa.",
                                _ => "Los clips se graban siempre.",
                            },
                            false,
                        );
                        self.restart_rec();
                    }
                }
                43 => {
                    self.cfg.mic = !self.cfg.mic;
                    self.save_cfg();
                    self.restart_rec();
                }
                48 => {
                    self.cfg.gpu_only = !self.cfg.gpu_only;
                    self.save_cfg();
                    self.notify(
                        if self.cfg.gpu_only {
                            "Solo placa de video: si tu placa no puede comprimir, no graba (el juego no pierde nada)."
                        } else {
                            "Si la placa no puede, se usa el procesador (puede bajar algo de FPS en PCs lentas)."
                        },
                        false,
                    );
                    self.restart_rec();
                }
                44 => {
                    self.cfg.sound = !self.cfg.sound;
                    self.save_cfg();
                    self.play_sound();
                }
                10..=14 => {
                    let s = DURATIONS[id - 10];
                    if s != self.cfg.secs {
                        self.cfg.secs = s;
                        self.save_cfg();
                        self.restart_rec();
                    }
                }
                20..=23 => {
                    let (height, fps) = [(720, 30), (720, 60), (480, 30), (0, 30)][id - 20];
                    if (height, fps) != (self.cfg.height, self.cfg.fps) {
                        self.cfg.height = height;
                        self.cfg.fps = fps;
                        self.save_cfg();
                        self.restart_rec();
                    }
                }
                30 => self.start_bind(1),
                31 => self.start_bind(2),
                32 => self.start_bind(3),
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
            // una grabación manual en curso se guarda antes de salir
            if let Some(mut m) = self.manual.take() {
                if let Some(mut c) = m.child.take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
                if let Some(dir) = self.out_dir() {
                    let out = dir.join(format!("grabacion_{}.mp4", now_stamp()));
                    let args = logic::remux_args(&m.ts.to_string_lossy(), &out.to_string_lossy());
                    let mut cmd = Command::new(&self.ffmpeg);
                    cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_stdio(&self.log_path));
                    if spawn_hidden(&mut cmd, 0).and_then(|mut c| c.wait()).is_ok_and(|s| s.success()) {
                        let _ = fs::remove_file(&m.ts);
                    }
                }
            }
            self.stop_rec();
            self.clear_buf();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &self.nid());
            }
        }
    }

    // ---------- ventana "Mis clips" ----------

    const GALLERY_BUTTONS: [(usize, &str); 6] = [
        (201, "▶  Abrir"),
        (202, "Compartir…"),
        (203, "Copiar (para Discord)"),
        (204, "Mostrar en carpeta"),
        (205, "Eliminar"),
        (206, "Abrir carpeta"),
    ];

    fn send(h: HWND, msg: u32, wp: usize, lp: isize) -> isize {
        unsafe { SendMessageW(h, msg, Some(WPARAM(wp)), Some(LPARAM(lp))).0 }
    }

    /// Abre la ventana de compartir de Windows con el archivo.
    fn share_file(hwnd: HWND, path: String, token: &mut Option<i64>) -> windows::core::Result<()> {
        use windows::ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager};
        use windows::Foundation::TypedEventHandler;
        use windows::Storage::{IStorageItem, StorageFile};
        use windows::core::{Interface, HSTRING};
        unsafe {
            let interop: IDataTransferManagerInterop = windows::core::factory::<DataTransferManager, IDataTransferManagerInterop>()?;
            let dtm: DataTransferManager = interop.GetForWindow(hwnd)?;
            if let Some(t) = token.take() {
                let _ = dtm.RemoveDataRequested(t);
            }
            let handler = TypedEventHandler::<DataTransferManager, DataRequestedEventArgs>::new(move |_, args| {
                let Some(args) = args.as_ref() else { return Ok(()) };
                let data = args.Request()?.Data()?;
                let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                data.Properties()?.SetTitle(&HSTRING::from(name))?;
                let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_str()))?.join()?;
                let items: windows_collections::IIterable<IStorageItem> = vec![Some(file.cast::<IStorageItem>()?)].into();
                data.SetStorageItemsReadOnly(&items)?;
                Ok(())
            });
            *token = Some(dtm.DataRequested(&handler)?);
            interop.ShowShareUIForWindow(hwnd)
        }
    }

    impl App {
        fn open_gallery(&mut self) {
            unsafe {
                if let Some(g) = &self.gallery {
                    let _ = ShowWindow(g.hwnd, SW_RESTORE);
                    let _ = SetForegroundWindow(g.hwnd);
                    return;
                }
                let hinst: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
                let dpi = windows::Win32::UI::HiDpi::GetDpiForSystem().max(96) as i32;
                let sc = |v: i32| v * dpi / 96;
                let Ok(hwnd) = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("jugadon_gallery"),
                    w!("Jugadón – Mis clips"),
                    WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    sc(900),
                    sc(620),
                    None,
                    None,
                    Some(hinst),
                    None,
                ) else {
                    return;
                };
                dark_title_bar(hwnd);
                let _ = SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_SMALL as usize)), Some(LPARAM(self.icon_on.0 as isize)));
                let big = load_icon_size(hinst, 1, GetSystemMetrics(SM_CXICON));
                let _ = SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_BIG as usize)), Some(LPARAM(big.0 as isize)));
                // fuentes del sistema (Segoe UI): normal y grande en negrita para el título
                let mut ncm = NONCLIENTMETRICSW { cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
                let _ = SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, ncm.cbSize, Some(&mut ncm as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
                let mut lf = ncm.lfMessageFont;
                lf.lfHeight = -sc(15);
                let font = CreateFontIndirectW(&lf);
                lf.lfHeight = -sc(30);
                lf.lfWeight = 800;
                let big_font = CreateFontIndirectW(&lf);
                let child = |class: PCWSTR, text: &str, style: u32, id: usize| -> HWND {
                    let t = wide(text);
                    let h = CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        class,
                        PCWSTR(t.as_ptr()),
                        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | style),
                        0,
                        0,
                        10,
                        10,
                        Some(hwnd),
                        Some(HMENU(id as _)),
                        Some(hinst),
                        None,
                    )
                    .unwrap_or_default();
                    let _ = SendMessageW(h, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
                    h
                };
                // botones dibujados por nosotros (naranja / oscuro)
                let buttons = GALLERY_BUTTONS.iter().map(|(id, t)| child(w!("BUTTON"), t, BS_OWNERDRAW as u32, *id)).collect();
                let list = child(w!("SysListView32"), "", LVS_ICON | LVS_SHOWSELALWAYS | LVS_SINGLESEL | LVS_AUTOARRANGE | WS_TABSTOP.0, 300);
                let _ = SetWindowTheme(list, w!("DarkMode_Explorer"), PCWSTR::null());
                send(list, LVM_SETEXTENDEDLISTVIEWSTYLE, 0, LVS_EX_DOUBLEBUFFER as isize);
                send(list, LVM_SETBKCOLOR, 0, BG_COLOR as isize);
                send(list, LVM_SETTEXTBKCOLOR, 0, BG_COLOR as isize);
                send(list, LVM_SETTEXTCOLOR, 0, 0x00F0_EBEB);
                let (tw, th) = thumb_size(dpi);
                let il = ImageList_Create(tw, th, ILC_COLOR32, 32, 32);
                add_placeholders(il, tw, th, self.icon_on);
                send(list, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, il.0 as isize);
                send(list, LVM_SETICONSPACING, 0, ((tw + sc(28)) as isize) | (((th + sc(52)) as isize) << 16));
                self.gallery = Some(Gallery { hwnd, list, buttons, font, big_font, il, files: Vec::new(), header: String::new(), sub: String::new() });
                self.layout_gallery();
                self.fill_gallery();
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
            }
        }

        fn layout_gallery(&self) {
            let Some(g) = &self.gallery else { return };
            unsafe {
                let mut r = RECT::default();
                let _ = GetClientRect(g.hwnd, &mut r);
                let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(g.hwnd).max(96) as i32;
                let sc = |v: i32| v * dpi / 96;
                let (m, bh) = (sc(12), sc(34));
                let top = header_height(dpi) + m;
                let mut x = m;
                for b in &g.buttons {
                    let mut t = [0u16; 64];
                    let n = GetWindowTextW(*b, &mut t).max(0);
                    let bw = sc(28) + n * sc(7);
                    let _ = MoveWindow(*b, x, top, bw, bh, true);
                    x += bw + sc(8);
                }
                let ltop = top + bh + m;
                let _ = MoveWindow(g.list, m, ltop, r.right - 2 * m, (r.bottom - ltop - m).max(0), true);
            }
        }

        /// Lee la carpeta de clips (solo nombres y fechas) y pide las miniaturas en segundo plano.
        fn fill_gallery(&mut self) {
            let folder = self.cfg.folder.clone();
            let status = self.status();
            let keys = format!(
                "{} guardar clip   ·   {} grabar / frenar   ·   {} captura",
                hk(self.cfg.clip),
                hk(self.cfg.rec),
                hk(self.cfg.shot)
            );
            let icon = self.icon_on;
            let Some(g) = self.gallery.as_mut() else { return };
            let mut files: Vec<(std::time::SystemTime, PathBuf, u64)> = fs::read_dir(&folder)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| e.path().extension().is_some_and(|x| x == "mp4" || x == "png"))
                        .filter_map(|e| {
                            let m = e.metadata().ok()?;
                            Some((m.modified().ok()?, e.path(), m.len()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            files.sort_by_key(|f| std::cmp::Reverse(f.0));
            files.truncate(2000);
            let total: u64 = files.iter().map(|f| f.2).sum();
            g.header = if files.is_empty() {
                format!("{status}   ·   Todavía no hay clips: abrí un juego y apretá {}", hk(self.cfg.clip))
            } else {
                format!("{status}   ·   {} archivos · {}", files.len(), logic::fmt_size(total))
            };
            g.sub = keys;
            unsafe {
                let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(g.hwnd).max(96) as i32;
                let (tw, th) = thumb_size(dpi);
                send(g.list, LVM_DELETEALLITEMS, 0, 0);
                let _ = ImageList_Remove(g.il, -1);
                add_placeholders(g.il, tw, th, icon);
                for (i, (_, path, len)) in files.iter().enumerate() {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let (kind, date) = logic::describe_file(&name);
                    let label = if date.is_empty() { format!("{name}\n{}", logic::fmt_size(*len)) } else { format!("{kind} · {date}\n{}", logic::fmt_size(*len)) };
                    let tn = wide(&label);
                    let item = LVITEMW { mask: LVIF_TEXT | LVIF_IMAGE, iItem: i as i32, pszText: PWSTR(tn.as_ptr() as _), iImage: 0, ..Default::default() };
                    send(g.list, LVM_INSERTITEMW, 0, &item as *const _ as isize);
                }
                let mut hr = RECT::default();
                let _ = GetClientRect(g.hwnd, &mut hr);
                hr.bottom = header_height(dpi);
                let _ = InvalidateRect(Some(g.hwnd), Some(&hr), false);
                start_thumbnails(g.hwnd, files.iter().map(|f| f.1.clone()).collect(), tw, th);
            }
            g.files = files.into_iter().map(|f| f.1).collect();
        }

        /// Llega una miniatura del hilo de fondo.
        fn add_thumb(&mut self, idx: usize, gen: u64, hbm: HBITMAP) {
            unsafe {
                if let Some(g) = &self.gallery {
                    if gen == THUMB_GEN.load(Ordering::Relaxed) {
                        let img = ImageList_Add(g.il, hbm, None);
                        if img >= 0 {
                            let item = LVITEMW { mask: LVIF_IMAGE, iItem: idx as i32, iImage: img, ..Default::default() };
                            send(g.list, LVM_SETITEMW, 0, &item as *const _ as isize);
                        }
                    }
                }
                let _ = DeleteObject(hbm.into());
            }
        }

        fn selected_file(&self) -> Option<PathBuf> {
            let g = self.gallery.as_ref()?;
            let i = send(g.list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize);
            g.files.get(usize::try_from(i).ok()?).cloned()
        }

        fn gallery_cmd(&mut self, id: usize) {
            if id == 206 {
                if let Some(d) = self.out_dir() {
                    let _ = Command::new("explorer.exe").arg(d).spawn();
                }
                return;
            }
            let Some(f) = self.selected_file() else {
                self.notify("Elegí un clip de la lista primero.", false);
                return;
            };
            let fs_ = f.to_string_lossy().into_owned();
            match id {
                201 => {
                    let p = wide(&fs_);
                    unsafe {
                        ShellExecuteW(None, w!("open"), PCWSTR(p.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
                    }
                }
                202 => {
                    let hwnd = self.gallery.as_ref().map(|g| g.hwnd).unwrap_or(self.hwnd);
                    let mut token = self.share_token.take();
                    let r = share_file(hwnd, fs_.clone(), &mut token);
                    self.share_token = token;
                    if r.is_err() && copy_file_to_clipboard(self.hwnd, &fs_) {
                        self.notify("No se pudo abrir Compartir de Windows: lo copié, pegalo con Ctrl+V donde quieras.", false);
                    }
                }
                203 => {
                    if copy_file_to_clipboard(self.hwnd, &fs_) {
                        self.notify("Copiado. Pegalo con Ctrl+V en Discord, WhatsApp o donde quieras.", false);
                    }
                }
                204 => {
                    let _ = Command::new("explorer.exe").raw_arg(format!("/select,\"{fs_}\"")).spawn();
                }
                205 => {
                    // a la papelera (Windows pregunta antes)
                    let from: Vec<u16> = fs_.encode_utf16().chain([0, 0]).collect();
                    let hwnd = self.gallery.as_ref().map(|g| g.hwnd).unwrap_or(self.hwnd);
                    let mut op = SHFILEOPSTRUCTW {
                        hwnd,
                        wFunc: FO_DELETE,
                        pFrom: PCWSTR(from.as_ptr()),
                        fFlags: FOF_ALLOWUNDO.0 as u16,
                        ..Default::default()
                    };
                    unsafe {
                        let _ = SHFileOperationW(&mut op);
                    }
                    self.fill_gallery();
                }
                _ => {}
            }
        }
    }

    // ---- aspecto de la ventana (colores, encabezado, botones, miniaturas) ----

    const BG_COLOR: u32 = 0x001E_1618; // RGB(24, 22, 30) en formato COLORREF (0x00BBGGRR)
    const WM_THUMB: u32 = WM_APP + 10;
    static THUMB_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn header_height(dpi: i32) -> i32 {
        84 * dpi / 96
    }

    fn thumb_size(dpi: i32) -> (i32, i32) {
        (192 * dpi / 96, 108 * dpi / 96)
    }

    /// Barra de título oscura (Windows 10 2004+ / 11) y del color del fondo en Windows 11.
    fn dark_title_bar(hwnd: HWND) {
        use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
        unsafe {
            let on: u32 = 1;
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &on as *const u32 as _, 4);
            let caption: u32 = BG_COLOR;
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(35), &caption as *const u32 as _, 4);
        }
    }

    /// Bitmap 32 bits de w×h relleno de color de fondo. Devuelve (bitmap, puntero a los píxeles).
    unsafe fn dark_dib(w: i32, h: i32) -> Option<(HBITMAP, *mut u32)> {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bmp = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let px = bits as *mut u32;
        for i in 0..(w * h) as usize {
            *px.add(i) = 0xFF2A_2633;
        }
        Some((bmp, px))
    }

    /// Las ImageList de 32 bits usan el canal alfa: dejamos todo opaco.
    unsafe fn opaque(px: *mut u32, n: usize) {
        let _ = GdiFlush();
        for i in 0..n {
            *px.add(i) |= 0xFF00_0000;
        }
    }

    /// Imagen 0 de la lista: el logo sobre fondo oscuro (mientras carga la miniatura).
    unsafe fn add_placeholders(il: HIMAGELIST, w: i32, h: i32, icon: HICON) {
        if let Some((bmp, px)) = dark_dib(w, h) {
            let dc = CreateCompatibleDC(None);
            let old = SelectObject(dc, bmp.into());
            let s = h / 2;
            let _ = DrawIconEx(dc, (w - s) / 2, (h - s) / 2, icon, s, s, 0, None, DI_NORMAL);
            SelectObject(dc, old);
            let _ = DeleteDC(dc);
            opaque(px, (w * h) as usize);
            ImageList_Add(il, bmp, None);
            let _ = DeleteObject(bmp.into());
        }
    }

    /// Miniatura del Explorador (usa su caché; no decodifica el video si ya la tiene), centrada en w×h.
    unsafe fn make_thumb(path: &Path, w: i32, h: i32) -> Option<HBITMAP> {
        let p = wide(&path.to_string_lossy());
        let f: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(p.as_ptr()), None).ok()?;
        let src = f.GetImage(SIZE { cx: w, cy: h }, SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY).ok()?;
        let mut bm = BITMAP::default();
        GetObjectW(src.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as _));
        let (sw, sh) = (bm.bmWidth.min(w), bm.bmHeight.abs().min(h));
        let (dst, px) = dark_dib(w, h)?;
        let d1 = CreateCompatibleDC(None);
        let d2 = CreateCompatibleDC(None);
        let o1 = SelectObject(d1, dst.into());
        let o2 = SelectObject(d2, src.into());
        let _ = BitBlt(d1, (w - sw) / 2, (h - sh) / 2, sw, sh, Some(d2), 0, 0, SRCCOPY);
        SelectObject(d1, o1);
        SelectObject(d2, o2);
        let _ = DeleteDC(d1);
        let _ = DeleteDC(d2);
        let _ = DeleteObject(src.into());
        opaque(px, (w * h) as usize);
        Some(dst)
    }

    /// Carga miniaturas en un hilo con prioridad de fondo (CPU y disco bajos). Se corta si la lista cambia o se cierra.
    fn start_thumbnails(hwnd: HWND, files: Vec<PathBuf>, w: i32, h: i32) {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
        let gen = THUMB_GEN.fetch_add(1, Ordering::Relaxed) + 1;
        let hw = hwnd.0 as isize;
        std::thread::spawn(move || unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            for (i, p) in files.iter().enumerate().take(400) {
                if THUMB_GEN.load(Ordering::Relaxed) != gen {
                    break;
                }
                if let Some(bmp) = make_thumb(p, w, h) {
                    let wp = (i & 0xFFFF) | ((gen as usize & 0xFFFF) << 16);
                    if PostMessageW(Some(HWND(hw as _)), WM_THUMB, WPARAM(wp), LPARAM(bmp.0 as isize)).is_err() {
                        let _ = DeleteObject(bmp.into());
                        break;
                    }
                }
            }
            CoUninitialize();
        });
    }

    /// Encabezado: degradé rosa → naranja, logo, título y estado.
    unsafe fn paint_header(hwnd: HWND, dc: HDC) {
        let Some((header, sub, big_font, font)) = with_app(|a| a.gallery.as_ref().map(|g| (g.header.clone(), g.sub.clone(), g.big_font, g.font))).flatten() else {
            return;
        };
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as i32;
        let sc = |v: i32| v * dpi / 96;
        let mut r = RECT::default();
        let _ = GetClientRect(hwnd, &mut r);
        let hh = header_height(dpi);
        let v = [
            TRIVERTEX { x: 0, y: 0, Red: 0xFF00, Green: 0x2E00, Blue: 0x6300, Alpha: 0 },
            TRIVERTEX { x: r.right, y: hh, Red: 0xFF00, Green: 0x9A00, Blue: 0x0000, Alpha: 0 },
        ];
        let gr = GRADIENT_RECT { UpperLeft: 0, LowerRight: 1 };
        let _ = GradientFill(dc, &v, &gr as *const _ as _, 1, GRADIENT_FILL_RECT_H);
        let icon = with_app(|a| a.icon_on).unwrap_or_default();
        let isz = sc(52);
        let _ = DrawIconEx(dc, sc(16), (hh - isz) / 2, icon, isz, isz, 0, None, DI_NORMAL);
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let old = SelectObject(dc, big_font.into());
        let t = wide("Jugadón");
        let _ = TextOutW(dc, sc(80), sc(10), &t[..t.len() - 1]);
        SelectObject(dc, font.into());
        let t: Vec<u16> = header.encode_utf16().collect();
        let _ = TextOutW(dc, sc(82), sc(46), &t);
        let t: Vec<u16> = sub.encode_utf16().collect();
        SetTextColor(dc, COLORREF(0x00F5_E8FF));
        let _ = TextOutW(dc, sc(82), sc(64), &t);
        SelectObject(dc, old);
    }

    /// Botones redondeados: los dos primeros naranjas, el resto oscuros.
    unsafe fn draw_button(di: &DRAWITEMSTRUCT) {
        let pressed = di.itemState.0 & ODS_SELECTED.0 != 0;
        let accent = di.CtlID == 201 || di.CtlID == 202;
        let color: u32 = match (accent, pressed) {
            (true, false) => 0x0028_6AFF,
            (true, true) => 0x0018_4ACC,
            (false, false) => 0x0042_3A3E,
            (false, true) => 0x002E_282B,
        };
        let r = di.rcItem;
        let bg = CreateSolidBrush(COLORREF(BG_COLOR));
        FillRect(di.hDC, &r, bg);
        let _ = DeleteObject(bg.into());
        let br = CreateSolidBrush(COLORREF(color));
        let pen = CreatePen(PS_SOLID, 1, COLORREF(color));
        let ob = SelectObject(di.hDC, br.into());
        let op = SelectObject(di.hDC, pen.into());
        let rad = (r.bottom - r.top) / 2;
        let _ = RoundRect(di.hDC, r.left, r.top, r.right, r.bottom, rad, rad);
        SelectObject(di.hDC, ob);
        SelectObject(di.hDC, op);
        let _ = DeleteObject(br.into());
        let _ = DeleteObject(pen.into());
        let mut t = [0u16; 64];
        let n = GetWindowTextW(di.hwndItem, &mut t).max(0) as usize;
        SetBkMode(di.hDC, TRANSPARENT);
        SetTextColor(di.hDC, COLORREF(0x00FF_FFFF));
        let mut rr = r;
        DrawTextW(di.hDC, &mut t[..n], &mut rr, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }

    unsafe extern "system" fn gallery_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                paint_header(hwnd, dc);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_DRAWITEM => {
                draw_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                LRESULT(1)
            }
            WM_SIZE => {
                with_app(|a| a.layout_gallery());
                let _ = InvalidateRect(Some(hwnd), None, true);
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let mi = &mut *(lp.0 as *mut MINMAXINFO);
                mi.ptMinTrackSize = POINT { x: 760, y: 420 };
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = wp.0 & 0xFFFF;
                with_app(|a| a.gallery_cmd(id));
                LRESULT(0)
            }
            WM_NOTIFY => {
                let h = &*(lp.0 as *const NMHDR);
                if h.idFrom == 300 && h.code == NM_DBLCLK {
                    with_app(|a| a.gallery_cmd(201));
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_THUMB => {
                let bmp = HBITMAP(lp.0 as _);
                let (idx, gen) = (wp.0 & 0xFFFF, ((wp.0 >> 16) & 0xFFFF) as u64);
                let cur = THUMB_GEN.load(Ordering::Relaxed) & 0xFFFF;
                if with_app(|a| a.add_thumb(idx, if gen == cur { THUMB_GEN.load(Ordering::Relaxed) } else { u64::MAX }, bmp)).is_none() {
                    let _ = DeleteObject(bmp.into());
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // cerrar con la X: se libera la ventana entera (miniaturas incluidas) y Jugadón sigue en la bandeja
                THUMB_GEN.fetch_add(1, Ordering::Relaxed);
                with_app(|a| {
                    if let Some(g) = a.gallery.take() {
                        let _ = DeleteObject(g.font.into());
                        let _ = DeleteObject(g.big_font.into());
                    }
                    a.share_token = None;
                });
                trim_ram();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
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
            let r = RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Jugadon"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut len));
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
                let _ = RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Jugadon"), REG_SZ.0, Some(v.as_ptr() as _), (v.len() * 2) as u32);
            } else {
                let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Jugadon"));
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

    /// Windows avisa cuando cambia la ventana activa: así no hace falta preguntar seguido si hay un juego.
    unsafe extern "system" fn foreground_changed(
        _: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
        _: u32,
        _: HWND,
        _: i32,
        _: i32,
        _: u32,
        _: u32,
    ) {
        let hwnd = HWND(MAIN_HWND.load(Ordering::Relaxed) as _);
        SetTimer(Some(hwnd), TIMER_FG, 1500, None);
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
                if m == WM_LBUTTONUP {
                    with_app(|a| a.open_gallery());
                } else if m == WM_RBUTTONUP {
                    show_menu(hwnd);
                } else if m == NIN_BALLOONUSERCLICK {
                    with_app(|a| a.show_last_file());
                }
                LRESULT(0)
            }
            WM_HOTKEY => {
                with_app(|a| match wp.0 {
                    1 => a.save_clip(),
                    2 => a.screenshot(),
                    3 => a.toggle_manual_screen(),
                    _ => {}
                });
                LRESULT(0)
            }
            WM_TIMER => {
                if wp.0 == TIMER_WATCHDOG {
                    with_app(|a| a.tick());
                } else if wp.0 == TIMER_FG {
                    let _ = KillTimer(Some(hwnd), TIMER_FG);
                    with_app(|a| a.tick());
                } else if wp.0 == TIMER_BIND {
                    with_app(|a| a.finish_bind(0, 0));
                }
                LRESULT(0)
            }
            WM_DONE => {
                let (text, file) = *Box::from_raw(lp.0 as *mut (String, Option<String>));
                with_app(|a| {
                    a.notify(&text, text.starts_with("No "));
                    if file.is_some() {
                        a.last_file = file;
                        a.fill_gallery();
                    }
                });
                trim_ram();
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

    unsafe fn load_icon_size(hinst: HINSTANCE, id: usize, size: i32) -> HICON {
        match LoadImageW(Some(hinst), PCWSTR(id as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR) {
            Ok(h) if !h.is_invalid() => HICON(h.0),
            _ => make_icon(),
        }
    }

    /// Ícono del exe (recurso 1); si no está, un punto de color generado.
    unsafe fn load_icon(hinst: HINSTANCE, id: usize) -> HICON {
        let size = GetSystemMetrics(SM_CXSMICON);
        match LoadImageW(Some(hinst), PCWSTR(id as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR) {
            Ok(h) if !h.is_invalid() => HICON(h.0),
            _ => make_icon(),
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
            MessageBoxW(None, PCWSTR(t.as_ptr()), w!("Jugadón"), MB_OK | MB_ICONWARNING);
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
            // píxeles reales (ventanas, pantalla e ícono bien medidos con escalado de Windows al 125/150 %)
            let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let _mutex = CreateMutexW(None, true, w!("Local\\jugadon_single_instance"));
            if GetLastError() == ERROR_ALREADY_EXISTS {
                message_box("Jugadón ya está abierto (mirá la bandeja del sistema, al lado del reloj).");
                return;
            }

            let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
            let ffmpeg = exe_dir.join("ffmpeg.exe");
            if !ffmpeg.exists() {
                message_box("Falta ffmpeg.exe al lado de jugadon.exe. Descomprimí el zip completo en una carpeta.");
                return;
            }
            let temp = std::env::temp_dir();
            let log_path = temp.join("jugadon.log");
            if fs::metadata(&log_path).is_ok_and(|m| m.len() > 1_000_000) {
                let _ = File::create(&log_path);
            }
            log_line(&log_path, "inicio");
            let appdata = env_path("APPDATA", ".");
            let cfg_path = appdata.join("Jugadon").join("config.ini");
            // config de la versión anterior (rclip), si existe
            let old_cfg = appdata.join("rclip").join("config.ini");
            let default_folder = env_path("USERPROFILE", ".").join("Videos").join("Jugadon").to_string_lossy().into_owned();
            let cfg = match fs::read_to_string(&cfg_path).or_else(|_| fs::read_to_string(&old_cfg)) {
                Ok(t) => Config::parse(&t, default_folder),
                Err(_) => Config::defaults(default_folder),
            };
            let _ = fs::create_dir_all(&cfg.folder);
            make_job();

            // si el autoarranque apunta a otra ruta (movieron la carpeta), lo actualizamos;
            // y si estaba el de la versión anterior (rclip), lo pasamos al nombre nuevo
            let old_run = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("rclip")).is_ok();
            if old_run || autostart_value().is_some_and(|v| v != exe_cmdline()) {
                set_autostart(true);
            }

            let hinst = GetModuleHandleW(None).unwrap_or_default();
            let class = w!("jugadon_tray");
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            let _ = windows::Win32::UI::Controls::InitCommonControlsEx(&windows::Win32::UI::Controls::INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<windows::Win32::UI::Controls::INITCOMMONCONTROLSEX>() as u32,
                dwICC: windows::Win32::UI::Controls::ICC_LISTVIEW_CLASSES,
            });
            let gwc = WNDCLASSW {
                lpfnWndProc: Some(gallery_proc),
                hInstance: hinst.into(),
                lpszClassName: w!("jugadon_gallery"),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: CreateSolidBrush(COLORREF(BG_COLOR)),
                ..Default::default()
            };
            RegisterClassW(&gwc);
            TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
            let Ok(hwnd) = CreateWindowExW(WS_EX_TOOLWINDOW, class, w!("Jugadón"), WS_POPUP, 0, 0, 0, 0, None, None, Some(hinst.into()), None) else {
                return;
            };
            MAIN_HWND.store(hwnd.0 as isize, Ordering::Relaxed);

            let enc = logic::first_encoder(cfg.encoder, cfg.gpu_only);
            let app = App {
                hwnd,
                cfg,
                cfg_path,
                ffmpeg,
                buf_dir: temp.join("jugadon_buf"),
                log_path,
                icon_on: load_icon(hinst.into(), 1),
                icon_off: load_icon(hinst.into(), 2),
                icon_rec: make_icon(),
                manual: None,
                rec_dir: temp.join("jugadon_rec"),
                menu_windows: Vec::new(),
                gallery: None,
                share_token: None,
                tick_ms: 2000,
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
                last_game: None,
                last_trim: None,
                last_file: None,
                sound: logic::clip_sound_wav(),
            };
            APP.with(|a| *a.borrow_mut() = Some(app));
            with_app(|a| {
                a.add_icon();
                a.register_keys();
                a.recover_recordings();
                a.tick(); // arranca a grabar ya si no está en "solo juegos" o si ya hay un juego abierto
                // "jugadon.exe --clips" abre directo la ventana (sirve para un acceso directo)
                if std::env::args().any(|x| x == "--clips") {
                    a.open_gallery();
                }
            });
            SetTimer(Some(hwnd), TIMER_WATCHDOG, 2000, None);
            let _ = windows::Win32::UI::Accessibility::SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(foreground_changed),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );

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
    const BUFFER_HNS: i64 = 4_000_000; // 400 ms (el hilo se despierta cada 40 ms)
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
    /// mic = mezclar también el micrófono por defecto (si falla, sigue sin mic).
    pub fn start(mic: bool) -> Option<Audio> {
        let (ftx, frx) = mpsc::sync_channel(1);
        let (tx, rx) = mpsc::channel::<File>();
        thread::spawn(move || unsafe { run(ftx, rx, mic) });
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

    impl Drop for Cap {
        fn drop(&mut self) {
            unsafe {
                let _ = self.client.Stop();
            }
        }
    }

    unsafe fn default_id(en: &IMMDeviceEnumerator, flow: EDataFlow) -> Option<String> {
        let dev = en.GetDefaultAudioEndpoint(flow, eConsole).ok()?;
        let p: PWSTR = dev.GetId().ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s
    }

    /// flow = eRender (sonido del sistema, por loopback) o eCapture (micrófono).
    /// want = formato que ya le dijimos a ffmpeg (al reconectar, o para el mic).
    unsafe fn open(en: &IMMDeviceEnumerator, flow: EDataFlow, want: Option<(u32, u16)>) -> windows::core::Result<Cap> {
        let dev = en.GetDefaultAudioEndpoint(flow, eConsole)?;
        let p = dev.GetId()?;
        let id = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as _));
        let lb = if flow == eRender { LOOPBACK } else { 0 };

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
        if client.Initialize(AUDCLNT_SHAREMODE_SHARED, lb | AUTOCONVERT | SRC_QUALITY, BUFFER_HNS, 0, &wf, None).is_ok() {
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
        let r = if ok { client.Initialize(AUDCLNT_SHAREMODE_SHARED, lb, BUFFER_HNS, 0, mix, None) } else { Err(windows::core::Error::empty()) };
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

    /// Una fuente de audio (sistema o mic) con su cola y reconexión automática.
    struct Source {
        flow: EDataFlow,
        cap: Option<Cap>,
        q: Vec<f32>,
    }

    impl Source {
        unsafe fn check_device(&mut self, en: &IMMDeviceEnumerator, fmt: (u32, u16)) {
            let changed = match (&self.cap, default_id(en, self.flow)) {
                (Some(c), Some(id)) => c.id != id,
                (None, Some(_)) => true,
                _ => false,
            };
            if changed {
                self.cap = None;
                self.cap = open(en, self.flow, Some(fmt)).ok();
            }
        }

        unsafe fn read(&mut self) {
            if let Some(c) = &self.cap {
                if drain(c, &mut self.q).is_err() {
                    self.cap = None; // dispositivo desconectado: se reabre en el próximo chequeo
                }
            }
        }
    }

    unsafe fn run(ftx: mpsc::SyncSender<Option<(u32, u16)>>, rx: mpsc::Receiver<File>, mic: bool) {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
            Ok(e) => e,
            Err(_) => {
                let _ = ftx.send(None);
                return;
            }
        };
        let sys = match open(&en, eRender, None) {
            Ok(c) => c,
            Err(_) => {
                let _ = ftx.send(None);
                return;
            }
        };
        let fmt = (sys.rate, sys.ch);
        let _ = ftx.send(Some(fmt));
        let Ok(mut pipe) = rx.recv() else { return };
        let (rate, chs) = (fmt.0 as u64, fmt.1 as usize);
        let ms = |n: u64| rate * n / 1000;

        // Pre-roll: el pipe tiene ~4 KB; la 2da escritura de 10 ms bloquea hasta que ffmpeg empieza a leer.
        let silence10 = vec![0f32; ms(10) as usize * chs];
        for _ in 0..2 {
            if !write_f32(&mut pipe, &silence10) {
                return;
            }
        }
        let mut sources = vec![Source { flow: eRender, cap: Some(sys), q: Vec::new() }];
        if mic {
            sources.push(Source { flow: eCapture, cap: open(&en, eCapture, Some(fmt)).ok(), q: Vec::new() });
        }
        for s in sources.iter_mut() {
            s.read(); // lo capturado antes de arrancar se descarta
            s.q.clear();
        }
        // El audio se escribe atado al reloj de pared, con 60 ms de colchón para el jitter.
        // Si una fuente no manda nada (loopback en silencio), va silencio; si se atrasa, se recorta.
        let lag = ms(60);
        let max_q = ms(200) as usize * chs;
        let t0 = Instant::now();
        let mut written: u64 = 0;
        let mut last_check = Instant::now();
        let mut out: Vec<f32> = Vec::new();

        loop {
            // 25 veces por segundo alcanza (el audio se junta en bloques de 40 ms)
            thread::sleep(Duration::from_millis(40));
            if last_check.elapsed() >= Duration::from_secs(1) {
                last_check = Instant::now();
                for s in sources.iter_mut() {
                    s.check_device(&en, fmt);
                }
            }
            for s in sources.iter_mut() {
                s.read();
            }
            let target = (t0.elapsed().as_micros() as u64 * rate / 1_000_000).saturating_sub(lag);
            if target > written {
                let n = (target - written) as usize;
                out.clear();
                out.resize(n * chs, 0.0);
                for s in sources.iter_mut() {
                    crate::logic::mix_into(&mut out, &mut s.q, 1.0);
                }
                if sources.len() > 1 {
                    for v in out.iter_mut() {
                        *v = v.clamp(-1.0, 1.0);
                    }
                }
                if !write_f32(&mut pipe, &out) {
                    break; // ffmpeg terminó
                }
                written += n as u64;
                if out.capacity() > ms(500) as usize * chs {
                    out = Vec::new(); // después de un bloqueo largo no quedarse con un buffer grande
                }
            }
            for s in sources.iter_mut() {
                crate::logic::trim_queue(&mut s.q, max_q);
            }
        }
    }
}
