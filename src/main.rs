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
        /// versión en la que se probó cómo grabar (vacío = falta probar)
        pub probed: String,
        /// 0 tin-tin, 1 suave, 2 moneda
        pub sound_style: u32,
        /// borrar solos los clips de más de N días (0 = nunca)
        pub keep_days: u32,
        /// borrar los más viejos si la carpeta pasa de N GB (0 = sin límite)
        pub max_gb: u32,
        /// cartelito arriba del juego al guardar ("Clip guardado ✓")
        pub overlay: bool,
        /// avisar cuando sale una versión nueva
        pub updates: bool,
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
                probed: String::new(),
                sound_style: 0,
                keep_days: 0,
                max_gb: 0,
                overlay: true,
                updates: true,
            }
        }

        pub fn parse(text: &str, default_folder: String) -> Self {
            let mut c = Self::defaults(default_folder);
            // configs anteriores a la versión 2 tenían el micrófono apagado por defecto: se ignora ese valor
            let ver = text.lines().find_map(|l| l.trim().strip_prefix("version=")?.parse::<u32>().ok()).unwrap_or(0);
            let v2 = ver >= 2;
            // antes de la versión 3 "solo placa" venía prendido y la lista de encoders era otra
            let v3 = ver >= 3;
            // versión 4: "solo placa" vuelve a venir activado (el procesador solo si el usuario lo permite)
            let v4 = ver >= 4;
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
                    ("encoder", Some(n)) if v3 && (n as usize) < ENCODERS.len() => c.encoder = n as usize,
                    ("folder", _) if !v.is_empty() => c.folder = v.to_string(),
                    ("bg", Some(n)) if n <= BG_ALWAYS => c.bg = n,
                    ("only_games", Some(0)) => c.bg = BG_ALWAYS, // config de la versión 0.3
                    ("rec_key", Some(n)) if (1..=254).contains(&n) => c.rec.vk = n,
                    ("rec_mods", Some(n)) => c.rec.mods = n & 7,
                    ("mic", Some(n)) if n <= 1 && v2 => c.mic = n == 1,
                    ("sound", Some(n)) if n <= 1 => c.sound = n == 1,
                    ("gpu_only", Some(n)) if n <= 1 && v4 => c.gpu_only = n == 1,
                    ("probed", _) if v4 => c.probed = v.to_string(),
                    ("sound_style", Some(n)) if n <= 2 => c.sound_style = n,
                    ("keep_days", Some(n)) if n <= 3650 => c.keep_days = n,
                    ("max_gb", Some(n)) if n <= 10_000 => c.max_gb = n,
                    ("overlay", Some(n)) if n <= 1 => c.overlay = n == 1,
                    ("updates", Some(n)) if n <= 1 => c.updates = n == 1,
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
                "version=4\nseconds={}\nfps={}\nquality={}\nclip_key={}\nclip_mods={}\nshot_key={}\nshot_mods={}\nfolder={}\nencoder={}\nbg={}\nrec_key={}\nrec_mods={}\nmic={}\nsound={}\ngpu_only={}\nprobed={}\nsound_style={}\nkeep_days={}\nmax_gb={}\noverlay={}\nupdates={}\n",
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
                self.gpu_only as u8,
                self.probed,
                self.sound_style,
                self.keep_days,
                self.max_gb,
                self.overlay as u8,
                self.updates as u8
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
    // Media Foundation: el compresor de la placa a través de Windows. Lo trae el driver de
    // cualquier marca desde hace años, así que anda aunque NVENC/AMF/QSV de FFmpeg pidan un
    // driver más nuevo o falte su runtime.
    const MF: &[&str] = &["-hw_encoding", "1", "-rate_control", "cbr", "-scenario", "display_remoting"];
    // x264 (sin placa que codifique): el preset más liviano y solo 2 hilos, para dejarle CPU al juego
    // los hilos se agregan según la resolución (1 hilo a 480p, 2 a 720p)
    const X264: &[&str] = &["-preset", "ultrafast", "-tune", "zerolatency"];
    // a 720p como máximo (una ventana chica no se agranda)
    const GPU: &str = "scale_d3d11=width=-2:height=min(720\\,trunc(ih/2)*2):format=nv12";
    const CPU: &str = "hwdownload,format=bgra,scale=-2:min(720\\,trunc(ih/2)*2):flags=fast_bilinear,format=nv12";

    /// Orden de prueba: todo en GPU primero, después escalado en CPU, último x264.
    /// ("720" en vf se reemplaza por el alto elegido.)
    pub const ENCODERS: [Enc; 13] = [
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
        // placa vía Windows: sirve con drivers viejos o sin runtime de AMF/QSV
        Enc { gdi: false, label: "Placa vía Windows", vf: GPU, codec: "h264_mf", opts: MF },
        Enc {
            gdi: false,
            label: "Placa vía Windows (copia)",
            vf: "scale_d3d11=width=-2:height=min(720\\,trunc(ih/2)*2):format=nv12,hwdownload,format=nv12",
            codec: "h264_mf",
            opts: MF,
        },
        Enc { gdi: false, label: "Placa vía Windows (escala CPU)", vf: CPU, codec: "h264_mf", opts: MF },
        // captura por GDI (drivers sin Desktop Duplication) pero comprimiendo con la placa
        Enc { gdi: true, label: "Placa vía Windows (captura GDI)", vf: "scale=-2:min(720\\,trunc(ih/2)*2):flags=fast_bilinear,format=nv12", codec: "h264_mf", opts: MF },
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

    /// Quién comprime el video, en palabras: "la placa de video (NVENC)" o "el procesador".
    pub fn who(enc: usize) -> String {
        let e = &ENCODERS[enc];
        if e.codec == "libx264" {
            "el procesador".to_string()
        } else {
            let name = match e.codec {
                "h264_nvenc" => "NVENC",
                "h264_amf" => "AMF",
                "h264_qsv" => "Quick Sync",
                _ => "vía Windows",
            };
            format!("la placa de video ({name})")
        }
    }

    #[cfg(test)]
    #[test]
    fn who_names() {
        assert_eq!(who(0), "la placa de video (NVENC)");
        assert_eq!(who(6), "la placa de video (vía Windows)");
        assert_eq!(who(11), "el procesador");
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
        /// prueba de 3 s (para elegir la forma de grabar que anda en esta PC)
        Test(&'a str),
    }

    /// Número de compilación: la prueba de cómo grabar se repite con cada versión nueva.
    pub const BUILD: &str = match option_env!("GITHUB_RUN_NUMBER") {
        Some(v) => v,
        None => "dev",
    };

    /// Qué se graba: la pantalla principal o una sola ventana (por su HWND).
    /// La ventana se graba con Windows Graphics Capture: sale solo esa ventana,
    /// aunque esté tapada o en segundo plano.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Source {
        Screen,
        Window(u64),
    }

    /// Pistas de audio con el micrófono aparte. Por stdin llegan 2×ch canales: [juego…, micrófono…].
    /// Pista 0 = juego + voz (la que suena en cualquier reproductor, Discord o el navegador),
    /// pista 1 = solo juego, pista 2 = solo micrófono (mono): con eso se rearma la mezcla después
    /// sin volver a grabar. Cuesta casi nada: el AAC "fast" es liviano y el video no se toca.
    pub fn split_filter(ch: u16) -> String {
        if ch == 1 {
            "[1:a]asplit=3[x][y][z];[x]pan=mono|c0=c0+c1,asoftclip=type=hard[mix];[y]pan=mono|c0=c0[game];[z]pan=mono|c0=c1[mic]".into()
        } else {
            "[1:a]asplit=3[x][y][z];[x]pan=stereo|c0=c0+c2|c1=c1+c3,asoftclip=type=hard[mix];[y]pan=stereo|c0=c0|c1=c1[game];[z]pan=mono|c0=0.5*c2+0.5*c3[mic]".into()
        }
    }

    /// Argumentos de grabación. audio = (Hz, canales, ¿micrófono en pista aparte?) de PCM f32 por stdin.
    pub fn record_args(enc: usize, height: u32, fps: u32, src: Source, audio: Option<(u32, u16, bool)>, out: Out) -> Vec<String> {
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
            a.push("-i".into());
            // una ventana se pide por su handle: nunca se graba el escritorio entero en su lugar
            a.push(match src {
                Source::Screen => "desktop".into(),
                Source::Window(h) => format!("hwnd={h}"),
            });
        } else {
            a.extend(s(&["-f", "lavfi", "-i"]));
            a.push(match src {
                Source::Screen => format!("ddagrab=framerate={fps}:draw_mouse=1"),
                Source::Window(h) => format!("gfxcapture=hwnd={h}:max_framerate={fps}:capture_cursor=1"),
            });
        }
        let split = matches!(audio, Some((_, ch, true)) if ch <= 2);
        if let Some((rate, ch, _)) = audio {
            // sin probesize/analyzeduration ffmpeg lee ~5 s de audio antes de arrancar el video (desfase)
            a.extend(s(&["-thread_queue_size", "128", "-probesize", "32", "-analyzeduration", "0", "-f", "f32le", "-ar"]));
            a.push(rate.to_string());
            a.push("-ac".into());
            a.push((if split { ch * 2 } else { ch }).to_string());
            a.extend(s(&["-i", "pipe:0"]));
            if split {
                a.extend(s(&["-filter_complex_threads", "1", "-filter_complex"]));
                a.push(format!("[0:v]{vf}[v];{}", split_filter(ch)));
                a.extend(s(&["-map", "[v]", "-map", "[mix]", "-map", "[game]", "-map", "[mic]"]));
            } else {
                a.extend(s(&["-map", "0:v", "-map", "1:a", "-vf"]));
                a.push(vf);
            }
        } else {
            a.extend(s(&["-map", "0:v", "-vf"]));
            a.push(vf);
        }
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
            if split {
                a.extend(s(&["-b:a:2", "64k"]));
            }
        }
        match out {
            Out::Segments { secs, dir } => {
                a.extend(s(&["-flush_packets", "1", "-f", "segment", "-segment_time", "2", "-segment_wrap"]));
                a.push(segment_wrap(secs).to_string());
                a.extend(s(&["-segment_format", "mpegts"]));
                a.push(format!("{dir}\\seg%03d.ts"));
            }
            // .ts: si algo se corta, lo grabado hasta ahí sigue sirviendo
            // y cada cuadro se escribe en el momento: al frenar no se pierde lo que quedaba en memoria
            // (con imagen quieta un video ocupa tan poco que antes no llegaba a escribirse nada)
            Out::File(path) => a.extend(s(&["-flush_packets", "1", "-y", "-f", "mpegts", path])),
            Out::Test(path) => a.extend(s(&["-t", "3", "-y", "-f", "mpegts", path])),
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

    /// Acortar un video a sus últimos `secs` segundos sin recomprimir (instantáneo, casi sin CPU).
    pub fn trim_args(input: &str, secs: u32, out: &str) -> Vec<String> {
        let from = format!("-{secs}");
        s(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-sseof", &from, "-i", input, "-map", "0", "-c", "copy", "-avoid_negative_ts", "make_zero", "-movflags", "+faststart", out])
    }

    /// Sacar los primeros `secs` segundos sin recomprimir.
    pub fn trim_start_args(input: &str, secs: u32, out: &str) -> Vec<String> {
        let from = secs.to_string();
        s(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-ss", &from, "-i", input, "-map", "0", "-c", "copy", "-avoid_negative_ts", "make_zero", "-movflags", "+faststart", out])
    }

    /// Recorte exacto elegido en el visor: desde `from` hasta `to` segundos, sin recomprimir.
    pub fn cut_args(input: &str, from: f64, to: f64, out: &str) -> Vec<String> {
        let (a, d) = (format!("{:.2}", from.max(0.0)), format!("{:.2}", (to - from).max(0.1)));
        s(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-ss", &a, "-i", input, "-t", &d, "-map", "0", "-c", "copy", "-avoid_negative_ts", "make_zero", "-movflags", "+faststart", out])
    }

    /// Servicios para compartir por link, en orden: si uno falla (caído, bloqueado, archivo muy grande),
    /// se prueba el siguiente. Devuelve (nombre, argumentos de curl.exe, horas que dura o 0 = no vence).
    /// curl.exe viene con Windows 10 (1803+) y 11.
    pub fn upload_plan(path: &str, permanent: bool, size: u64) -> Vec<(&'static str, Vec<String>, u32)> {
        // curl interpreta "\" adentro de las comillas: la ruta va con barras normales (Windows las acepta)
        let p = path.replace('\\', "/");
        let base = || s(&["-sS", "-f", "--retry", "1", "--connect-timeout", "15", "--max-time", "1800", "-A", "Jugadon/1.0"]);
        let with = |extra: &[String], url: &str| {
            let mut a = base();
            for e in extra {
                a.push("-F".into());
                a.push(e.clone());
            }
            a.push(url.into());
            a
        };
        let mb = size >> 20;
        let mut plan = Vec::new();
        if permanent {
            if mb < 200 {
                plan.push(("catbox", with(&["reqtype=fileupload".into(), format!("fileToUpload=@\"{p}\"")], "https://catbox.moe/user/api.php"), 0));
            }
        } else {
            if mb < 1000 {
                plan.push(("litterbox", with(&["reqtype=fileupload".into(), "time=72h".into(), format!("fileToUpload=@\"{p}\"")], "https://litterbox.catbox.moe/resources/internals/api.php"), 72));
            }
            if mb < 128 {
                plan.push(("uguu", with(&[format!("files[]=@\"{p}\"")], "https://uguu.se/upload"), 48));
            }
        }
        // gofile: sin límite práctico; el link dura mientras lo sigan mirando (unos 10 días sin visitas)
        plan.push(("gofile", with(&[format!("file=@\"{p}\"")], "https://upload.gofile.io/uploadfile"), 240));
        plan
    }

    /// Saca el link de la respuesta del servicio (texto plano o JSON).
    pub fn parse_upload(out: &str) -> Option<String> {
        let t = out.trim();
        if t.starts_with("https://") && !t.contains(char::is_whitespace) {
            return Some(t.to_string());
        }
        for key in ["\"downloadPage\":\"", "\"url\":\""] {
            if let Some(i) = t.find(key) {
                let rest = &t[i + key.len()..];
                let url = rest[..rest.find('"')?].replace("\\/", "/");
                if url.starts_with("https://") {
                    return Some(url);
                }
            }
        }
        None
    }

    /// Nombre de una copia: "clip_X.mp4" + "sin10s" -> "clip_X_sin10s.mp4" (el título personalizado se mantiene al final).
    pub fn derived_name(name: &str, tag: &str) -> String {
        let stem = name.rsplit_once('.').map_or(name, |x| x.0);
        match stem.split_once(" - ") {
            Some((base, custom)) => format!("{base}_{tag} - {custom}.mp4"),
            None => format!("{stem}_{tag}.mp4"),
        }
    }

    /// Para Discord (máx. 10 MB): (alto, kbit/s de video) según la duración.
    pub fn discord_plan(dur_secs: u32) -> (u32, u32) {
        // 9,5 MB de margen, menos 96 kbit/s de audio y ~4 % del contenedor
        let total = 9.5 * 1024.0 * 1024.0 * 8.0 / 1000.0;
        let v = (total * 0.96 / dur_secs.max(1) as f64 - 96.0).clamp(150.0, 6000.0) as u32;
        let h = if v >= 2000 {
            720
        } else if v >= 900 {
            480
        } else {
            360
        };
        (h, v)
    }

    /// Recomprimir para Discord con la misma forma de comprimir que anda en esta PC (la placa, si se puede).
    pub fn discord_args(enc: usize, input: &str, dur_secs: u32, out: &str) -> Vec<String> {
        let e = &ENCODERS[enc];
        let (h, v) = discord_plan(dur_secs);
        let mut a = s(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-hwaccel", "auto", "-i", input, "-map", "0:v:0", "-map", "0:a:0?", "-vf"]);
        a.push(format!("scale=-2:min({h}\\,trunc(ih/2)*2):flags=bicubic,format=nv12"));
        a.extend(s(&["-r", "30", "-c:v", e.codec]));
        a.extend(s(e.opts));
        a.extend(["-b:v".into(), format!("{v}k"), "-maxrate".into(), format!("{v}k"), "-bufsize".into(), format!("{}k", v * 2)]);
        a.extend(s(&["-g", "60", "-bf", "0", "-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart", out]));
        a
    }

    #[cfg(test)]
    pub fn s_test(v: &[&str]) -> Vec<String> {
        s(v)
    }

    /// Lo que ffmpeg cuenta de un video (de `ffmpeg -i archivo`, sin ffprobe).
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Info {
        pub audio: usize,
        pub dur: f64,
        pub w: u32,
        pub h: u32,
        pub fps: f64,
    }

    pub fn parse_info(text: &str) -> Info {
        let mut i = Info::default();
        if let Some(p) = text.find("Duration: ") {
            let t = text[p + 10..].split(',').next().unwrap_or("");
            let parts: Vec<f64> = t.trim().split(':').filter_map(|x| x.parse().ok()).collect();
            if parts.len() == 3 {
                i.dur = parts[0] * 3600.0 + parts[1] * 60.0 + parts[2];
            }
        }
        for line in text.lines().filter(|l| l.contains("Stream #")) {
            if line.contains("Audio:") {
                i.audio += 1;
            } else if line.contains("Video:") && i.w == 0 {
                for tok in line.split([' ', ',']) {
                    if let Some((a, b)) = tok.split_once('x') {
                        if let (Ok(w), Ok(h)) = (a.parse::<u32>(), b.parse::<u32>()) {
                            if w >= 16 && h >= 16 {
                                (i.w, i.h) = (w, h);
                                break;
                            }
                        }
                    }
                }
                let toks: Vec<&str> = line.split([' ', ',']).filter(|t| !t.is_empty()).collect();
                if let Some(k) = toks.iter().position(|t| *t == "fps") {
                    i.fps = k.checked_sub(1).and_then(|k| toks[k].parse().ok()).unwrap_or(0.0);
                }
            }
        }
        i
    }

    /// Para leer la info de un video: ffmpeg sin salida termina con error, pero escribe la info.
    pub fn info_args(input: &str) -> Vec<String> {
        s(&["-hide_banner", "-nostdin", "-i", input])
    }

    /// Video vertical (1080×1920) para TikTok / Reels / Shorts, con la placa si se puede.
    /// blur = false: se recorta el centro. blur = true: entra todo el video y arriba y abajo
    /// va el mismo video desenfocado (el desenfoque se hace chiquito, 270×480: casi no gasta).
    pub fn vertical_args(enc: usize, input: &str, blur: bool, out: &str) -> Vec<String> {
        let e = &ENCODERS[enc];
        let mut a = s(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-hwaccel", "auto", "-i", input]);
        if blur {
            a.push("-filter_complex".into());
            a.push(
                "[0:v:0]split=2[a][b];[a]scale=270:480:force_original_aspect_ratio=increase:flags=fast_bilinear,crop=270:480,boxblur=8:2,scale=1080:1920:flags=fast_bilinear[bg];\
                 [b]scale=1080:1920:force_original_aspect_ratio=decrease:flags=bicubic,setsar=1[fg];[bg][fg]overlay=(W-w)/2:(H-h)/2,format=nv12[v]"
                    .into(),
            );
            a.extend(s(&["-map", "[v]"]));
        } else {
            a.extend(s(&["-map", "0:v:0", "-vf"]));
            a.push(
                "crop=min(iw\\,trunc(ih*9/16/2)*2):ih,scale=1080:1920:force_original_aspect_ratio=decrease:flags=bicubic,pad=1080:1920:(ow-iw)/2:(oh-ih)/2,setsar=1,format=nv12".into(),
            );
        }
        a.extend(s(&["-map", "0:a:0?", "-c:v", e.codec]));
        a.extend(s(e.opts));
        a.extend(s(&["-b:v", "8000k", "-maxrate", "8000k", "-bufsize", "16000k", "-g", "60", "-bf", "0", "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart", out]));
        a
    }

    /// Juntar varios clips en uno (en el orden dado). Todo queda del tamaño y los fps del primero;
    /// si a alguno le falta el audio, se pone silencio en su lugar.
    pub fn join_args(enc: usize, inputs: &[(String, Info)], out: &str) -> Vec<String> {
        let e = &ENCODERS[enc];
        let first = inputs.first().map(|x| x.1.clone()).unwrap_or_default();
        let even = |v: u32, d: u32| if v >= 16 { v & !1 } else { d };
        let (w, h) = (even(first.w, 1280), even(first.h, 720));
        let fps = if first.fps >= 20.0 { (first.fps.round() as u32).min(60) } else { 30 };
        let mut a = s(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
        for (p, _) in inputs {
            a.extend(s(&["-hwaccel", "auto", "-i", p]));
        }
        let mut extra = inputs.len();
        let mut f = String::new();
        let mut silent = Vec::new();
        for (k, (_, inf)) in inputs.iter().enumerate() {
            f += &format!("[{k}:v:0]scale={w}:{h}:force_original_aspect_ratio=decrease:flags=bicubic,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={fps},format=yuv420p[v{k}];");
            let src = if inf.audio > 0 {
                format!("[{k}:a:0]")
            } else {
                silent.push(inf.dur.max(0.5));
                extra += 1;
                format!("[{}:a]", extra - 1)
            };
            f += &format!("{src}aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo[a{k}];");
        }
        for d in silent {
            a.extend(s(&["-f", "lavfi", "-t"]));
            a.push(format!("{d:.2}"));
            a.extend(s(&["-i", "anullsrc=r=48000:cl=stereo"]));
        }
        for k in 0..inputs.len() {
            f += &format!("[v{k}][a{k}]");
        }
        f += &format!("concat=n={}:v=1:a=1[cv][ca];[cv]format=nv12[v]", inputs.len());
        a.push("-filter_complex".into());
        a.push(f);
        a.extend(s(&["-map", "[v]", "-map", "[ca]", "-c:v", e.codec]));
        a.extend(s(e.opts));
        let br = bitrate_kbit(h.min(720), fps) * if h > 720 { 2 } else { 1 };
        a.extend(["-b:v".into(), format!("{br}k"), "-maxrate".into(), format!("{br}k"), "-bufsize".into(), format!("{}k", br * 2), "-g".into(), (fps * 2).to_string()]);
        a.extend(s(&["-bf", "0", "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart", out]));
        a
    }

    /// Opciones de sonido para clips grabados con el micrófono aparte (pista 1 = juego, 2 = micrófono).
    pub const SOUND_MODES: [(&str, &str); 5] = [
        ("Sacar mi voz (solo el juego)", "sinmic"),
        ("Solo mi voz (sin el juego)", "solomic"),
        ("Mi voz más fuerte", "vozfuerte"),
        ("Mi voz más baja", "vozbaja"),
        ("El juego más bajo", "juegobajo"),
    ];

    /// Rearma el sonido sin tocar el video (copia: instantáneo).
    pub fn sound_args(input: &str, mode: usize, out: &str) -> Vec<String> {
        let mut a = s(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i", input, "-map", "0:v:0"]);
        let mix = |g: f32, m: f32| format!("[0:a:1]volume={g}[g];[0:a:2]aformat=channel_layouts=stereo,volume={m}[m];[g][m]amix=inputs=2:normalize=0:duration=first,asoftclip=type=hard[a]");
        match mode {
            0 | 1 => {
                a.extend(s(&["-map", if mode == 0 { "0:a:1" } else { "0:a:2" }, "-c", "copy"]));
            }
            _ => {
                let (g, m) = match mode {
                    2 => (0.55, 1.7),
                    3 => (1.0, 0.4),
                    _ => (0.3, 1.0),
                };
                a.push("-filter_complex".into());
                a.push(mix(g, m));
                a.extend(s(&["-map", "[a]", "-c:v", "copy", "-c:a", "aac", "-b:a", "160k"]));
            }
        }
        a.extend(s(&["-movflags", "+faststart", out]));
        a
    }

    pub const REPO_URL: &str = "https://github.com/bautistacasero265-maker/jugadon";

    /// curl.exe a la API de GitHub: la última versión publicada (un pedido chico, una vez al abrir y cada 12 h).
    pub fn update_args() -> Vec<String> {
        s(&[
            "-sS",
            "-f",
            "-L",
            "--connect-timeout",
            "10",
            "--max-time",
            "20",
            "-A",
            "Jugadon/1.0",
            "-H",
            "Accept: application/vnd.github+json",
            "https://api.github.com/repos/bautistacasero265-maker/jugadon/releases/latest",
        ])
    }

    /// Número de versión de la respuesta ("tag_name": "v1.0.45" -> 45).
    pub fn latest_version(json: &str) -> Option<u32> {
        let i = json.find("\"tag_name\"")?;
        let rest = &json[i + 10..];
        let rest = &rest[rest.find('"')? + 1..];
        let tag = &rest[..rest.find('"')?];
        tag.strip_prefix("v1.0.")?.parse().ok()
    }

    /// ¿Hay una versión más nueva que esta? (las compilaciones locales "dev" no avisan)
    pub fn is_newer(build: &str, latest: u32) -> bool {
        build.parse::<u32>().is_ok_and(|b| latest > b)
    }

    /// Mezcla dos fuentes en canales separados: cada cuadro queda [a (ch canales), b (ch canales)].
    /// Lo que falta (fuente callada o atrasada) va en silencio; lo usado sale de las colas.
    pub fn interleave2(out: &mut [f32], a: &mut Vec<f32>, b: &mut Vec<f32>, ch: usize) {
        let frames = out.len() / (2 * ch);
        for (i, f) in out.chunks_exact_mut(2 * ch).enumerate() {
            for c in 0..ch {
                f[c] = a.get(i * ch + c).copied().unwrap_or(0.0);
                f[ch + c] = b.get(i * ch + c).copied().unwrap_or(0.0);
            }
        }
        let (na, nb) = (a.len().min(frames * ch), b.len().min(frames * ch));
        a.drain(..na);
        b.drain(..nb);
    }

    /// Nombre nuevo al renombrar: se conserva el tipo y la fecha ("clip_AAAA-…") y se agrega el título.
    /// Devuelve None si el texto queda vacío.
    pub fn renamed(name: &str, text: &str) -> Option<String> {
        let (stem, ext) = name.rsplit_once('.').unwrap_or((name, "mp4"));
        let base = stem.split_once(" - ").map_or(stem, |x| x.0);
        let clean: String = text.chars().filter(|c| !"\\/:*?\"<>|".contains(*c) && !c.is_control()).collect();
        let clean: String = clean.trim().trim_end_matches('.').chars().take(60).collect();
        (!clean.is_empty()).then(|| format!("{base} - {clean}.{ext}"))
    }

    /// Título puesto por el usuario ("clip_… - ace en Valorant.mp4" -> "ace en Valorant").
    pub fn custom_title(name: &str) -> Option<&str> {
        let stem = name.rsplit_once('.').map_or(name, |x| x.0);
        stem.split_once(" - ").map(|x| x.1).filter(|t| !t.is_empty())
    }

    /// Limpieza automática. files: (nombre, días de antigüedad, bytes, favorito), en cualquier orden.
    /// Devuelve qué borrar: lo de más de `keep_days` días y, si la carpeta pasa de `max_gb`, los más viejos.
    /// Los favoritos y lo de hoy nunca se borran.
    pub fn cleanup_plan(mut files: Vec<(String, u32, u64, bool)>, keep_days: u32, max_gb: u32) -> Vec<String> {
        files.sort_by_key(|f| std::cmp::Reverse(f.1)); // más viejos primero
        let mut del = Vec::new();
        let mut total: u64 = files.iter().map(|f| f.2).sum();
        let limit = max_gb as u64 * (1 << 30);
        for (name, age, size, fav) in files {
            if fav || age == 0 {
                continue;
            }
            if (keep_days > 0 && age > keep_days) || (max_gb > 0 && total > limit) {
                total -= size;
                del.push(name);
            }
        }
        del
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
        } else if bytes >= 1 << 20 {
            format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
        } else {
            format!("{} KB", bytes.div_ceil(1024))
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
    /// Sonido al guardar (WAV en memoria): 0 tin-tin, 1 suave, 2 moneda.
    pub fn clip_sound_wav(style: u32) -> Vec<u8> {
        let rate = 22050u32;
        let mut pcm: Vec<i16> = Vec::new();
        let (notes, vol): (&[(f32, u32)], f32) = match style {
            1 => (&[(659.3, 160)], 0.22),
            2 => (&[(987.8, 60), (1318.5, 190)], 0.3),
            _ => (&[(880.0, 70), (1318.5, 110)], 0.35),
        };
        for &(freq, ms) in notes {
            let n = rate * ms / 1000;
            for i in 0..n {
                let t = i as f32 / rate as f32;
                let env = (i as f32 / 200.0).min(1.0) * (1.0 - i as f32 / n as f32).powf(1.5);
                pcm.push(((t * freq * std::f32::consts::TAU).sin() * env * vol * 32767.0) as i16);
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
            assert!(!Config::parse("version=4\ngpu_only=0\n", "F".into()).gpu_only);
            // config de la versión 3 (procesador permitido por defecto): vuelve a solo placa
            assert!(Config::parse("version=3\ngpu_only=0\n", "F".into()).gpu_only);
            // config vieja con solo placa y encoder guardado: vuelve a los valores nuevos
            let c = Config::parse("version=2\ngpu_only=0\nencoder=7\n", "F".into());
            assert!(c.gpu_only && c.encoder == 0);
            let c = Config::parse(&Config { encoder: 9, gpu_only: false, ..Config::defaults("F".into()) }.serialize(), "F".into());
            assert!(!c.gpu_only && c.encoder == 9);
            let c = Config::parse(&Config { probed: "41".into(), ..Config::defaults("F".into()) }.serialize(), "F".into());
            assert_eq!(c.probed, "41");
            assert!(Config::parse("version=2\nprobed=41\n", "F".into()).probed.is_empty());
            let t = record_args(12, 0, 30, Source::Screen, None, Out::Test("p.ts")).join(" ");
            assert!(t.ends_with("-t 3 -y -f mpegts p.ts") && t.contains("gdigrab"));
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
            assert!(d.overlay && d.updates);
            let c = Config::parse(&Config { overlay: false, updates: false, ..Config::defaults("F".into()) }.serialize(), "F".into());
            assert!(!c.overlay && !c.updates);
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
            assert_eq!(seen.len(), 10);
            assert!(seen.iter().any(|&i| ENCODERS[i].codec == "h264_mf" && ENCODERS[i].gdi));
            // sin restricción: prueba todos
            let mut n = 1;
            let (mut e, mut t) = (0, 0);
            while let Some((ne, nt)) = next_encoder(e, t, false) {
                n += 1;
                (e, t) = (ne, nt);
            }
            assert_eq!(n, ENCODERS.len());
            // arrancando de uno guardado en el medio, da la vuelta
            assert_eq!(next_encoder(9, 0, true), Some((0, 4)));
            assert_eq!(first_encoder(10, true), 0);
            assert_eq!(first_encoder(10, false), 10);
            // los de procesador van al final: primero se prueba toda forma de usar la placa
            let first_cpu = ENCODERS.iter().position(|e| e.codec == "libx264").unwrap();
            assert!(ENCODERS[first_cpu..].iter().all(|e| e.codec == "libx264"));
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
            assert_eq!(fmt_size(60_000), "59 KB");
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
            let w = clip_sound_wav(0);
            assert!(clip_sound_wav(1).len() != w.len() && clip_sound_wav(2).len() > 44);
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
        fn mic_on_its_own_track() {
            let a = record_args(0, 720, 30, Source::Screen, Some((48000, 2, true)), Out::Segments { secs: 30, dir: "D" }).join(" ");
            assert!(a.contains("-ac 4 -i pipe:0 -filter_complex_threads 1 -filter_complex [0:v]scale_d3d11="), "{a}");
            assert!(a.contains("[v];[1:a]asplit=3"), "{a}");
            assert!(a.contains("-map [v] -map [mix] -map [game] -map [mic]"), "{a}");
            assert!(a.contains("-b:a 128k -b:a:2 64k"), "{a}");
            assert!(!a.contains(" -vf "), "{a}");
            // sin micrófono (o con 5.1) queda como siempre
            let b = record_args(0, 720, 30, Source::Screen, Some((48000, 6, true)), Out::Segments { secs: 30, dir: "D" }).join(" ");
            assert!(b.contains("-ac 6 -i pipe:0 -map 0:v -map 1:a -vf"), "{b}");
            let mono = split_filter(1);
            assert!(mono.contains("pan=mono|c0=c0+c1") && mono.contains("pan=mono|c0=c1[mic]"));
        }

        #[test]
        fn interleave_two_sources() {
            let mut out = vec![9.0; 8];
            let mut a = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
            let mut b = vec![-1.0, -2.0];
            interleave2(&mut out, &mut a, &mut b, 2);
            assert_eq!(out, vec![1.0, 2.0, -1.0, -2.0, 3.0, 4.0, 0.0, 0.0]);
            assert_eq!(a, vec![5.0, 6.0]);
            assert!(b.is_empty());
        }

        #[test]
        fn video_info() {
            let t = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'clip.mp4':\n  Duration: 00:01:02.50, start: 0.000000, bitrate: 6000 kb/s\n  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, progressive), 1280x720 [SAR 1:1 DAR 16:9], 5800 kb/s, 60 fps, 60 tbr, 15360 tbn (default)\n  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 128 kb/s (default)\n  Stream #0:2[0x3](und): Audio: aac (LC), 48000 Hz, stereo, fltp, 128 kb/s\n  Stream #0:3[0x4](und): Audio: aac (LC), 48000 Hz, mono, fltp, 64 kb/s\nAt least one output file must be specified\n";
            let i = parse_info(t);
            assert_eq!(i, Info { audio: 3, dur: 62.5, w: 1280, h: 720, fps: 60.0 });
            let j = parse_info("Duration: 00:00:09.97, start\n  Stream #0:0: Video: h264, nv12, 404x720, 29.97 fps, 30 tbr\n");
            assert_eq!((j.audio, j.w, j.h, j.fps), (0, 404, 720, 29.97));
            assert_eq!(parse_info("basura"), Info::default());
        }

        #[test]
        fn vertical_and_join() {
            let v = vertical_args(0, "in.mp4", false, "out.mp4").join(" ");
            assert!(v.contains("-vf crop=min(iw\\,trunc(ih*9/16/2)*2):ih,scale=1080:1920"), "{v}");
            assert!(v.contains("-map 0:a:0? -c:v h264_nvenc"), "{v}");
            let b = vertical_args(6, "in.mp4", true, "out.mp4").join(" ");
            assert!(b.contains("boxblur") && b.contains("overlay=(W-w)/2:(H-h)/2") && b.contains("-map [v]") && b.contains("-c:v h264_mf"), "{b}");
            let a = Info { audio: 3, dur: 10.0, w: 1280, h: 720, fps: 30.0 };
            let n = Info { audio: 0, dur: 4.0, w: 854, h: 480, fps: 60.0 };
            let j = join_args(0, &[("a.mp4".into(), a.clone()), ("b.mp4".into(), n), ("c.mp4".into(), a)], "o.mp4").join(" ");
            assert!(j.contains("-hwaccel auto -i a.mp4 -hwaccel auto -i b.mp4 -hwaccel auto -i c.mp4 -f lavfi -t 4.00 -i anullsrc"), "{j}");
            assert!(j.contains("[1:v:0]scale=1280:720") && j.contains("[3:a]aresample") && j.contains("[2:a:0]aresample"), "{j}");
            assert!(j.contains("[v0][a0][v1][a1][v2][a2]concat=n=3:v=1:a=1"), "{j}");
            assert!(j.contains("-b:v 6000k") && j.ends_with("o.mp4"), "{j}");
        }

        #[test]
        fn sound_modes() {
            let a = sound_args("i.mp4", 0, "o.mp4").join(" ");
            assert!(a.contains("-map 0:v:0 -map 0:a:1 -c copy"), "{a}");
            assert!(sound_args("i.mp4", 1, "o.mp4").join(" ").contains("-map 0:a:2 -c copy"));
            let m = sound_args("i.mp4", 2, "o.mp4").join(" ");
            assert!(m.contains("[0:a:1]volume=0.55[g];[0:a:2]aformat=channel_layouts=stereo,volume=1.7[m]") && m.contains("-c:v copy -c:a aac"), "{m}");
            for k in 0..SOUND_MODES.len() {
                assert!(sound_args("i", k, "o").last().unwrap() == "o");
            }
        }

        #[test]
        fn update_check() {
            let j = r#"{"url":"x","tag_name": "v1.0.52","name":"Jugadón 1.0.52"}"#;
            assert_eq!(latest_version(j), Some(52));
            assert_eq!(latest_version(r#"{"tag_name":"promo-v7"}"#), None);
            assert_eq!(latest_version("{}"), None);
            assert!(is_newer("45", 52));
            assert!(!is_newer("52", 52));
            assert!(!is_newer("dev", 99));
            assert!(update_args().last().unwrap().ends_with("/releases/latest"));
        }

        #[test]
        fn record_args_gpu_with_audio() {
            let a = record_args(0, 720, 60, Source::Screen, Some((48000, 2, false)), Out::Segments { secs: 30, dir: "C:\\T\\jugadon_buf" }).join(" ");
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
            let a = record_args(11, 720, 30, Source::Screen, None, Out::Segments { secs: 60, dir: "D" }).join(" ");
            assert!(a.contains("-nostdin"));
            // x264 en CPU: 720p si se pide, máximo 30 fps, 2 hilos
            assert!(a.contains("-map 0:v -vf hwdownload,format=bgra,scale=-2:min(720\\,trunc(ih/2)*2)"));
            assert!(a.contains("-c:v libx264 -preset ultrafast -tune zerolatency -threads 2"));
            // automático con procesador: 480p, 1 hilo
            let a = record_args(11, 0, 30, Source::Screen, None, Out::File("x")).join(" ");
            assert!(a.contains("scale=-2:min(480") && a.contains("-threads 1") && a.contains("-b:v 2500k"));
            // automático con placa: 720p
            let a = record_args(0, 0, 30, Source::Screen, None, Out::File("x")).join(" ");
            assert!(a.contains("height=min(720") && a.contains("-b:v 6000k") && !a.contains("-threads"));
            assert_eq!(Config::defaults("F".into()).height, 0);
            assert_eq!(Config::parse("height=720\n", "F".into()).height, 0);
            assert_eq!(Config::parse("quality=720\n", "F".into()).height, 720);
            assert!(a.contains("-b:v 6000k"));
            assert!(a.contains("-g 60"));
            let a = record_args(10, 720, 60, Source::Screen, None, Out::File("x")).join(" ");
            assert!(a.contains("ddagrab=framerate=30:"));
            assert!(a.contains("scale_d3d11=width=-2:height=min(720\\,trunc(ih/2)*2):format=nv12,hwdownload,format=nv12"));
            // GDI: otra entrada, sin lavfi
            let a = record_args(12, 720, 30, Source::Screen, None, Out::File("x")).join(" ");
            assert!(a.contains("-f gdigrab -framerate 30 -draw_mouse 1 -i desktop -map 0:v -vf scale=-2:min(720"));
            assert!(!a.contains("lavfi"));
            // GDI con una ventana: se pide esa ventana por su handle, nunca el escritorio
            let a = record_args(12, 720, 30, Source::Window(1234), None, Out::File("x")).join(" ");
            assert!(a.contains("-i hwnd=1234 ") && !a.contains("desktop"));
            // placa vía Windows (Media Foundation): sin copia, por hardware, CBR
            let a = record_args(6, 0, 30, Source::Screen, None, Out::File("x")).join(" ");
            assert!(a.contains("format=nv12 -c:v h264_mf -hw_encoding 1 -rate_control cbr") && a.contains("-b:v 6000k"));
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
            assert!(a.ends_with("-flush_packets 1 -y -f mpegts C:\\T\\rec.ts"));
            assert!(!a.contains("segment"));
            let r = remux_args("a.ts", "b.mp4").join(" ");
            assert!(r.contains("-i a.ts -map 0 -c copy -movflags +faststart b.mp4"));
            assert_eq!(fmt_elapsed(187), "3:07");
            assert!(cut_args("a.mp4", 1.5, 4.0, "b.mp4").join(" ").contains("-ss 1.50 -i a.mp4 -t 2.50 -map 0 -c copy"));
            let plan = upload_plan("C:\\V\\clip a.mp4", false, 10 << 20);
            assert_eq!(plan.iter().map(|x| x.0).collect::<Vec<_>>(), ["litterbox", "uguu", "gofile"]);
            assert!(plan[0].1.join(" ").contains("-F time=72h -F fileToUpload=@\"C:/V/clip a.mp4\" https://litterbox.catbox.moe/"));
            assert_eq!(upload_plan("x.mp4", false, 500 << 20).iter().map(|x| x.0).collect::<Vec<_>>(), ["litterbox", "gofile"]);
            assert_eq!(upload_plan("x.mp4", true, 300 << 20).iter().map(|x| x.0).collect::<Vec<_>>(), ["gofile"]);
            assert_eq!(upload_plan("x.mp4", true, 10).last().unwrap().2, 240);
            assert_eq!(parse_upload("https://litter.catbox.moe/abc.mp4\n"), Some("https://litter.catbox.moe/abc.mp4".into()));
            assert_eq!(parse_upload(r#"{"success":true,"files":[{"url":"https:\/\/h.uguu.se\/x.mp4"}]}"#), Some("https://h.uguu.se/x.mp4".into()));
            assert_eq!(parse_upload(r#"{"status":"ok","data":{"downloadPage":"https://gofile.io/d/AbC","id":"1"}}"#), Some("https://gofile.io/d/AbC".into()));
            assert_eq!(parse_upload("<html>error</html>"), None);
            assert!(trim_start_args("a.mp4", 10, "b.mp4").join(" ").contains("-ss 10 -i a.mp4 -map 0 -c copy"));
            assert_eq!(derived_name("clip_X.mp4", "sin10s"), "clip_X_sin10s.mp4");
            assert_eq!(derived_name("clip_X - ace.mp4", "discord"), "clip_X_discord - ace.mp4");
            assert_eq!(discord_plan(30).0, 720);
            assert!(discord_plan(60).1 > 1000 && discord_plan(60).1 < 1300);
            assert_eq!(discord_plan(600), (360, 150));
            let d = discord_args(0, "a.mp4", 30, "b.mp4").join(" ");
            assert!(d.contains("-c:v h264_nvenc") && d.contains("-b:a 96k") && d.ends_with("b.mp4"));
            assert_eq!(renamed("clip_2026-11-18_22-21-15.mp4", " ace: en *Valorant*  "), Some("clip_2026-11-18_22-21-15 - ace en Valorant.mp4".into()));
            assert_eq!(renamed("clip_A - viejo.mp4", "nuevo"), Some("clip_A - nuevo.mp4".into()));
            assert_eq!(renamed("clip_A.mp4", " ?? "), None);
            assert_eq!(custom_title("clip_A - ace en Valorant.mp4"), Some("ace en Valorant"));
            assert_eq!(custom_title("clip_A.mp4"), None);
            assert_eq!(describe_file("clip_2026-11-18_22-21-15 - ace.mp4"), ("Clip", "18/11/2026 22:21".to_string()));
            let files = vec![("a".into(), 40, 5 << 30, false), ("b".into(), 20, 5 << 30, true), ("c".into(), 10, 5 << 30, false), ("d".into(), 0, 5 << 30, false)];
            assert_eq!(cleanup_plan(files.clone(), 30, 0), vec!["a".to_string()]);
            assert_eq!(cleanup_plan(files.clone(), 0, 10), vec!["a".to_string(), "c".to_string()]);
            assert!(cleanup_plan(files, 0, 0).is_empty());
            let t = trim_args("a.mp4", 15, "b.mp4").join(" ");
            assert!(t.contains("-sseof -15 -i a.mp4 -map 0 -c copy") && t.ends_with("+faststart b.mp4"));
            assert_eq!(derived_name("clip_2026-11-18_22-21-15.mp4", "ultimos15s"), "clip_2026-11-18_22-21-15_ultimos15s.mp4");
            assert_eq!(describe_file(&derived_name("clip_2026-11-18_22-21-15.mp4", "ultimos30s")), ("Clip", "18/11/2026 22:21".to_string()));
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
        let mut args = logic::record_args(11, 720, 30, logic::Source::Screen, Some((48000, 2, false)), logic::Out::Segments { secs: 6, dir: &dirs });
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

/// Corre también en Linux (con RCLIP_FFMPEG=/usr/bin/ffmpeg) para probar los filtros antes de subir.
#[cfg(test)]
mod win_tests_tools {
    use crate::logic;
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn probe(ff: &str, f: &str) -> String {
        let probe = std::path::Path::new(ff).with_file_name(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" });
        let o = Command::new(probe)
            .args(["-v", "error", "-show_entries", "stream=codec_type,width,height,channels:format=duration", "-of", "default=nw=1", f])
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).to_string()
    }

    fn info(ff: &str, f: &str) -> logic::Info {
        let o = Command::new(ff).args(logic::info_args(f)).output().unwrap();
        logic::parse_info(&String::from_utf8_lossy(&o.stderr))
    }

    /// Volumen medio en dB de la (única) pista de audio.
    fn mean_db(ff: &str, f: &str) -> f64 {
        let o = Command::new(ff).args(["-hide_banner", "-nostdin", "-i", f, "-map", "0:a:0", "-af", "volumedetect", "-f", "null", "-"]).output().unwrap();
        let t = String::from_utf8_lossy(&o.stderr).to_string();
        t.lines().find_map(|l| l.split("mean_volume:").nth(1)).and_then(|v| v.trim().trim_end_matches("dB").trim().parse().ok()).unwrap_or(-99.0)
    }

    fn run(ff: &str, args: &[String]) {
        let st = Command::new(ff).args(args).status().unwrap();
        assert!(st.success(), "falló: {}", args.join(" "));
    }

    /// Graba con el micrófono en pista aparte (juego = 440 Hz, voz = 880 Hz) y prueba
    /// las opciones de sonido, el video vertical y juntar clips, todo con x264 (el servidor no tiene placa).
    #[test]
    fn mic_tracks_vertical_join() {
        let Ok(ff) = std::env::var("RCLIP_FFMPEG") else {
            eprintln!("RCLIP_FFMPEG no definido: se saltea");
            return;
        };
        let tmp = std::env::temp_dir();
        let p = |n: &str| tmp.join(n).to_string_lossy().into_owned();
        let ts = p("jugadon_test_mic.ts");
        let mut args = logic::record_args(11, 720, 30, logic::Source::Screen, Some((48000, 2, true)), logic::Out::File(&ts));
        for a in args.iter_mut() {
            if a.starts_with("ddagrab=") {
                *a = "testsrc2=size=1280x720:rate=30,realtime".into();
            } else if a.contains("hwdownload,format=bgra,") {
                *a = a.replacen("hwdownload,format=bgra,", "", 1);
            }
        }
        eprintln!("args: {}", args.join(" "));
        let mut child = Command::new(&ff).args(&args).stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
        let mut w = child.stdin.take().unwrap();
        let t0 = Instant::now();
        let mut frames = 0u64;
        while t0.elapsed() < Duration::from_secs(6) {
            let mut buf = Vec::with_capacity(4800 * 4 * 4);
            for i in 0..4800u64 {
                let t = (frames + i) as f32 / 48000.0;
                let game = (t * 440.0 * std::f32::consts::TAU).sin() * 0.3;
                let mic = (t * 880.0 * std::f32::consts::TAU).sin() * 0.3;
                for v in [game, game, mic, mic] {
                    buf.extend_from_slice(&v.to_le_bytes());
                }
            }
            w.write_all(&buf).expect("ffmpeg dejó de leer el audio");
            frames += 4800;
            let due = Duration::from_micros(frames * 1_000_000 / 48000);
            if let Some(d) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(d);
            }
        }
        drop(w);
        let _ = child.kill();
        let _ = child.wait();
        let clip = p("jugadon_test_mic.mp4");
        run(&ff, &logic::remux_args(&ts, &clip));
        let i = info(&ff, &clip);
        eprintln!("clip: {i:?}\n{}", probe(&ff, &clip));
        assert_eq!(i.audio, 3, "tienen que quedar 3 pistas (juego+voz, juego, voz)");
        assert!(i.dur > 3.0, "{i:?}");
        let pr = probe(&ff, &clip);
        assert_eq!(pr.matches("channels=2").count(), 2, "{pr}");
        assert_eq!(pr.matches("channels=1").count(), 1, "{pr}");

        // cada opción de sonido: una sola pista, con sonido
        let mut db = Vec::new();
        for (k, (label, tag)) in logic::SOUND_MODES.iter().enumerate() {
            let out = p(&format!("jugadon_test_{tag}.mp4"));
            run(&ff, &logic::sound_args(&clip, k, &out));
            let o = info(&ff, &out);
            let v = mean_db(&ff, &out);
            eprintln!("{label}: {o:?} volumen medio {v} dB");
            assert_eq!(o.audio, 1, "{label}");
            assert!(o.w == 1280 && o.dur > 3.0, "{label}: {o:?}");
            assert!(v > -30.0, "{label}: quedó sin sonido ({v} dB)");
            db.push(v);
        }
        // "mi voz más fuerte" suena más que "mi voz más baja"... y "el juego más bajo" menos que el original
        assert!(db[2] > db[3], "{db:?}");

        // vertical (recortado y con fondo)
        for blur in [false, true] {
            let out = p(&format!("jugadon_test_vertical{}.mp4", blur as u8));
            run(&ff, &logic::vertical_args(11, &clip, blur, &out));
            let o = info(&ff, &out);
            eprintln!("vertical {blur}: {o:?}");
            assert!(o.w == 1080 && o.h == 1920 && o.audio == 1 && o.dur > 3.0, "{o:?}");
        }

        // juntar: el clip de 3 pistas + uno sin audio (lleva silencio) + el clip otra vez
        let mute = p("jugadon_test_mute.mp4");
        run(&ff, &logic::s_test(&["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=854x480:rate=60", "-t", "2", "-c:v", "libx264", "-preset", "ultrafast", &mute]));
        let inputs: Vec<(String, logic::Info)> = [&clip, &mute, &clip].iter().map(|f| (f.to_string(), info(&ff, f))).collect();
        let total: f64 = inputs.iter().map(|x| x.1.dur).sum();
        let out = p("jugadon_test_montaje.mp4");
        let ja = logic::join_args(11, &inputs, &out);
        eprintln!("juntar: {}", ja.join(" "));
        run(&ff, &ja);
        let o = info(&ff, &out);
        eprintln!("montaje: {o:?} (suma {total:.2})");
        assert!(o.w == 1280 && o.h == 720 && o.audio == 1, "{o:?}");
        assert!((o.dur - total).abs() < 1.0, "duración {} vs {total}", o.dur);
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
        let mut args = logic::record_args(11, 720, 30, logic::Source::Window(0), None, logic::Out::File(&ts));
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
        let gdi = logic::ENCODERS.iter().position(|e| e.gdi && e.codec == "libx264").unwrap();
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
        let app = crate::logic::record_args(0, 720, 30, crate::logic::Source::Screen, Some((48000, 2, false)), crate::logic::Out::Segments { secs: 30, dir: "D" });
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
    use windows::core::{w, PCWSTR};
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
    /// terminó la prueba de cómo grabar (wParam = encoder que anduvo, o usize::MAX)
    const WM_PROBED: u32 = WM_APP + 4;
    /// terminó de subir un clip: lParam = Box<(nombre, link o error, vence, permanente)>
    const WM_LINK: u32 = WM_APP + 5;
    /// terminó de buscar versión nueva (wParam = número de la última publicada, 0 = no se supo)
    const WM_UPDATE: u32 = WM_APP + 6;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // El juego siempre gana la CPU: grabar en "debajo de lo normal", guardar/capturar en "baja".
    const BELOW_NORMAL: u32 = 0x0000_4000;
    const IDLE: u32 = 0x0000_0040;
    const TIMER_WATCHDOG: usize = 1;
    const TIMER_BIND: usize = 2;
    /// chequeo 1,5 s después de cambiar de ventana (el juego suele pasar a pantalla completa un toque después)
    const TIMER_FG: usize = 3;
    /// buscar versión nueva: a los 20 s de abrir y después cada 12 h
    const TIMER_UPDATE: usize = 7;
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
        /// ya se comprobó que ffmpeg está escribiendo el archivo
        writing: bool,
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
        /// visor/editor de un clip: solo existe mientras está abierto
        viewer: Option<Viewer>,
        share_token: Option<i64>,
        /// intervalo actual del watchdog: 2 s grabando, 10 s en reposo
        tick_ms: u32,
        /// probando en segundo plano qué forma de grabar anda en esta PC
        probing: bool,
        /// clips que se están subiendo para compartir (no subir dos veces el mismo)
        uploading: std::collections::HashSet<String>,
        rec: Rec,
        binding: u8,
        hook: Option<HHOOK>,
        last_game: Option<Instant>,
        last_trim: Option<Instant>,
        last_file: Option<String>,
        sound: Vec<u8>,
        /// versión nueva publicada (número de compilación), si hay
        update: Option<u32>,
        /// el último globito es el de la versión nueva (tocarlo abre la descarga)
        click_update: std::cell::Cell<bool>,
    }

    /// Cuánto duró de verdad un proceso que ya terminó (desde que arrancó hasta que salió).
    /// Sirve para no confundir "ffmpeg falló al arrancar" con "anduvo un rato y se cortó",
    /// aunque la app se entere varios segundos después.
    fn real_runtime(child: &Child) -> Option<Duration> {
        use windows::Win32::System::Threading::GetProcessTimes;
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        unsafe { GetProcessTimes(HANDLE(child.as_raw_handle() as _), &mut c, &mut e, &mut k, &mut u).ok()? };
        let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
        let (c, e) = (t(c), t(e));
        (e > c).then(|| Duration::from_nanos((e - c) * 100))
    }

    /// Explicación corta del último error de ffmpeg (para el aviso), mirando el final del log.
    fn ffmpeg_hint(log: &Path) -> &'static str {
        let tail = fs::read(log).map(|b| String::from_utf8_lossy(&b[b.len().saturating_sub(6000)..]).to_lowercase()).unwrap_or_default();
        if tail.contains("nvenc api version") || tail.contains("driver does not support") || tail.contains("minimum required nvidia driver") {
            " Actualizá el driver de NVIDIA: la grabación con la placa necesita uno reciente."
        } else if tail.contains("no capable devices found") || tail.contains("openencodesessionex failed") {
            " La placa no aceptó comprimir video (¿otro programa la está usando para grabar?)."
        } else if tail.contains("ddagrab") || tail.contains("duplicat") || tail.contains("d3d11va device") {
            " Windows no dejó capturar la pantalla. Actualizá el driver de video y cerrá otros grabadores."
        } else if tail.contains("gfxcapture") {
            " Windows no dejó capturar esa ventana (necesita Windows 10 2004 o más nuevo)."
        } else {
            " Mirá %TEMP%\\jugadon.log."
        }
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

    fn copy_text_to_clipboard(hwnd: HWND, text: &str) -> bool {
        use windows::Win32::System::DataExchange::*;
        use windows::Win32::System::Memory::*;
        let w: Vec<u16> = text.encode_utf16().chain([0]).collect();
        unsafe {
            let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) else { return false };
            let p = GlobalLock(h) as *mut u16;
            if p.is_null() {
                let _ = GlobalFree(Some(h));
                return false;
            }
            std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
            let _ = GlobalUnlock(h);
            if OpenClipboard(Some(hwnd)).is_err() {
                let _ = GlobalFree(Some(h));
                return false;
            }
            let _ = EmptyClipboard();
            let ok = SetClipboardData(13 /* CF_UNICODETEXT */, Some(HANDLE(h.0))).is_ok();
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
    /// Lo que ffmpeg cuenta de un video (pistas de audio, duración, tamaño, fps).
    fn video_info(ffmpeg: &Path, file: &str) -> logic::Info {
        let mut cmd = Command::new(ffmpeg);
        cmd.args(logic::info_args(file)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        match spawn_hidden(&mut cmd, IDLE).and_then(|c| c.wait_with_output()) {
            Ok(o) => logic::parse_info(&String::from_utf8_lossy(&o.stderr)),
            Err(_) => logic::Info::default(),
        }
    }

    /// Nombre libre para una copia: "clip_X_tag.mp4", o "clip_X_tag2.mp4" si ya existe.
    fn free_name(f: &Path, tag: &str) -> PathBuf {
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut out = f.with_file_name(logic::derived_name(&name, tag));
        let mut n = 2;
        while out.exists() && n < 1000 {
            out = f.with_file_name(logic::derived_name(&name, &format!("{tag}{n}")));
            n += 1;
        }
        out
    }

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

    /// ¿ffmpeg ya escribió algo en el buffer de clips?
    fn buf_has_data(dir: &Path) -> bool {
        fs::read_dir(dir).is_ok_and(|rd| rd.flatten().any(|e| e.metadata().is_ok_and(|m| m.len() > 0)))
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
                return format!("● Grabando {} – {}{a}\ncon {}", m.what, logic::fmt_elapsed(m.t0.elapsed().as_secs()), logic::who(m.enc));
            }
            if self.probing && self.rec.child.is_none() {
                return "Buscando la mejor forma de grabar en esta PC…".to_string();
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
                format!("Clips activos{audio}\ncon {}", logic::who(self.rec.enc))
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
            // el estado también se ve en la ventana "Mis clips"
            self.redraw_gallery();
        }

        fn notify(&self, msg: &str, warn: bool) {
            self.click_update.set(false);
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
            let fmt = audio.as_ref().map(|(a, _)| (a.rate, a.channels, a.split));
            let args = logic::record_args(enc, self.cfg.height, self.cfg.fps, src, fmt, out);
            let audio_desc = fmt.map_or("no".to_string(), |f| format!("{} Hz, {} canales{}", f.0, f.1, if f.2 { ", micrófono en pista aparte" } else { "" }));
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
            self.set_tick(2000);
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
                m.writing = false;
            }
            self.set_tick(2000);
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
            self.manual = Some(Manual { child: None, enc: logic::first_encoder(self.cfg.encoder, self.cfg.gpu_only), tried: 0, started: now, t0: now, ts, src, what, audio: false, writing: false });
            self.spawn_manual();
            self.play_sound();
            let what = self.manual.as_ref().map(|m| m.what.clone()).unwrap_or_default();
            self.notify(&format!("Grabando {what}. Apretá {} para frenar y guardar.", hk(self.cfg.rec)), false);
            self.show_overlay("Grabando", &format!("{} para frenar y guardar", hk(self.cfg.rec)), 1);
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
                let hint = ffmpeg_hint(&self.log_path);
                let gpu = if self.cfg.gpu_only { " Si tu placa no puede, actualizá su driver o activá Opciones → Permitir grabar con el procesador." } else { "" };
                self.notify(&format!("No se pudo grabar.{hint}{gpu}"), true);
                return;
            }
            self.play_sound();
            let name = format!("grabacion_{}.mp4", now_stamp());
            let dur = logic::fmt_elapsed(m.t0.elapsed().as_secs());
            let who = logic::who(m.enc);
            let msg = if cut {
                format!("La grabación se cortó; se guardó lo grabado ({dur}, con {who}): {name}")
            } else {
                format!("Grabación guardada ({dur}, con {who}): {name}")
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

        fn links_path(&self) -> PathBuf {
            self.cfg_path.with_file_name("links.txt")
        }

        /// Link ya subido de este archivo que todavía no venció: "nombre\tlink\tvence (unix, 0 = nunca)".
        fn cached_link(&self, name: &str, permanent: bool) -> Option<String> {
            let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let text = fs::read_to_string(self.links_path()).ok()?;
            text.lines().rev().find_map(|l| {
                let mut it = l.split('\t');
                let (n, url, exp) = (it.next()?, it.next()?, it.next()?.parse::<u64>().ok()?);
                // un link temporal sirve si le queda al menos 1 hora; uno permanente sirve siempre
                let ok = n == name && (exp == 0 || (!permanent && exp > now + 3600));
                ok.then(|| url.to_string())
            })
        }

        /// Compartir por link: sube el clip y copia el link al portapapeles. Lo ve cualquiera, tenga o no la app.
        fn share_link(&mut self, f: PathBuf, permanent: bool) {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some(url) = self.cached_link(&name, permanent) {
                copy_text_to_clipboard(self.hwnd, &url);
                self.notify(&format!("Link copiado al portapapeles: {url}\nPegalo donde quieras."), false);
                return;
            }
            if !self.uploading.insert(name.clone()) {
                self.notify("Ese clip ya se está subiendo: en un ratito tenés el link.", false);
                return;
            }
            let size = fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
            let _ = permanent;
            self.notify(&format!("Subiendo el clip ({}) para compartirlo… Cuando termine, el link queda copiado.", logic::fmt_size(size)), false);
            let log = self.log_path.clone();
            let curl = std::env::var("SystemRoot").map(|r| PathBuf::from(r).join("System32").join("curl.exe")).unwrap_or_else(|_| PathBuf::from("curl.exe"));
            std::thread::spawn(move || {
                // se prueba cada servicio hasta que uno devuelva el link
                let mut got: Option<(String, u32)> = None;
                for (svc, args, hours) in logic::upload_plan(&f.to_string_lossy(), permanent, size) {
                    let mut cmd = Command::new(&curl);
                    cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(log_stdio(&log));
                    let out = spawn_hidden(&mut cmd, BELOW_NORMAL).and_then(|c| c.wait_with_output());
                    let text = out.as_ref().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
                    let url = out.as_ref().ok().filter(|o| o.status.success()).and_then(|_| logic::parse_upload(&text));
                    log_line(&log, &format!("compartir link ({svc}): {}", url.as_deref().unwrap_or("falló")));
                    if let Some(u) = url {
                        got = Some((u, hours));
                        break;
                    }
                }
                let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let (url, hours) = got.unwrap_or_default();
                let exp = if hours == 0 { 0 } else { now + hours as u64 * 3600 };
                let msg = Box::new((name, url, exp, hours));
                unsafe {
                    let raw = Box::into_raw(msg);
                    if PostMessageW(Some(HWND(MAIN_HWND.load(Ordering::Relaxed) as _)), WM_LINK, WPARAM(0), LPARAM(raw as isize)).is_err() {
                        drop(Box::from_raw(raw));
                    }
                }
            });
        }

        fn link_done(&mut self, name: String, url: String, exp: u64, hours: u32) {
            self.uploading.remove(&name);
            if url.is_empty() {
                self.notify("No pude subir el clip (¿hay internet?). Probá de nuevo en un rato.", true);
                return;
            }
            let _ = fs::OpenOptions::new().create(true).append(true).open(self.links_path()).and_then(|mut f| {
                use std::io::Write;
                writeln!(f, "{name}\t{url}\t{exp}")
            });
            copy_text_to_clipboard(self.hwnd, &url);
            self.play_sound();
            let when = match hours {
                0 => "no vence".to_string(),
                h if h <= 72 => format!("se borra solo en {} días", h / 24),
                _ => "se borra si nadie lo mira por unos días".to_string(),
            };
            self.notify(&format!("¡Link copiado al portapapeles! Pegalo donde quieras: lo ve cualquiera, tenga o no Jugadón ({when}).\n{url}"), false);
        }

        fn fav_path(&self) -> PathBuf {
            self.cfg_path.with_file_name("favoritos.txt")
        }

        fn load_favs(&self) -> std::collections::HashSet<String> {
            fs::read_to_string(self.fav_path()).map(|t| t.lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()).collect()).unwrap_or_default()
        }

        fn save_favs(&self, favs: &std::collections::HashSet<String>) {
            let mut v: Vec<&String> = favs.iter().collect();
            v.sort();
            let text: String = v.iter().map(|n| format!("{n}\n")).collect();
            let _ = fs::write(self.fav_path(), text);
        }

        /// Borra solos (a la papelera) los clips viejos según Opciones → Borrar clips viejos solos.
        /// Solo mira la carpeta de clips, solo archivos de Jugadón, nunca favoritos ni lo de hoy.
        fn auto_cleanup(&mut self) {
            if self.cfg.keep_days == 0 && self.cfg.max_gb == 0 {
                return;
            }
            let favs = self.load_favs();
            let now = std::time::SystemTime::now();
            let files: Vec<(String, u32, u64, bool)> = fs::read_dir(&self.cfg.folder)
                .map(|rd| {
                    rd.flatten()
                        .filter_map(|e| {
                            let name = e.file_name().to_string_lossy().into_owned();
                            let ours = (name.starts_with("clip_") || name.starts_with("grabacion") || name.starts_with("shot_")) && (name.ends_with(".mp4") || name.ends_with(".png"));
                            if !ours {
                                return None;
                            }
                            let m = e.metadata().ok()?;
                            let age = now.duration_since(m.modified().ok()?).map_or(0, |d| (d.as_secs() / 86_400) as u32);
                            let fav = favs.contains(&name);
                            Some((name, age, m.len(), fav))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let del = logic::cleanup_plan(files, self.cfg.keep_days, self.cfg.max_gb);
            if del.is_empty() {
                return;
            }
            let dir = PathBuf::from(&self.cfg.folder);
            let mut from: Vec<u16> = Vec::new();
            for n in &del {
                from.extend(dir.join(n).to_string_lossy().encode_utf16());
                from.push(0);
            }
            from.push(0);
            let mut op = SHFILEOPSTRUCTW {
                hwnd: self.hwnd,
                wFunc: FO_DELETE,
                pFrom: PCWSTR(from.as_ptr()),
                fFlags: (FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_SILENT.0 | FOF_NOERRORUI.0) as u16,
                ..Default::default()
            };
            unsafe {
                let _ = SHFileOperationW(&mut op);
            }
            log_line(&self.log_path, &format!("limpieza automática: {} archivos a la papelera", del.len()));
            self.notify(&format!("Limpieza automática: mandé {} clips viejos a la papelera.", del.len()), false);
            self.fill_gallery();
        }

        /// La primera vez (y con cada versión nueva) prueba en segundo plano, de a una y con prioridad baja,
        /// cada forma de grabar: 3 s a un archivo temporal. Se queda con la primera que escribe video de verdad.
        /// Así F8/F9 andan al toque con lo mejor que tenga esta PC, sea cual sea la placa o el driver.
        fn start_probe(&mut self) {
            if self.probing || self.cfg.probed == logic::BUILD {
                return;
            }
            self.probing = true;
            log_line(&self.log_path, "prueba de grabación: buscando la forma que anda en esta PC");
            let (ffmpeg, log) = (self.ffmpeg.clone(), self.log_path.clone());
            let (height, fps, gpu_only) = (self.cfg.height, self.cfg.fps, self.cfg.gpu_only);
            let test = std::env::temp_dir().join("jugadon_prueba.ts");
            std::thread::spawn(move || {
                let mut found = usize::MAX;
                for e in (0..ENCODERS.len()).filter(|&e| logic::allowed(e, gpu_only)) {
                    let _ = fs::remove_file(&test);
                    let path = test.to_string_lossy().into_owned();
                    let args = logic::record_args(e, height, fps, Source::Screen, None, logic::Out::Test(&path));
                    let mut cmd = Command::new(&ffmpeg);
                    cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_stdio(&log));
                    let Ok(mut child) = spawn_hidden(&mut cmd, IDLE) else { break };
                    let t0 = Instant::now();
                    // 3 s de video + arranque; si se cuelga (driver que no responde) se corta a los 15 s
                    let status = loop {
                        match child.try_wait() {
                            Ok(Some(st)) => break Some(st),
                            Ok(None) if t0.elapsed() < Duration::from_secs(15) => std::thread::sleep(Duration::from_millis(250)),
                            _ => {
                                let _ = child.kill();
                                let _ = child.wait();
                                break None;
                            }
                        }
                    };
                    let size = fs::metadata(&test).map(|m| m.len()).unwrap_or(0);
                    let ok = status.is_some_and(|s| s.success()) && size > 0;
                    log_line(&log, &format!("prueba de grabación: {} {} ({} KB)", ENCODERS[e].label, if ok { "anda" } else { "no anda" }, size / 1024));
                    if ok {
                        found = e;
                        break;
                    }
                }
                let _ = fs::remove_file(&test);
                unsafe {
                    let _ = PostMessageW(Some(HWND(MAIN_HWND.load(Ordering::Relaxed) as _)), WM_PROBED, WPARAM(found), LPARAM(0));
                }
            });
        }

        fn probe_done(&mut self, enc: usize) {
            self.probing = false;
            if enc >= ENCODERS.len() {
                log_line(&self.log_path, "prueba de grabación: ninguna forma anduvo");
                let hint = ffmpeg_hint(&self.log_path);
                let gpu = if self.cfg.gpu_only { " Actualizá el driver de la placa, o activá Opciones → Permitir grabar con el procesador." } else { "" };
                self.notify(&format!("No encontré cómo grabar video en esta PC.{hint}{gpu}"), true);
                return;
            }
            self.cfg.encoder = enc;
            self.cfg.probed = logic::BUILD.to_string();
            self.save_cfg();
            let (h, f) = logic::effective(enc, self.cfg.height, self.cfg.fps);
            log_line(&self.log_path, &format!("prueba de grabación: se usa {} ({h}p {f} fps)", ENCODERS[enc].label));
            // el buffer de clips arranca (o se pasa) directo a lo que anduvo
            if !self.rec.good || self.rec.gave_up || self.rec.child.is_none() {
                self.rec.enc = enc;
                if self.want_rec() {
                    self.restart_rec();
                }
            }
            self.update_tip();
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
            self.set_tick(if busy { 2000 } else { 10_000 });
        }

        fn set_tick(&mut self, ms: u32) {
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
                let mut stalled = false;
                if let Some(c) = m.child.as_mut() {
                    if let Ok(None) = c.try_wait() {
                        // ¿está escribiendo de verdad? Algunos drivers dejan a ffmpeg abierto sin sacar ningún cuadro
                        if !m.writing && m.started.elapsed() >= QUICK_DEATH {
                            if fs::metadata(&m.ts).is_ok_and(|x| x.len() > 0) {
                                m.writing = true;
                                log_line(&self.log_path, &format!("grabación escribiendo con {}", ENCODERS[m.enc].label));
                            } else {
                                log_line(&self.log_path, &format!("{} quedó abierto sin escribir nada: pruebo otra forma", ENCODERS[m.enc].label));
                                let _ = c.kill();
                                let _ = c.wait();
                                stalled = true;
                            }
                        }
                        if !stalled {
                            self.update_tip(); // muestra el tiempo grabado
                            return;
                        }
                    }
                }
                // ffmpeg de la grabación manual terminó solo (o no escribía)
                let ran = if stalled { Duration::ZERO } else { m.child.as_ref().and_then(real_runtime).unwrap_or(Duration::ZERO) };
                log_line(&self.log_path, &format!("ffmpeg (grabación) terminó después de {} s con {}", ran.as_secs(), ENCODERS[m.enc].label));
                match logic::next_encoder(m.enc, m.tried, self.cfg.gpu_only) {
                    Some((e, t)) if ran < QUICK_DEATH => {
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
                    Ok(None) if !self.rec.good && self.rec.started.elapsed() >= QUICK_DEATH && !buf_has_data(&self.buf_dir) => {
                        // abierto pero sin escribir nada (driver que se cuelga): se espera un poco y si no, a otra forma
                        if self.rec.started.elapsed() < Duration::from_secs(12) {
                            return;
                        }
                        log_line(&self.log_path, &format!("{} quedó abierto sin escribir nada: pruebo otra forma", ENCODERS[self.rec.enc].label));
                        let _ = c.kill();
                        let _ = c.wait();
                        "sin escribir".to_string()
                    }
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
                let ran = if status == "sin escribir" {
                    Duration::ZERO
                } else {
                    self.rec.child.as_ref().and_then(real_runtime).unwrap_or_else(|| self.rec.started.elapsed())
                };
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
                                let hint = ffmpeg_hint(&self.log_path);
                                self.notify(&format!("La placa de video no pudo grabar.{hint} Para grabar igual con el procesador: Opciones → \"Permitir grabar con el procesador\"."), true);
                            } else {
                                let hint = ffmpeg_hint(&self.log_path);
                                self.notify(&format!("No pude iniciar la grabación.{hint}"), true);
                            }
                            self.update_tip();
                            return;
                        }
                    }
                }
                self.update_tip();
            }
            // mientras se prueba cómo grabar no se arranca el buffer (no hacer el trabajo dos veces)
            if self.rec.child.is_none() && !self.rec.gave_up && !self.probing && self.rec.next_try.is_none_or(|t| now >= t) {
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
            let who = logic::who(self.rec.enc);
            let out = dir.join(&name).to_string_lossy().into_owned();
            let args = logic::clip_args(&segs, &out);
            self.play_sound();
            let (ffmpeg, log) = (self.ffmpeg.clone(), self.log_path.clone());
            std::thread::spawn(move || {
                let mut cmd = Command::new(ffmpeg);
                cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_stdio(&log));
                let ok = spawn_hidden(&mut cmd, IDLE).and_then(|mut c| c.wait()).is_ok_and(|s| s.success());
                if ok {
                    post_text(format!("Clip guardado (con {who}): {name}\nClic acá para verlo."), Some(out));
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
                if let Some(v) = self.update {
                    add(m, 51, &format!("⬆ Bajar la versión nueva (v1.0.{v})"), false);
                }
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
                let so = CreatePopupMenu().unwrap_or_default();
                for (i, name) in ["Tin-tin", "Suave", "Moneda"].iter().enumerate() {
                    add(so, 70 + i, name, self.cfg.sound && self.cfg.sound_style == i as u32);
                }
                sep(so);
                add(so, 44, "Sin sonido", !self.cfg.sound);
                let t = wide("Sonido al guardar");
                let _ = AppendMenuW(o, MF_POPUP, so.0 as usize, PCWSTR(t.as_ptr()));
                let cl = CreatePopupMenu().unwrap_or_default();
                for (i, d) in [0u32, 7, 30, 90].iter().enumerate() {
                    let label = if *d == 0 { "Nunca por antigüedad".to_string() } else { format!("Los de más de {d} días") };
                    add(cl, 60 + i, &label, self.cfg.keep_days == *d);
                }
                sep(cl);
                for (i, g) in [0u32, 10, 25, 50].iter().enumerate() {
                    let label = if *g == 0 { "Sin límite de espacio".to_string() } else { format!("Los más viejos si pasan de {g} GB") };
                    add(cl, 64 + i, &label, self.cfg.max_gb == *g);
                }
                sep(cl);
                let t = wide("(los favoritos nunca se borran; todo va a la papelera)");
                let _ = AppendMenuW(cl, MF_STRING | MF_GRAYED, 0, PCWSTR(t.as_ptr()));
                let t = wide("Borrar clips viejos solos");
                let _ = AppendMenuW(o, MF_POPUP, cl.0 as usize, PCWSTR(t.as_ptr()));
                add(o, 48, "Permitir grabar con el procesador si la placa no puede", !self.cfg.gpu_only);
                sep(o);
                add(o, 30, "Tecla de clip…", false);
                add(o, 31, "Tecla de captura…", false);
                add(o, 32, "Tecla de grabar/frenar…", false);
                sep(o);
                add(o, 52, "Cartel arriba del juego al guardar", self.cfg.overlay);
                add(o, 53, "Avisarme si hay una versión nueva", self.cfg.updates);
                sep(o);
                add(o, 41, "Iniciar con Windows", autostart_enabled());
                add(o, 49, "Volver a probar cómo grabar (después de cambiar el driver)", false);
                add(o, 42, "Ver registro (si algo no anda)", false);
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
                    if self.cfg.gpu_only && !logic::allowed(self.cfg.encoder, true) {
                        // lo elegido era el procesador: buscar de nuevo una forma con la placa
                        self.cfg.probed.clear();
                    }
                    self.save_cfg();
                    self.notify(
                        if self.cfg.gpu_only {
                            "Solo placa de video: el procesador no se usa nunca para grabar."
                        } else {
                            "Primero se prueba siempre la placa; el procesador solo si ninguna forma de usar la placa anda (480p, prioridad baja)."
                        },
                        false,
                    );
                    self.restart_rec();
                    if self.cfg.probed.is_empty() {
                        self.start_probe();
                    }
                }
                70..=72 => {
                    self.cfg.sound = true;
                    self.cfg.sound_style = (id - 70) as u32;
                    self.sound = logic::clip_sound_wav(self.cfg.sound_style);
                    self.save_cfg();
                    self.play_sound();
                }
                60..=67 => {
                    if id < 64 {
                        self.cfg.keep_days = [0, 7, 30, 90][id - 60];
                    } else {
                        self.cfg.max_gb = [0, 10, 25, 50][id - 64];
                    }
                    self.save_cfg();
                    self.auto_cleanup();
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
                49 => {
                    self.cfg.probed.clear();
                    self.save_cfg();
                    self.stop_rec();
                    self.rec.good = false;
                    self.notify("Probando cómo grabar en esta PC (unos segundos)…", false);
                    self.start_probe();
                }
                42 => {
                    if !self.log_path.exists() {
                        log_line(&self.log_path, "registro abierto desde el menú");
                    }
                    let _ = Command::new("notepad.exe").arg(&self.log_path).spawn();
                }
                50 => return true,
                51 => self.open_update(),
                52 => {
                    self.cfg.overlay = !self.cfg.overlay;
                    self.save_cfg();
                    if self.cfg.overlay {
                        self.show_overlay("Así se ve el cartel", "Aparece unos segundos al guardar", 0);
                    }
                }
                53 => {
                    self.cfg.updates = !self.cfg.updates;
                    self.save_cfg();
                    if self.cfg.updates {
                        self.check_updates();
                    }
                }
                // prueba automática: hacer de cuenta que salió una versión nueva
                54 => self.update_found(u32::MAX),
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
                    sc(1080),
                    sc(700),
                    None,
                    None,
                    Some(hinst),
                    None,
                ) else {
                    log_line(&self.log_path, &format!("no se pudo crear la ventana de clips: {:?}", GetLastError()));
                    return;
                };
                log_line(&self.log_path, "ventana de clips abierta");
                dark_title_bar(hwnd);
                let _ = SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_SMALL as usize)), Some(LPARAM(self.icon_on.0 as isize)));
                let big = load_icon_size(hinst, 1, GetSystemMetrics(SM_CXICON));
                let _ = SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_BIG as usize)), Some(LPARAM(big.0 as isize)));
                let font = |px: i32, weight: i32, face: PCWSTR| {
                    CreateFontW(-sc(px), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, face)
                };
                let ui = ui_face();
                let ic = icon_face();
                let fonts = Fonts {
                    brand: font(24, 800, ui),
                    h1: font(26, 700, ui),
                    body: font(14, 400, ui),
                    bold: font(14, 600, ui),
                    small: font(12, 400, ui),
                    badge: font(11, 700, ui),
                    icon: font(16, 400, ic),
                    icon_big: font(22, 400, ic),
                };
                // hilo de miniaturas: vive mientras la ventana está abierta (se corta al soltar el Sender)
                let (tx, rx) = std::sync::mpsc::channel::<ThumbReq>();
                start_thumb_thread(hwnd, rx);
                self.gallery = Some(Gallery {
                    hwnd,
                    logo: load_icon_size(hinst, 1, sc(96)),
                    fonts,
                    items: Vec::new(),
                    view: Vec::new(),
                    nav: 0,
                    counts: [0; 4],
                    total: 0,
                    sel: None,
                    hover: Hit::None,
                    scroll: 0,
                    content_h: 0,
                    page_h: 0,
                    cols: 1,
                    hits: Vec::new(),
                    visible: Vec::new(),
                    gen: 1,
                    tx: Some(tx),
                    drag: None,
                    tracking: false,
                    focus: None,
                    target: None,
                    favs: std::collections::HashSet::new(),
                    joins: Vec::new(),
                    update: self.update,
                    edit: None,
                    edit_brush: CreateSolidBrush(COLORREF(rgb(52, 46, 72))),
                });
                self.fill_gallery();
                // refresco del reloj de grabación y del estado (solo mientras la ventana está abierta)
                SetTimer(Some(hwnd), 1, 1000, None);
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
            }
        }

        /// Lee la carpeta de clips (solo nombres, tamaños y fechas). Las miniaturas se piden al dibujar.
        fn fill_gallery(&mut self) {
            let folder = self.cfg.folder.clone();
            if self.gallery.is_none() {
                return;
            }
            let favs = self.load_favs();
            let Some(g) = self.gallery.as_mut() else { return };
            g.favs = favs;
            let mut files: Vec<(std::time::SystemTime, PathBuf, u64)> = fs::read_dir(&folder)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4") || x.eq_ignore_ascii_case("png")))
                        .filter_map(|e| {
                            let m = e.metadata().ok()?;
                            Some((m.modified().ok()?, e.path(), m.len()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            files.sort_by_key(|f| std::cmp::Reverse(f.0));
            files.truncate(3000);
            let sel_path = g.sel.and_then(|i| g.items.get(i)).map(|it| it.path.clone());
            // las miniaturas que ya estaban se reusan: al borrar o guardar un clip no "parpadea" nada
            type Kept = (HBITMAP, Option<u32>, (i32, i32));
            let mut keep: std::collections::HashMap<PathBuf, Kept> =
                g.items.iter_mut().filter_map(|it| Some((it.path.clone(), (it.thumb.take()?, it.dur, it.tsize)))).collect();
            g.gen += 1;
            g.items = files
                .into_iter()
                .map(|(_, path, len)| {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let (kind, date) = logic::describe_file(&name);
                    let k = match kind {
                        "Clip" => 0,
                        "Grabación" => 1,
                        "Captura" => 2,
                        _ => 3,
                    };
                    let title = match logic::custom_title(&name) {
                        Some(t) => t.to_string(),
                        None if date.is_empty() => name.clone(),
                        None => format!("{kind} del {date}"),
                    };
                    let (thumb, dur, tsize) = keep.remove(&path).map_or((None, None, (0, 0)), |(b, d, t)| (Some(b), d, t));
                    Item { path, name, kind: k, title, size: len, dur, asked: false, thumb, tsize }
                })
                .collect();
            for (_, (b, _, _)) in keep {
                unsafe {
                    let _ = DeleteObject(b.into());
                }
            }
            g.counts = [0; 4];
            for it in &g.items {
                g.counts[it.kind as usize] += 1;
            }
            g.total = g.items.iter().map(|i| i.size).sum();
            g.sel = sel_path.and_then(|p| g.items.iter().position(|i| i.path == p));
            g.rebuild_view();
            unsafe {
                let _ = InvalidateRect(Some(g.hwnd), None, false);
            }
        }

        /// Copias de un video sin tocar el original: 0 = últimos N s, 1 = sin los primeros N s, 2 = para Discord.
        /// Corren en un hilo con prioridad baja; al terminar avisan y la ventana se actualiza.
        fn make_copy(&self, op: u8, secs: u32) {
            let Some(f) = self.selected_file() else {
                self.notify("Elegí un clip de la lista primero.", false);
                return;
            };
            let dur = self.gallery.as_ref().and_then(|g| g.items.get(g.sel?)?.dur);
            self.copy_of(f, op, secs as f64, 0.0, dur);
        }

        /// op: 0 = últimos `a` s, 1 = sin los primeros `a` s, 2 = para Discord, 3 = recorte de `a` a `b` s.
        fn copy_of(&self, f: PathBuf, op: u8, a: f64, b: f64, dur: Option<u32>) {
            let secs = a as u32;
            if !f.extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4")) {
                self.notify("Esto es solo para videos.", false);
                return;
            }
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let tag = match op {
                0 => format!("ultimos{secs}s"),
                1 => format!("sin{secs}s"),
                3 => "recorte".to_string(),
                _ => "discord".to_string(),
            };
            let out = f.with_file_name(logic::derived_name(&name, &tag));
            let (ffmpeg, log, enc) = (self.ffmpeg.clone(), self.log_path.clone(), self.cfg.encoder.min(ENCODERS.len() - 1));
            if op == 2 {
                self.notify("Achicando el clip para Discord (lo comprime la placa, puede tardar unos segundos)…", false);
            }
            std::thread::spawn(move || {
                let (src, dst) = (f.to_string_lossy().into_owned(), out.to_string_lossy().into_owned());
                let args = match op {
                    0 => logic::trim_args(&src, secs, &dst),
                    1 => logic::trim_start_args(&src, secs, &dst),
                    3 => logic::cut_args(&src, a, b, &dst),
                    _ => logic::discord_args(enc, &src, dur.unwrap_or(60), &dst),
                };
                let mut cmd = Command::new(ffmpeg);
                cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_stdio(&log));
                let ok = spawn_hidden(&mut cmd, IDLE).and_then(|mut c| c.wait()).is_ok_and(|s| s.success());
                let size = fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
                if ok && size > 0 {
                    let msg = match op {
                        0 => format!("Listo: quedaron los últimos {secs} s en un clip nuevo (el original no se tocó)."),
                        1 => format!("Listo: copia sin los primeros {secs} s (el original no se tocó)."),
                        3 => format!("Recorte guardado ({}): {}", logic::fmt_elapsed((b - a).round() as u64), out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                        _ if size > 10 << 20 => format!("Copia para Discord lista, pero quedó de {} (el clip es muy largo: acortalo primero).", logic::fmt_size(size)),
                        _ => format!("Copia para Discord lista ({}): copiala y pegala en Discord.", logic::fmt_size(size)),
                    };
                    post_text(msg, Some(dst));
                } else {
                    let _ = fs::remove_file(&out);
                    post_text("No pude hacer la copia del clip (mirá el registro).".to_string(), None);
                }
            });
        }

        /// Copias que se rehacen: 0 = vertical recortado, 1 = vertical con fondo desenfocado,
        /// 10 + k = sonido (logic::SOUND_MODES[k]). En un hilo con prioridad baja; el original no se toca.
        fn special_copy(&self, f: PathBuf, op: usize) {
            if !f.extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4")) {
                self.notify("Esto es solo para videos.", false);
                return;
            }
            let tag = match op {
                0 => "vertical",
                1 => "vfondo",
                k if k >= 10 && k - 10 < logic::SOUND_MODES.len() => logic::SOUND_MODES[k - 10].1,
                _ => return,
            };
            let out = free_name(&f, tag);
            let (ffmpeg, log, enc) = (self.ffmpeg.clone(), self.log_path.clone(), self.cfg.encoder.min(ENCODERS.len() - 1));
            if op < 10 {
                self.notify("Armando la versión vertical (1080×1920, la hace la placa; puede tardar unos segundos)…", false);
            }
            std::thread::spawn(move || {
                let (src, dst) = (f.to_string_lossy().into_owned(), out.to_string_lossy().into_owned());
                if op >= 10 && video_info(&ffmpeg, &src).audio < 3 {
                    post_text(
                        "No se puede separar tu voz de este clip: se grabó con el micrófono apagado o con una versión vieja. Los clips nuevos con micrófono ya traen tu voz y el juego por separado.".into(),
                        None,
                    );
                    return;
                }
                let args = if op < 10 { logic::vertical_args(enc, &src, op == 1, &dst) } else { logic::sound_args(&src, op - 10, &dst) };
                let mut cmd = Command::new(&ffmpeg);
                cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_stdio(&log));
                let ok = spawn_hidden(&mut cmd, IDLE).and_then(|mut c| c.wait()).is_ok_and(|s| s.success());
                let size = fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
                let fname = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if ok && size > 0 {
                    let msg = if op < 10 {
                        format!("Listo: versión vertical para TikTok, Reels y Shorts ({}): {fname}", logic::fmt_size(size))
                    } else {
                        format!("Listo, sonido nuevo ({}): {fname}", logic::SOUND_MODES[op - 10].0.to_lowercase())
                    };
                    post_text(msg, Some(dst));
                } else {
                    let _ = fs::remove_file(&out);
                    post_text("No pude hacer la copia del clip (mirá el registro).".to_string(), None);
                }
            });
        }

        /// Marcar / desmarcar el clip elegido para juntarlo con otros (en el orden en que se marcan).
        fn toggle_join(&mut self) {
            let Some(f) = self.selected_file() else { return };
            if !f.extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4")) {
                self.notify("Solo se pueden juntar videos.", false);
                return;
            }
            let Some(g) = self.gallery.as_mut() else { return };
            match g.joins.iter().position(|p| *p == f) {
                Some(i) => {
                    g.joins.remove(i);
                }
                None => g.joins.push(f),
            }
            g.redraw();
        }

        /// Junta los clips marcados en un video nuevo (los originales no se tocan).
        fn join_marked(&mut self) {
            let files: Vec<PathBuf> = match self.gallery.as_ref() {
                Some(g) => g.joins.iter().filter(|p| p.exists()).cloned().collect(),
                None => return,
            };
            if files.len() < 2 {
                self.notify("Marcá al menos 2 clips para juntar: clic derecho → Marcar para juntar (o Ctrl + clic en el clip).", false);
                return;
            }
            if let Some(g) = self.gallery.as_mut() {
                g.joins.clear();
                g.redraw();
            }
            let out = free_name(&files[0], "montaje");
            let n = files.len();
            let (ffmpeg, log, enc) = (self.ffmpeg.clone(), self.log_path.clone(), self.cfg.encoder.min(ENCODERS.len() - 1));
            self.notify(&format!("Juntando {n} clips en un solo video (lo hace la placa; puede tardar un poco)…"), false);
            std::thread::spawn(move || {
                let inputs: Vec<(String, logic::Info)> = files
                    .iter()
                    .map(|p| {
                        let ps = p.to_string_lossy().into_owned();
                        let info = video_info(&ffmpeg, &ps);
                        (ps, info)
                    })
                    .collect();
                let total: f64 = inputs.iter().map(|x| x.1.dur).sum();
                let dst = out.to_string_lossy().into_owned();
                let mut cmd = Command::new(&ffmpeg);
                cmd.args(logic::join_args(enc, &inputs, &dst)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_stdio(&log));
                let ok = spawn_hidden(&mut cmd, IDLE).and_then(|mut c| c.wait()).is_ok_and(|s| s.success());
                let fname = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if ok && fs::metadata(&out).is_ok_and(|m| m.len() > 0) {
                    post_text(format!("Listo: juntaste {n} clips en un solo video ({}): {fname}", logic::fmt_elapsed(total.round() as u64)), Some(dst));
                } else {
                    let _ = fs::remove_file(&out);
                    post_text("No pude juntar los clips (mirá el registro).".to_string(), None);
                }
            });
        }

        /// Busca en GitHub si salió una versión nueva (un pedido chico con curl.exe de Windows).
        fn check_updates(&self) {
            if !self.cfg.updates || logic::BUILD.parse::<u32>().is_err() {
                return;
            }
            let hw = self.hwnd.0 as isize;
            let log = self.log_path.clone();
            std::thread::spawn(move || {
                let curl = std::env::var("SystemRoot").map(|r| PathBuf::from(r).join("System32").join("curl.exe")).unwrap_or_else(|_| PathBuf::from("curl.exe"));
                let mut cmd = Command::new(curl);
                cmd.args(logic::update_args()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
                let v = spawn_hidden(&mut cmd, IDLE)
                    .and_then(|c| c.wait_with_output())
                    .ok()
                    .and_then(|o| logic::latest_version(&String::from_utf8_lossy(&o.stdout)))
                    .unwrap_or(0);
                let what = if v > 0 { format!("v1.0.{v}") } else { "no se supo (¿sin internet?)".to_string() };
                log_line(&log, &format!("buscar versión nueva: la última publicada es {what} (esta es la {})", logic::BUILD));
                unsafe {
                    let _ = PostMessageW(Some(HWND(hw as _)), WM_UPDATE, WPARAM(v as usize), LPARAM(0));
                }
            });
        }

        fn update_found(&mut self, v: u32) {
            if v == 0 || !logic::is_newer(logic::BUILD, v) || self.update == Some(v) {
                return;
            }
            self.update = Some(v);
            log_line(&self.log_path, &format!("hay una versión nueva: v1.0.{v}"));
            if let Some(g) = self.gallery.as_mut() {
                g.update = Some(v);
                g.redraw();
            }
            self.notify(&format!("Salió una versión nueva de Jugadón (v1.0.{v}). Tocá acá para bajarla."), false);
            self.click_update.set(true);
        }

        fn open_update(&self) {
            let url = wide(&format!("{}/releases/latest", logic::REPO_URL));
            unsafe {
                ShellExecuteW(None, w!("open"), PCWSTR(url.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
            }
        }

        /// Cartelito arriba del juego según el aviso (solo para lo que pasa mientras jugás).
        fn overlay_for(&self, text: &str) {
            let starts = |p: &[&str]| p.iter().any(|x| text.starts_with(x));
            let (title, sub, kind) = if starts(&["Clip guardado"]) {
                ("Clip guardado", format!("Últimos {} s · está en Mis clips", self.cfg.secs), 0)
            } else if starts(&["Captura guardada"]) {
                ("Captura guardada", "Está en Mis clips".to_string(), 0)
            } else if starts(&["Grabación guardada", "La grabación se cortó"]) {
                ("Grabación guardada", "Está en Mis clips".to_string(), 0)
            } else if starts(&["No se pudo guardar el clip", "No se pudo sacar la captura"]) {
                ("No se pudo guardar", "Mirá el aviso de la bandeja".to_string(), 2)
            } else {
                return;
            };
            self.show_overlay(title, &sub, kind);
        }

        /// kind: 0 listo, 1 grabando, 2 problema.
        fn show_overlay(&self, title: &str, sub: &str, kind: u8) {
            if !self.cfg.overlay {
                return;
            }
            let ok = unsafe { overlay_show(title, sub, kind) };
            log_line(&self.log_path, &if ok { format!("cartel en pantalla: {title}") } else { "cartel en pantalla: este Windows no deja ocultarlo de la grabación (no se muestra)".to_string() });
        }

        fn toggle_fav(&mut self) {
            let Some(f) = self.selected_file() else { return };
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let mut favs = self.load_favs();
            if !favs.remove(&name) {
                favs.insert(name);
            }
            self.save_favs(&favs);
            if let Some(g) = self.gallery.as_mut() {
                g.favs = favs;
                g.rebuild_view();
                g.redraw();
            }
        }

        /// Renombrar: el cuadro de texto aparece sobre el título de la tarjeta.
        fn begin_rename(&mut self) {
            let Some(g) = self.gallery.as_mut() else { return };
            let Some(sel) = g.sel else { return };
            if g.edit.is_some() {
                return;
            }
            let Some(r) = g.hits.iter().find(|h| h.1 == Hit::Card(sel)).map(|h| h.0) else { return };
            let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(g.hwnd) }.max(96) as i32;
            let s = |v: i32| v * dpi / 96;
            let it = &g.items[sel];
            let name = it.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let current = logic::custom_title(&name).unwrap_or("").to_string();
            let t = wide(&current);
            unsafe {
                let hinst: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
                let Ok(e) = CreateWindowExW(
                    WS_EX_NOPARENTNOTIFY,
                    w!("EDIT"),
                    PCWSTR(t.as_ptr()),
                    WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32),
                    r.left + s(10),
                    r.bottom - s(54),
                    r.right - r.left - s(20),
                    s(26),
                    Some(g.hwnd),
                    Some(HMENU(400 as _)),
                    Some(hinst),
                    None,
                ) else {
                    return;
                };
                let _ = SendMessageW(e, WM_SETFONT, Some(WPARAM(g.fonts.bold.0 as usize)), Some(LPARAM(1)));
                let _ = SendMessageW(e, EM_SETLIMITTEXT, Some(WPARAM(60)), None);
                let _ = SendMessageW(e, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                let _ = SetWindowSubclass(e, Some(edit_proc), 1, 0);
                g.edit = Some((e, sel));
                let _ = SetFocus(Some(e));
            }
        }

        fn end_rename(&mut self, commit: bool) {
            let Some(g) = self.gallery.as_mut() else { return };
            let Some((e, idx)) = g.edit.take() else { return };
            let mut buf = [0u16; 128];
            let n = unsafe { GetWindowTextW(e, &mut buf) }.max(0) as usize;
            let text = String::from_utf16_lossy(&buf[..n]);
            unsafe {
                let _ = DestroyWindow(e);
                let _ = SetFocus(Some(g.hwnd));
            }
            let Some(path) = g.items.get(idx).map(|i| i.path.clone()) else { return };
            if !commit {
                g.redraw();
                return;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let Some(new) = logic::renamed(&name, &text) else {
                g.redraw();
                return;
            };
            if new == name {
                return;
            }
            let to = path.with_file_name(&new);
            if to.exists() || fs::rename(&path, &to).is_err() {
                self.notify("No pude cambiarle el nombre (¿está abierto en otro programa?).", true);
                return;
            }
            let mut favs = self.load_favs();
            if favs.remove(&name) {
                favs.insert(new);
                self.save_favs(&favs);
            }
            self.fill_gallery();
        }

        fn selected_file(&self) -> Option<PathBuf> {
            let g = self.gallery.as_ref()?;
            g.items.get(g.sel?).map(|i| i.path.clone())
        }

        fn gallery_cmd(&mut self, id: usize) {
            match id {
                206 => {
                    if let Some(d) = self.out_dir() {
                        let _ = Command::new("explorer.exe").arg(d).spawn();
                    }
                    return;
                }
                207 => {
                    self.fill_gallery();
                    return;
                }
                // dejar solo los últimos 10/15/30/60 s, o sacar los primeros 5/10/30 s (sin recomprimir)
                250..=253 => {
                    self.make_copy(0, [10, 15, 30, 60][id - 250]);
                    return;
                }
                254..=256 => {
                    self.make_copy(1, [5, 10, 30][id - 254]);
                    return;
                }
                260 => {
                    self.make_copy(2, 0);
                    return;
                }
                281 => {
                    self.join_marked();
                    return;
                }
                282 => {
                    if let Some(g) = self.gallery.as_mut() {
                        g.joins.clear();
                        g.redraw();
                    }
                    return;
                }
                208 => {
                    self.toggle_fav();
                    return;
                }
                209 => {
                    // se arma después de dibujar (para saber dónde está la tarjeta)
                    if let Some(g) = self.gallery.as_mut() {
                        g.reveal_sel();
                        g.redraw();
                        unsafe {
                            let _ = PostMessageW(Some(g.hwnd), WM_RENAME, WPARAM(0), LPARAM(0));
                        }
                    }
                    return;
                }
                // prueba automática: elegir la tarjeta N de lo que se ve
                210..=239 => {
                    if let Some(g) = self.gallery.as_mut() {
                        g.sel = g.view.get(id - 210).copied();
                        g.redraw();
                    }
                    return;
                }
                // prueba automática: elegir la sección
                240..=244 => {
                    if let Some(g) = self.gallery.as_mut() {
                        g.set_nav((id - 240) as u8);
                    }
                    return;
                }
                _ => {}
            }
            let Some(f) = self.selected_file() else {
                self.notify("Elegí un clip de la lista primero.", false);
                return;
            };
            let fs_ = f.to_string_lossy().into_owned();
            match id {
                201 if f.extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4")) => {
                    let (title, dur) = self.gallery.as_ref().and_then(|g| g.items.get(g.sel?)).map(|it| (it.title.clone(), it.dur)).unwrap_or_default();
                    self.open_viewer(f.clone(), title, dur);
                }
                201 | 272 => {
                    let p = wide(&fs_);
                    unsafe {
                        ShellExecuteW(None, w!("open"), PCWSTR(p.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
                    }
                }
                202 => self.share_link(f.clone(), false),
                270 => self.share_link(f.clone(), true),
                262 | 263 => self.special_copy(f.clone(), id - 262),
                290..=294 => self.special_copy(f.clone(), 10 + id - 290),
                280 => self.toggle_join(),
                271 => {
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
                        fFlags: (FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_SILENT.0) as u16,
                        ..Default::default()
                    };
                    unsafe {
                        let _ = SHFileOperationW(&mut op);
                    }
                    if !f.exists() {
                        self.notify("Mandado a la papelera (si te arrepentiste, lo recuperás de ahí).", false);
                        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let mut favs = self.load_favs();
                        if favs.remove(&name) {
                            self.save_favs(&favs);
                        }
                    }
                    self.fill_gallery();
                }
                _ => {}
            }
        }

        /// Clic en la ventana. Lo que abre menús se devuelve para hacerlo fuera del préstamo de App.
        fn gallery_click(&mut self, x: i32, y: i32, double: bool) -> Option<Outside> {
            let g = self.gallery.as_mut()?;
            let hit = g.hit_at(x, y);
            if g.focus.is_some() {
                g.focus = None;
                g.redraw();
            }
            self.gallery_activate(hit, double, y)
        }

        /// Lo que hace cada parte de la ventana (con el mouse o con Enter/Espacio).
        fn gallery_activate(&mut self, hit: Hit, double: bool, y: i32) -> Option<Outside> {
            let g = self.gallery.as_mut()?;
            match hit {
                Hit::Nav(n) => g.set_nav(n),
                Hit::Card(i) => {
                    // un toque en el clip lo abre para verlo y editarlo (el doble clic no lo abre dos veces)
                    g.sel = Some(i);
                    g.redraw();
                    // Ctrl + clic: marcar para juntar
                    if unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0 {
                        self.toggle_join();
                    } else if !double {
                        self.gallery_cmd(201);
                    }
                }
                Hit::Play(i) => {
                    g.sel = Some(i);
                    g.redraw();
                    self.gallery_cmd(201);
                }
                Hit::Act(i, cmd) => {
                    g.sel = Some(i);
                    g.redraw();
                    self.gallery_cmd(cmd);
                }
                Hit::Btn(0) => {
                    self.toggle_manual_screen();
                    self.redraw_gallery();
                }
                Hit::Btn(1) => self.save_clip(),
                Hit::Btn(2) => self.gallery_cmd(206),
                Hit::Btn(4) => self.join_marked(),
                Hit::Btn(5) => {
                    g.joins.clear();
                    g.redraw();
                }
                Hit::Btn(6) => self.open_update(),
                Hit::Btn(7) => {}
                Hit::Btn(_) => return Some(Outside::Menu),
                Hit::Bar => {
                    g.drag = Some(y);
                    g.drag_to(y);
                    unsafe {
                        SetCapture(g.hwnd);
                    }
                }
                Hit::None => {
                    if g.sel.take().is_some() {
                        g.redraw();
                    }
                }
            }
            None
        }

        fn redraw_gallery(&self) {
            if let Some(g) = &self.gallery {
                g.redraw();
            }
        }

        /// Llega una miniatura (y la duración del video) del hilo de fondo.
        fn add_thumb(&mut self, gen: u64, idx: usize, bmp: HBITMAP, dur: Option<u32>, size: (i32, i32)) {
            let Some(g) = self.gallery.as_mut() else {
                unsafe {
                    let _ = DeleteObject(bmp.into());
                }
                return;
            };
            match g.items.get_mut(idx) {
                Some(it) if gen == g.gen && !bmp.is_invalid() => {
                    // reemplaza la anterior (si era de otro tamaño)
                    if let Some(old) = it.thumb.replace(bmp) {
                        unsafe {
                            let _ = DeleteObject(old.into());
                        }
                    }
                    it.tsize = size;
                    it.asked = false;
                    it.dur = dur.or(it.dur);
                }
                Some(it) if gen == g.gen => {
                    it.dur = dur.or(it.dur);
                    return;
                }
                _ => unsafe {
                    let _ = DeleteObject(bmp.into());
                    return;
                },
            }
            g.evict_thumbs();
            g.redraw();
        }

        fn paint_gallery(&mut self, dc: HDC, w: i32, h: i32) {
            let status = self.status();
            let rec = self.manual.as_ref().map(|m| logic::fmt_elapsed(m.t0.elapsed().as_secs()));
            let live = self.manual.is_some() || (self.rec.child.is_some() && self.rec.good);
            let keys = (hk(self.cfg.clip), hk(self.cfg.rec), hk(self.cfg.shot));
            let secs = self.cfg.secs;
            let Some(g) = self.gallery.as_mut() else { return };
            unsafe { g.paint(dc, w, h, &status, rec, live, keys, secs) };
        }
    }

    // ---- ventana "Mis clips": dibujada a mano (estilo de las apps de clips), sin controles ----

    struct Fonts {
        brand: HFONT,
        h1: HFONT,
        body: HFONT,
        bold: HFONT,
        small: HFONT,
        badge: HFONT,
        icon: HFONT,
        icon_big: HFONT,
    }

    struct Item {
        path: PathBuf,
        name: String,
        /// 0 clip, 1 grabación, 2 captura, 3 otro video
        kind: u8,
        title: String,
        size: u64,
        dur: Option<u32>,
        thumb: Option<HBITMAP>,
        /// tamaño de la miniatura (se pide del tamaño de la tarjeta: se dibuja sin escalar)
        tsize: (i32, i32),
        asked: bool,
    }

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Hit {
        None,
        Nav(u8),
        Card(usize),
        Play(usize),
        Act(usize, usize),
        /// 0 grabar, 1 guardar clip, 2 carpeta, 3 ajustes, 4 juntar, 5 cancelar juntar, 6 versión nueva, 7 fondo de la barra
        Btn(u8),
        Bar,
    }

    enum Outside {
        Menu,
    }

    struct Gallery {
        hwnd: HWND,
        logo: HICON,
        fonts: Fonts,
        items: Vec<Item>,
        /// índices de `items` de la sección elegida
        view: Vec<usize>,
        nav: u8,
        counts: [usize; 4],
        total: u64,
        sel: Option<usize>,
        hover: Hit,
        scroll: i32,
        content_h: i32,
        page_h: i32,
        cols: usize,
        hits: Vec<(RECT, Hit)>,
        visible: Vec<usize>,
        gen: u64,
        tx: Option<std::sync::mpsc::Sender<ThumbReq>>,
        drag: Option<i32>,
        tracking: bool,
        /// foco del teclado (Tab): sección, botón o las tarjetas (Card(usize::MAX))
        focus: Option<Hit>,
        /// destino del desplazamiento suave
        target: Option<i32>,
        /// nombres de archivo marcados como favoritos
        favs: std::collections::HashSet<String>,
        /// cuadro de texto para renombrar (control, tarjeta)
        edit: Option<(HWND, usize)>,
        edit_brush: HBRUSH,
        /// clips marcados para juntar, en orden
        joins: Vec<PathBuf>,
        /// versión nueva para bajar (botón verde de la izquierda)
        update: Option<u32>,
    }

    const NAV: [(&str, u16); 5] = [("Todo", 0xE8A9), ("Clips", 0xE714), ("Grabaciones", 0xE7C8), ("Capturas", 0xEB9F), ("Favoritos", 0xE734)];
    /// pedido de miniatura: (generación, índice, archivo, ancho, alto) — se hace del tamaño exacto de la tarjeta
    type ThumbReq = (u64, usize, PathBuf, i32, i32);
    /// miniaturas en memoria como máximo (las que no se ven se sueltan)
    const THUMB_CAP: usize = 36;

    const fn rgb(r: u32, g: u32, b: u32) -> u32 {
        r | (g << 8) | (b << 16)
    }
    const BG_COLOR: u32 = rgb(12, 12, 14);
    const SIDE: u32 = rgb(17, 17, 20);
    const CARD: u32 = rgb(24, 24, 28);
    const CARD_HOVER: u32 = rgb(33, 33, 38);
    const NAV_HOVER: u32 = rgb(27, 27, 31);
    const NAV_SEL: u32 = rgb(34, 34, 39);
    const TEXT: u32 = rgb(247, 247, 250);
    const MUTED: u32 = rgb(156, 156, 159);
    const ORANGE: u32 = rgb(255, 122, 32);
    const PINK: u32 = rgb(255, 92, 64);
    const RED: u32 = rgb(240, 52, 72);
    const BLUE: u32 = rgb(64, 150, 255);
    const GREEN: u32 = rgb(46, 214, 120);
    const THUMB_BG: u32 = rgb(20, 20, 23);
    const WM_THUMB: u32 = WM_APP + 10;

    impl Gallery {
        /// Para el menú de una tarjeta: (¿es video?, ¿favorito?, ¿marcado para juntar?, cuántos marcados)
        fn menu_info(&self, i: usize) -> (bool, bool, bool, usize) {
            let it = &self.items[i];
            let joins = self.joins.iter().filter(|p| self.items.iter().any(|x| &x.path == *p)).count();
            (it.kind != 2, self.favs.contains(&it.name), self.joins.contains(&it.path), joins)
        }

        fn redraw(&self) {
            unsafe {
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            }
        }

        fn rebuild_view(&mut self) {
            self.view = (0..self.items.len())
                .filter(|&i| match self.nav {
                    0 => true,
                    1 => matches!(self.items[i].kind, 0 | 3),
                    4 => self.favs.contains(&self.items[i].name),
                    n => self.items[i].kind == n - 1,
                })
                .collect();
        }

        fn set_nav(&mut self, n: u8) {
            if n != self.nav {
                self.nav = n;
                self.scroll = 0;
                self.rebuild_view();
            }
            self.redraw();
        }

        fn free_thumbs(&mut self) {
            for it in &mut self.items {
                if let Some(b) = it.thumb.take() {
                    unsafe {
                        let _ = DeleteObject(b.into());
                    }
                }
                it.asked = false;
            }
        }

        /// Si hay demasiadas miniaturas en memoria, se sueltan las que no están a la vista.
        fn evict_thumbs(&mut self) {
            let mut have: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].thumb.is_some()).collect();
            if have.len() <= THUMB_CAP {
                return;
            }
            let center = self.visible.first().copied().unwrap_or(0) as i64;
            have.retain(|i| !self.visible.contains(i));
            have.sort_by_key(|&i| std::cmp::Reverse((i as i64 - center).abs()));
            let extra = (have.len() + self.visible.len()).saturating_sub(THUMB_CAP);
            for &i in have.iter().take(extra) {
                if let Some(b) = self.items[i].thumb.take() {
                    unsafe {
                        let _ = DeleteObject(b.into());
                    }
                }
                self.items[i].asked = false;
            }
        }

        fn hit_at(&self, x: i32, y: i32) -> Hit {
            self.hits
                .iter()
                .rev()
                .find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
                .map_or(Hit::None, |h| h.1)
        }

        fn max_scroll(&self) -> i32 {
            (self.content_h - self.page_h).max(0)
        }

        /// Desplazamiento suave: se fija el destino y un timer de 15 ms se acerca (solo mientras se mueve).
        fn scroll_by(&mut self, dy: i32) {
            let base = self.target.unwrap_or(self.scroll);
            let t = (base + dy).clamp(0, self.max_scroll());
            if t != self.scroll {
                self.target = Some(t);
                unsafe {
                    SetTimer(Some(self.hwnd), 2, 15, None);
                }
            }
        }

        fn scroll_step(&mut self) {
            let Some(t) = self.target else {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), 2);
                }
                return;
            };
            let d = t - self.scroll;
            if d.abs() <= 2 {
                self.scroll = t;
                self.target = None;
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), 2);
                }
            } else {
                self.scroll += (d as f32 * 0.35) as i32 + d.signum();
            }
            self.redraw();
        }

        /// Arrastrar la barra de desplazamiento.
        fn drag_to(&mut self, y: i32) {
            let Some(bar) = self.hits.iter().find(|h| h.1 == Hit::Bar).map(|h| h.0) else { return };
            let track = (bar.bottom - bar.top).max(1);
            let f = ((y - bar.top) as f32 / track as f32).clamp(0.0, 1.0);
            let s = (f * self.max_scroll() as f32) as i32;
            if s != self.scroll {
                self.scroll = s;
                self.redraw();
            }
        }

        /// Asegura que la tarjeta elegida se vea (al moverse con las flechas).
        fn reveal_sel(&mut self) {
            let Some(sel) = self.sel else { return };
            let Some(pos) = self.view.iter().position(|&i| i == sel) else { return };
            let Some(card) = self.hits.iter().find(|h| matches!(h.1, Hit::Card(_))).map(|h| h.0) else { return };
            let ch = card.bottom - card.top;
            let gap = ch / 10;
            let row = (pos / self.cols.max(1)) as i32;
            let top = row * (ch + gap);
            if top < self.scroll {
                self.scroll = top;
            } else if top + ch > self.scroll + self.page_h {
                self.scroll = (top + ch - self.page_h).min(self.max_scroll());
            }
        }

        /// Orden del Tab: secciones, botones de la barra, tarjetas.
        fn tab(&mut self, back: bool) {
            let order: Vec<Hit> = (0..NAV.len() as u8).map(Hit::Nav).chain([Hit::Btn(0), Hit::Btn(1), Hit::Btn(2), Hit::Btn(3), Hit::Card(usize::MAX)]).collect();
            let cur = self.focus.and_then(|f| order.iter().position(|&h| h == f));
            let n = order.len();
            let next = match (cur, back) {
                (None, false) => 0,
                (None, true) => n - 1,
                (Some(i), false) => (i + 1) % n,
                (Some(i), true) => (i + n - 1) % n,
            };
            self.focus = Some(order[next]);
            if order[next] == Hit::Card(usize::MAX) && self.sel.is_none() {
                self.sel = self.view.first().copied();
            }
            self.redraw();
        }

        fn key(&mut self, vk: u16) -> Option<usize> {
            let n = self.view.len();
            let pos = self.sel.and_then(|s| self.view.iter().position(|&i| i == s));
            let cols = self.cols.max(1);
            let next = match (vk, pos) {
                (_, _) if n == 0 => None,
                (0x25, Some(p)) => Some(p.saturating_sub(1)),
                (0x27, Some(p)) => Some((p + 1).min(n - 1)),
                (0x26, Some(p)) => Some(p.saturating_sub(cols)),
                (0x28, Some(p)) => Some((p + cols).min(n - 1)),
                (0x24, _) => Some(0),
                (0x23, _) => Some(n - 1),
                (0x25..=0x28, None) => Some(0),
                _ => None,
            };
            if let Some(p) = next {
                self.sel = Some(self.view[p]);
                self.reveal_sel();
                self.redraw();
                return None;
            }
            match vk {
                0x0D => Some(201),
                0x2E => Some(205),
                0x71 => Some(209),
                0x74 => Some(207),
                0x21 => {
                    self.scroll_by(-self.page_h * 9 / 10);
                    None
                }
                0x22 => {
                    self.scroll_by(self.page_h * 9 / 10);
                    None
                }
                _ => None,
            }
        }

        #[allow(clippy::too_many_arguments)]
        unsafe fn paint(&mut self, dc: HDC, w: i32, h: i32, status: &str, rec: Option<String>, live: bool, keys: (String, String, String), secs: u32) {
            let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(self.hwnd).max(96) as i32;
            let s = |v: i32| v * dpi / 96;
            self.hits.clear();
            SetBkMode(dc, TRANSPARENT);
            fill(dc, rc(0, 0, w, h), BG_COLOR);

            // ---------- barra lateral ----------
            let sw = s(224);
            fill(dc, rc(0, 0, sw, h), SIDE);
            fill(dc, rc(sw - 1, 0, sw, h), rgb(30, 30, 35));
            let _ = DrawIconEx(dc, s(20), s(20), self.logo, s(40), s(40), 0, None, DI_NORMAL);
            text(dc, "Jugadón", rc(s(68), s(16), sw, s(48)), self.fonts.brand, TEXT, DT_SINGLELINE | DT_VCENTER);
            text(dc, "tus mejores jugadas", rc(s(69), s(44), sw, s(62)), self.fonts.small, MUTED, DT_SINGLELINE | DT_VCENTER);
            let mut y = s(88);
            text(dc, "BIBLIOTECA", rc(s(24), y, sw, y + s(20)), self.fonts.badge, MUTED, DT_SINGLELINE | DT_VCENTER);
            y += s(26);
            for (i, (name, glyph)) in NAV.iter().enumerate() {
                let r = rc(s(12), y, sw - s(12), y + s(38));
                let selected = self.nav == i as u8;
                let hover = self.hover == Hit::Nav(i as u8);
                if selected {
                    round(dc, r, s(10), NAV_SEL);
                    round(dc, rc(r.left, r.top + s(10), r.left + s(3), r.bottom - s(10)), s(3), ORANGE);
                } else if hover {
                    round(dc, r, s(10), NAV_HOVER);
                }
                let c = if selected { TEXT } else { rgb(205, 205, 208) };
                glyph_at(dc, *glyph, rc(r.left + s(14), r.top, r.left + s(40), r.bottom), self.fonts.icon, if selected { ORANGE } else { c });
                text(dc, name, rc(r.left + s(46), r.top, r.right, r.bottom), if selected { self.fonts.bold } else { self.fonts.body }, c, DT_SINGLELINE | DT_VCENTER);
                let n = match i {
                    0 => self.items.len(),
                    1 => self.counts[0] + self.counts[3],
                    4 => self.items.iter().filter(|it| self.favs.contains(&it.name)).count(),
                    _ => self.counts[i - 1],
                };
                let ns = n.to_string();
                let pw = s(14) + ns.len() as i32 * s(7);
                let pr = rc(r.right - s(12) - pw, r.top + s(10), r.right - s(12), r.bottom - s(10));
                round(dc, pr, s(10), if selected { rgb(73, 73, 76) } else { rgb(43, 43, 46) });
                text(dc, &ns, pr, self.fonts.small, if selected { TEXT } else { MUTED }, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                self.hits.push((r, Hit::Nav(i as u8)));
                y += s(42);
            }

            // botones grandes y estado, desde abajo
            let bx = (s(16), sw - s(16));
            let mut by = h - s(16);
            // tarjeta de estado
            let card = rc(bx.0, by - s(74), bx.1, by);
            round(dc, card, s(12), rgb(33, 33, 36));
            let dot = if rec.is_some() {
                RED
            } else if live {
                GREEN
            } else {
                rgb(125, 125, 128)
            };
            let dr = rc(card.left + s(14), card.top + s(17), card.left + s(24), card.top + s(27));
            ellipse(dc, dr, dot);
            let head = if rec.is_some() {
                "Grabando"
            } else if live {
                "Clips activos"
            } else {
                "En espera"
            };
            text(dc, head, rc(card.left + s(32), card.top + s(10), card.right - s(8), card.top + s(34)), self.fonts.bold, TEXT, DT_SINGLELINE | DT_VCENTER);
            let st = status.trim_start_matches("● ");
            text(dc, st, rc(card.left + s(14), card.top + s(34), card.right - s(10), card.bottom - s(6)), self.fonts.small, MUTED, DT_WORDBREAK | DT_END_ELLIPSIS);
            by = card.top - s(12);
            if let Some(v) = self.update {
                let r = rc(bx.0, by - s(44), bx.1, by);
                let on = self.hover == Hit::Btn(6);
                grad_round(dc, r, s(12), if on { rgb(110, 240, 160) } else { GREEN }, rgb(16, 180, 150), false);
                glyph_at(dc, 0xE896, rc(r.left + s(10), r.top, r.left + s(36), r.bottom), self.fonts.icon, rgb(6, 34, 18));
                text(dc, "Bajar versión nueva", rc(r.left + s(40), r.top + s(4), r.right - s(6), r.top + s(24)), self.fonts.bold, rgb(6, 34, 18), DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
                text(dc, &format!("v1.0.{v}"), rc(r.left + s(40), r.top + s(22), r.right - s(6), r.bottom - s(4)), self.fonts.small, rgb(10, 60, 34), DT_SINGLELINE | DT_VCENTER);
                self.hits.push((r, Hit::Btn(6)));
                by = r.top - s(10);
            }
            // ajustes y carpeta
            for (b, glyph, label) in [(3u8, 0xE713u16, "Ajustes"), (2, 0xE838, "Abrir carpeta")] {
                let r = rc(bx.0, by - s(38), bx.1, by);
                let hover = self.hover == Hit::Btn(b);
                if hover {
                    round(dc, r, s(10), NAV_HOVER);
                }
                glyph_at(dc, glyph, rc(r.left + s(12), r.top, r.left + s(38), r.bottom), self.fonts.icon, rgb(205, 205, 208));
                text(dc, label, rc(r.left + s(44), r.top, r.right, r.bottom), self.fonts.body, rgb(224, 224, 227), DT_SINGLELINE | DT_VCENTER);
                self.hits.push((r, Hit::Btn(b)));
                by = r.top - s(2);
            }
            by -= s(10);
            // guardar clip
            let r = rc(bx.0, by - s(44), bx.1, by);
            let hover = self.hover == Hit::Btn(1);
            round(dc, r, s(12), if hover { rgb(63, 63, 66) } else { rgb(48, 48, 51) });
            glyph_at(dc, 0xE74E, rc(r.left + s(14), r.top, r.left + s(40), r.bottom), self.fonts.icon, ORANGE);
            text(dc, "Guardar clip", rc(r.left + s(46), r.top, r.right, r.bottom), self.fonts.bold, TEXT, DT_SINGLELINE | DT_VCENTER);
            text(dc, &keys.0, rc(r.left, r.top, r.right - s(14), r.bottom), self.fonts.small, MUTED, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
            self.hits.push((r, Hit::Btn(1)));
            by = r.top - s(10);
            // grabar / frenar (el botón principal)
            let r = rc(bx.0, by - s(50), bx.1, by);
            let hover = self.hover == Hit::Btn(0);
            match &rec {
                Some(t) => {
                    round(dc, r, s(14), if hover { rgb(255, 80, 96) } else { RED });
                    let sq = rc(r.left + s(18), r.top + s(19), r.left + s(30), r.top + s(31));
                    round(dc, sq, s(3), TEXT);
                    text(dc, "Frenar", rc(r.left + s(42), r.top, r.right, r.bottom), self.fonts.bold, TEXT, DT_SINGLELINE | DT_VCENTER);
                    text(dc, t, rc(r.left, r.top, r.right - s(16), r.bottom), self.fonts.bold, TEXT, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
                }
                None => {
                    round(dc, r, s(14), if hover { rgb(255, 144, 70) } else { ORANGE });
                    ellipse(dc, rc(r.left + s(18), r.top + s(18), r.left + s(32), r.top + s(32)), TEXT);
                    text(dc, "Grabar pantalla", rc(r.left + s(42), r.top, r.right, r.bottom), self.fonts.bold, TEXT, DT_SINGLELINE | DT_VCENTER);
                    text(dc, &keys.1, rc(r.left, r.top, r.right - s(16), r.bottom), self.fonts.small, rgb(255, 236, 226), DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
                }
            }
            self.hits.push((r, Hit::Btn(0)));

            // ---------- contenido ----------
            let cx0 = sw + s(32);
            let cx1 = w - s(28);
            // resplandor de color arriba (decoración: un degradé, sin imágenes)
            let title = NAV[self.nav as usize].0;
            let title = if self.nav == 0 { "Mis clips" } else { title };
            text(dc, title, rc(cx0, s(14), cx1, s(52)), self.fonts.h1, TEXT, DT_SINGLELINE | DT_VCENTER);
            let tw_ = text_w(dc, self.fonts.h1, title);
            let _ = tw_;
            // datos y atajos en "pastillas"
            let n = self.view.len();
            let size: u64 = self.view.iter().map(|&i| self.items[i].size).sum();
            let mut x = cx0;
            let cy = (s(58), s(84));
            let info = format!("{n} {}  ·  {}", if n == 1 { "archivo" } else { "archivos" }, logic::fmt_size(size));
            x = chip(dc, x, cy, None, &info, self.fonts.small, s) + s(14);
            for (k, what) in [(&keys.0, format!("guardar últimos {}", logic::fmt_elapsed(secs as u64))), (&keys.1, "grabar".to_string()), (&keys.2, "captura".to_string())] {
                if x + s(150) > cx1 {
                    break;
                }
                x = chip(dc, x, cy, Some(k), &what, self.fonts.small, s) + s(8);
            }
            let top = s(100);
            let page = rc(cx0, top, cx1, h);
            self.page_h = (h - top).max(1);
            self.visible.clear();

            if self.view.is_empty() {
                self.content_h = 0;
                self.scroll = 0;
                let mid = (cx0 + cx1) / 2;
                let cy = top + (h - top) / 2 - s(90);
                ellipse(dc, rc(mid - s(86), cy - s(38), mid + s(86), cy + s(134)), rgb(28, 28, 31));
                ellipse(dc, rc(mid - s(68), cy - s(20), mid + s(68), cy + s(116)), rgb(38, 38, 41));
                let _ = DrawIconEx(dc, mid - s(48), cy, self.logo, s(96), s(96), 0, None, DI_NORMAL);
                let (t1, t2) = match self.nav {
                    4 => ("Todavía no hay favoritos", "Pasá el mouse por un clip y tocá la estrellita (o clic derecho → Favorito).".to_string()),
                    3 => ("Todavía no hay capturas", format!("Apretá {} y se guarda una foto de la pantalla.", keys.2)),
                    2 => ("Todavía no hay grabaciones", format!("Apretá {} para empezar a grabar y de nuevo {} para guardar.", keys.1, keys.1)),
                    _ => ("Todavía no hay clips", format!("Abrí un juego y apretá {}: se guardan los últimos {}.", keys.0, logic::fmt_elapsed(secs as u64))),
                };
                text(dc, t1, rc(cx0, cy + s(108), cx1, cy + s(140)), self.fonts.h1, TEXT, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
                text(dc, &t2, rc(cx0, cy + s(142), cx1, cy + s(166)), self.fonts.body, MUTED, DT_SINGLELINE | DT_CENTER | DT_VCENTER | DT_END_ELLIPSIS);
                self.paint_focus(dc, s(2));
                return;
            }

            // grilla de tarjetas
            let gap = s(18);
            let avail = (cx1 - cx0).max(s(200));
            let cols = ((avail + gap) / (s(220) + gap)).max(1) as usize;
            self.cols = cols;
            let cw = (avail - gap * (cols as i32 - 1)) / cols as i32;
            let th = cw * 9 / 16;
            let ch = th + s(60);
            let rows = n.div_ceil(cols) as i32;
            self.content_h = rows * (ch + gap) + s(8);
            self.scroll = self.scroll.clamp(0, self.max_scroll());
            let rgn = CreateRectRgn(page.left - s(8), page.top, page.right + s(8), page.bottom);
            SelectClipRgn(dc, Some(rgn));
            let mut asks = Vec::new();
            for (pos, &idx) in self.view.iter().enumerate() {
                let (row, col) = ((pos / cols) as i32, (pos % cols) as i32);
                let x = cx0 + col * (cw + gap);
                let y = top + row * (ch + gap) - self.scroll;
                if y + ch < top - s(8) || y > h {
                    continue;
                }
                self.visible.push(idx);
                let it = &self.items[idx];
                // sin miniatura, o de otro tamaño (se agrandó la ventana): se pide de nuevo, del tamaño justo
                if !it.asked && (it.thumb.is_none() || it.tsize != (cw, th)) {
                    asks.push(idx);
                }
                let r = rc(x, y, x + cw, y + ch);
                let hovered = matches!(self.hover, Hit::Card(i) | Hit::Play(i) | Hit::Act(i, _) if i == idx);
                let selected = self.sel == Some(idx);
                let join_no = self.joins.iter().position(|p| *p == it.path);
                // tarjeta
                // sombra suave
                shade_round(dc, rc(r.left + s(2), r.top + s(5), r.right + s(2), r.bottom + s(6)), s(16), 0, 90);
                if selected {
                    round(dc, rc(r.left - s(2), r.top - s(2), r.right + s(2), r.bottom + s(2)), s(16), ORANGE);
                } else if join_no.is_some() {
                    round(dc, rc(r.left - s(2), r.top - s(2), r.right + s(2), r.bottom + s(2)), s(16), GREEN);
                }
                round(dc, r, s(14), if hovered || selected { CARD_HOVER } else { CARD });
                // miniatura (esquinas de arriba redondeadas)
                let tr = rc(r.left, r.top, r.right, r.top + th);
                let clip = CreateRoundRectRgn(tr.left, tr.top, tr.right + 1, tr.bottom + s(16), s(14), s(14));
                let page_rgn = CreateRectRgn(page.left - s(8), page.top, page.right + s(8), page.bottom);
                let _ = CombineRgn(Some(clip), Some(clip), Some(page_rgn), RGN_AND);
                let tr_rgn = CreateRectRgn(tr.left, tr.top, tr.right, tr.bottom);
                let _ = CombineRgn(Some(clip), Some(clip), Some(tr_rgn), RGN_AND);
                let _ = DeleteObject(tr_rgn.into());
                SelectClipRgn(dc, Some(clip));
                fill(dc, tr, THUMB_BG);
                match it.thumb {
                    Some(b) => blit_cover(dc, b, it.tsize, tr),
                    None => {
                        let sz = th / 3;
                        let _ = DrawIconEx(dc, (tr.left + tr.right - sz) / 2, (tr.top + tr.bottom - sz) / 2, self.logo, sz, sz, 0, None, DI_NORMAL);
                    }
                }
                if hovered {
                    shade(dc, tr, 0, 70);
                }
                // etiqueta del tipo
                let (label, color) = match it.kind {
                    0 => ("CLIP", ORANGE),
                    1 => ("GRABACIÓN", PINK),
                    2 => ("CAPTURA", BLUE),
                    _ => ("VIDEO", rgb(140, 120, 255)),
                };
                // etiqueta sobria: pastilla oscura translúcida con un punto de color
                let lw = s(26) + label.chars().count() as i32 * s(7);
                let lr = rc(tr.left + s(10), tr.top + s(10), tr.left + s(10) + lw, tr.top + s(30));
                shade_round(dc, lr, s(10), 0, 170);
                ellipse(dc, rc(lr.left + s(8), lr.top + s(7), lr.left + s(14), lr.top + s(13)), color);
                text(dc, label, rc(lr.left + s(12), lr.top, lr.right, lr.bottom), self.fonts.badge, TEXT, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                let fav = self.favs.contains(&it.name);
                if fav {
                    let fr = rc(lr.right + s(6), lr.top, lr.right + s(28), lr.bottom);
                    round(dc, fr, s(10), rgb(255, 196, 0));
                    glyph_at(dc, 0xE735, fr, self.fonts.small, rgb(60, 40, 0));
                }
                if let Some(k) = join_no {
                    // número de orden para juntar
                    let x0 = lr.right + if fav { s(34) } else { s(6) };
                    let jr = rc(x0, lr.top, x0 + s(20), lr.bottom);
                    ellipse(dc, jr, GREEN);
                    text(dc, &(k + 1).to_string(), jr, self.fonts.badge, rgb(6, 40, 20), DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                }
                // duración
                if let Some(d) = it.dur {
                    let ds = logic::fmt_elapsed(d as u64);
                    let dw = s(14) + ds.len() as i32 * s(7);
                    let dr = rc(tr.right - s(10) - dw, tr.bottom - s(32), tr.right - s(10), tr.bottom - s(10));
                    shade_round(dc, dr, s(10), 0, 190);
                    text(dc, &ds, dr, self.fonts.bold, TEXT, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                }
                SelectClipRgn(dc, Some(rgn));
                let _ = DeleteObject(clip.into());
                let _ = DeleteObject(page_rgn.into());
                self.hits.push((r, Hit::Card(idx)));
                if hovered {
                    // botón ▶ en el medio y acciones rápidas arriba a la derecha
                    let pr = s(26);
                    let (mx, my) = ((tr.left + tr.right) / 2, (tr.top + tr.bottom) / 2);
                    let play = rc(mx - pr, my - pr, mx + pr, my + pr);
                    let ph = self.hover == Hit::Play(idx);
                    ellipse(dc, play, if ph { rgb(255, 144, 70) } else { ORANGE });
                    glyph_at(dc, 0xE768, rc(play.left + s(3), play.top, play.right + s(3), play.bottom), self.fonts.icon_big, TEXT);
                    self.hits.push((play, Hit::Play(idx)));
                    let mut ax = tr.right - s(10);
                    let star = if self.favs.contains(&self.items[idx].name) { 0xE735u16 } else { 0xE734 };
                    for (cmd, glyph) in [(205usize, 0xE74Du16), (203, 0xE8C8), (202, 0xE71B), (208, star)] {
                        let ar = rc(ax - s(32), tr.top + s(8), ax, tr.top + s(40));
                        let ah = self.hover == Hit::Act(idx, cmd);
                        if ah {
                            ellipse(dc, ar, match cmd {
                                205 => RED,
                                208 => rgb(230, 170, 0),
                                _ => ORANGE,
                            });
                        } else {
                            shade_round(dc, ar, s(32), 0, 170);
                        }
                        glyph_at(dc, glyph, ar, self.fonts.icon, TEXT);
                        if ah {
                            // etiqueta de qué hace el botón
                            let label = match cmd {
                                205 => "Eliminar",
                                203 => "Copiar",
                                208 => "Favorito",
                                _ => "Copiar link",
                            };
                            let lw = text_w(dc, self.fonts.small, label) + s(16);
                            let lr = rc((ar.left + ar.right - lw) / 2, ar.bottom + s(6), (ar.left + ar.right + lw) / 2, ar.bottom + s(28));
                            shade_round(dc, lr, s(10), 0, 210);
                            text(dc, label, lr, self.fonts.small, TEXT, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                        }
                        self.hits.push((ar, Hit::Act(idx, cmd)));
                        ax -= s(38);
                    }
                }
                // textos
                text(dc, &it.title, rc(r.left + s(14), tr.bottom + s(8), r.right - s(12), tr.bottom + s(30)), self.fonts.bold, TEXT, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
                let info = match it.dur {
                    Some(d) => format!("{}  ·  {}", logic::fmt_elapsed(d as u64), logic::fmt_size(it.size)),
                    None => logic::fmt_size(it.size),
                };
                text(dc, &info, rc(r.left + s(14), tr.bottom + s(30), r.right - s(12), tr.bottom + s(50)), self.fonts.small, MUTED, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            }
            SelectClipRgn(dc, None);
            let _ = DeleteObject(rgn.into());
            // barra flotante para juntar clips (solo si hay alguno marcado)
            let nj = self.joins.iter().filter(|p| self.items.iter().any(|i| &i.path == *p)).count();
            if nj > 0 {
                let bw = s(500).min(page.right - page.left - s(16));
                let cx = (page.left + page.right) / 2;
                let bar = rc(cx - bw / 2, h - s(78), cx + bw / 2, h - s(20));
                shade_round(dc, rc(bar.left + s(2), bar.top + s(6), bar.right + s(2), bar.bottom + s(8)), s(22), 0, 120);
                round(dc, bar, s(20), rgb(42, 42, 45));
                self.hits.push((bar, Hit::Btn(7)));
                let nr = rc(bar.left + s(14), bar.top + s(17), bar.left + s(38), bar.top + s(41));
                ellipse(dc, nr, GREEN);
                text(dc, &nj.to_string(), nr, self.fonts.bold, rgb(6, 40, 20), DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                let jr = rc(bar.right - s(204), bar.top + s(11), bar.right - s(102), bar.bottom - s(11));
                let cr = rc(bar.right - s(94), bar.top + s(11), bar.right - s(12), bar.bottom - s(11));
                let msg = if nj < 2 { "Marcá otro clip (Ctrl + clic)".to_string() } else { format!("{nj} clips para juntar, en orden") };
                text(dc, &msg, rc(nr.right + s(10), bar.top, jr.left - s(8), bar.bottom), self.fonts.bold, TEXT, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
                if nj >= 2 {
                    let on = self.hover == Hit::Btn(4);
                    grad_round(dc, jr, s(18), if on { rgb(110, 240, 160) } else { GREEN }, rgb(16, 180, 150), false);
                } else {
                    round(dc, jr, s(18), rgb(61, 61, 64));
                }
                text(dc, "Juntar", jr, self.fonts.bold, if nj >= 2 { rgb(6, 34, 18) } else { MUTED }, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                round(dc, cr, s(18), if self.hover == Hit::Btn(5) { rgb(77, 77, 80) } else { rgb(57, 57, 60) });
                text(dc, "Cancelar", cr, self.fonts.small, TEXT, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                self.hits.push((jr, Hit::Btn(4)));
                self.hits.push((cr, Hit::Btn(5)));
            }
            // barra de desplazamiento finita
            if self.max_scroll() > 0 {
                let track = rc(w - s(12), top + s(4), w - s(6), h - s(8));
                let th_ = ((track.bottom - track.top) * self.page_h / self.content_h.max(1)).max(s(40));
                let ty = track.top + (track.bottom - track.top - th_) * self.scroll / self.max_scroll().max(1);
                let active = self.hover == Hit::Bar || self.drag.is_some();
                round(dc, rc(track.left, ty, track.right, ty + th_), s(6), if active { ORANGE } else { rgb(75, 75, 78) });
                self.hits.push((rc(track.left - s(6), track.top, track.right + s(4), track.bottom), Hit::Bar));
            }
            self.paint_focus(dc, s(2));
            for idx in asks {
                self.items[idx].asked = true;
                if let Some(tx) = &self.tx {
                    let _ = tx.send((self.gen, idx, self.items[idx].path.clone(), cw, th));
                }
            }
        }
    }

    impl Gallery {
        /// Anillo blanco alrededor de lo que tiene el foco del teclado (Tab).
        unsafe fn paint_focus(&self, dc: HDC, wd: i32) {
            let Some(f) = self.focus else { return };
            let target = if f == Hit::Card(usize::MAX) { self.sel.map(Hit::Card) } else { Some(f) };
            let Some(r) = target.and_then(|t| self.hits.iter().find(|h| h.1 == t)).map(|h| h.0) else { return };
            let pen = CreatePen(PS_SOLID, wd, COLORREF(TEXT));
            let op = SelectObject(dc, pen.into());
            let ob = SelectObject(dc, GetStockObject(NULL_BRUSH));
            let _ = RoundRect(dc, r.left - wd * 2, r.top - wd * 2, r.right + wd * 2, r.bottom + wd * 2, wd * 7, wd * 7);
            SelectObject(dc, ob);
            SelectObject(dc, op);
            let _ = DeleteObject(pen.into());
        }
    }

    impl Drop for Gallery {
        fn drop(&mut self) {
            self.tx = None;
            self.free_thumbs();
            unsafe {
                let _ = DeleteObject(self.edit_brush.into());
                for f in [self.fonts.brand, self.fonts.h1, self.fonts.body, self.fonts.bold, self.fonts.small, self.fonts.badge, self.fonts.icon, self.fonts.icon_big] {
                    let _ = DeleteObject(f.into());
                }
                let _ = DestroyIcon(self.logo);
            }
        }
    }

    // ---- ayudas de dibujo (GDI) ----

    fn rc(l: i32, t: i32, r: i32, b: i32) -> RECT {
        RECT { left: l, top: t, right: r, bottom: b }
    }

    unsafe fn fill(dc: HDC, r: RECT, c: u32) {
        let b = CreateSolidBrush(COLORREF(c));
        FillRect(dc, &r, b);
        let _ = DeleteObject(b.into());
    }

    unsafe fn round(dc: HDC, r: RECT, rad: i32, c: u32) {
        let b = CreateSolidBrush(COLORREF(c));
        let p = CreatePen(PS_SOLID, 1, COLORREF(c));
        let ob = SelectObject(dc, b.into());
        let op = SelectObject(dc, p.into());
        let _ = RoundRect(dc, r.left, r.top, r.right, r.bottom, rad, rad);
        SelectObject(dc, ob);
        SelectObject(dc, op);
        let _ = DeleteObject(b.into());
        let _ = DeleteObject(p.into());
    }

    unsafe fn ellipse(dc: HDC, r: RECT, c: u32) {
        let b = CreateSolidBrush(COLORREF(c));
        let p = CreatePen(PS_SOLID, 1, COLORREF(c));
        let ob = SelectObject(dc, b.into());
        let op = SelectObject(dc, p.into());
        let _ = Ellipse(dc, r.left, r.top, r.right, r.bottom);
        SelectObject(dc, ob);
        SelectObject(dc, op);
        let _ = DeleteObject(b.into());
        let _ = DeleteObject(p.into());
    }

    fn vertex(x: i32, y: i32, c: u32) -> TRIVERTEX {
        TRIVERTEX { x, y, Red: ((c & 0xFF) << 8) as u16, Green: (c & 0xFF00) as u16, Blue: ((c >> 8) & 0xFF00) as u16, Alpha: 0 }
    }

    /// Degradé (horizontal, o vertical) recortado a una forma.
    unsafe fn grad_in(dc: HDC, r: RECT, rgn: HRGN, a: u32, b: u32, vertical: bool) {
        let mut old = CreateRectRgn(0, 0, 0, 0);
        let had = GetClipRgn(dc, old) == 1;
        let _ = ExtSelectClipRgn(dc, Some(rgn), RGN_AND);
        let v = [vertex(r.left, r.top, a), vertex(r.right, r.bottom, b)];
        let gr = GRADIENT_RECT { UpperLeft: 0, LowerRight: 1 };
        let _ = GradientFill(dc, &v, &gr as *const _ as _, 1, if vertical { GRADIENT_FILL_RECT_V } else { GRADIENT_FILL_RECT_H });
        SelectClipRgn(dc, if had { Some(old) } else { None });
        let _ = DeleteObject(rgn.into());
        let _ = DeleteObject(std::mem::take(&mut old).into());
    }

    unsafe fn grad_round(dc: HDC, r: RECT, rad: i32, a: u32, b: u32, vertical: bool) {
        grad_in(dc, r, CreateRoundRectRgn(r.left, r.top, r.right + 1, r.bottom + 1, rad, rad), a, b, vertical);
    }


    /// Capa semitransparente de un color (para oscurecer la miniatura o los fondos de las etiquetas).
    unsafe fn shade(dc: HDC, r: RECT, c: u32, alpha: u8) {
        let src = CreateCompatibleDC(Some(dc));
        let bmp = CreateCompatibleBitmap(dc, 1, 1);
        let old = SelectObject(src, bmp.into());
        SetPixel(src, 0, 0, COLORREF(c));
        let bf = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: 0 };
        let _ = AlphaBlend(dc, r.left, r.top, r.right - r.left, r.bottom - r.top, src, 0, 0, 1, 1, bf);
        SelectObject(src, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(src);
    }

    unsafe fn shade_round(dc: HDC, r: RECT, rad: i32, c: u32, alpha: u8) {
        let mut old = CreateRectRgn(0, 0, 0, 0);
        let had = GetClipRgn(dc, old) == 1;
        let rgn = CreateRoundRectRgn(r.left, r.top, r.right + 1, r.bottom + 1, rad, rad);
        let _ = ExtSelectClipRgn(dc, Some(rgn), RGN_AND);
        shade(dc, r, c, alpha);
        SelectClipRgn(dc, if had { Some(old) } else { None });
        let _ = DeleteObject(rgn.into());
        let _ = DeleteObject(std::mem::take(&mut old).into());
    }

    unsafe fn text(dc: HDC, s: &str, r: RECT, font: HFONT, c: u32, flags: DRAW_TEXT_FORMAT) {
        let old = SelectObject(dc, font.into());
        SetTextColor(dc, COLORREF(c));
        let mut t: Vec<u16> = s.encode_utf16().collect();
        let mut r = r;
        DrawTextW(dc, &mut t, &mut r, flags | DT_NOPREFIX);
        SelectObject(dc, old);
    }

    unsafe fn text_w(dc: HDC, font: HFONT, s: &str) -> i32 {
        let old = SelectObject(dc, font.into());
        let t: Vec<u16> = s.encode_utf16().collect();
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &t, &mut sz);
        SelectObject(dc, old);
        sz.cx
    }

    /// Pastilla con texto (y una tecla dibujada como tecla, si hay). Devuelve dónde termina.
    unsafe fn chip(dc: HDC, x: i32, (t, b): (i32, i32), key: Option<&str>, label: &str, font: HFONT, s: impl Fn(i32) -> i32) -> i32 {
        let kw = key.map_or(0, |k| text_w(dc, font, k) + s(14));
        let lw = text_w(dc, font, label);
        let r = rc(x, t, x + s(10) + kw + lw + s(12), b);
        round(dc, r, s(12), rgb(33, 33, 36));
        let mut tx = r.left + s(10);
        if let Some(k) = key {
            let kr = rc(tx - s(4), t + s(4), tx - s(4) + kw, b - s(4));
            round(dc, kr, s(6), rgb(63, 63, 66));
            text(dc, k, kr, font, TEXT, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
            tx += kw + s(2);
        }
        text(dc, label, rc(tx, t, r.right, b), font, MUTED, DT_SINGLELINE | DT_VCENTER);
        r.right
    }

    /// ¿Está esta fuente instalada? (mira el archivo en C:\Windows\Fonts, una sola vez)
    fn has_font(file: &str) -> bool {
        std::env::var("WINDIR").is_ok_and(|w| Path::new(&w).join("Fonts").join(file).exists())
    }

    /// Letra moderna de Windows 11 (Segoe UI Variable); en Windows 10, Segoe UI.
    fn ui_face() -> PCWSTR {
        static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *V.get_or_init(|| has_font("SegUIVar.ttf")) {
            w!("Segoe UI Variable Text")
        } else {
            w!("Segoe UI")
        }
    }

    /// Íconos de Windows 11 (Segoe Fluent Icons, mismos códigos); en Windows 10, Segoe MDL2 Assets.
    fn icon_face() -> PCWSTR {
        static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *V.get_or_init(|| has_font("SegoeIcons.ttf")) {
            w!("Segoe Fluent Icons")
        } else {
            w!("Segoe MDL2 Assets")
        }
    }

    unsafe fn glyph_at(dc: HDC, g: u16, r: RECT, font: HFONT, c: u32) {
        let old = SelectObject(dc, font.into());
        SetTextColor(dc, COLORREF(c));
        let mut t = [g];
        let mut r = r;
        DrawTextW(dc, &mut t, &mut r, DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX);
        SelectObject(dc, old);
    }

    /// Dibuja la miniatura llenando el rectángulo (recorta lo que sobra, sin deformar).
    /// Dibuja la miniatura en el rectángulo. Si es del mismo tamaño (lo normal) es una copia directa, sin escalar.
    unsafe fn blit_cover(dc: HDC, bmp: HBITMAP, (bw, bh): (i32, i32), r: RECT) {
        let src = CreateCompatibleDC(Some(dc));
        let old = SelectObject(src, bmp.into());
        let (dw, dh) = (r.right - r.left, r.bottom - r.top);
        if (bw, bh) == (dw, dh) {
            let _ = BitBlt(dc, r.left, r.top, dw, dh, Some(src), 0, 0, SRCCOPY);
        } else {
            // mientras llega la del tamaño nuevo: escalado rápido
            let (mut sw, mut sh) = (bw, bh);
            if sw * dh > sh * dw {
                sw = sh * dw / dh.max(1);
            } else {
                sh = sw * dh / dw.max(1);
            }
            SetStretchBltMode(dc, COLORONCOLOR);
            let _ = StretchBlt(dc, r.left, r.top, dw, dh, Some(src), (bw - sw) / 2, (bh - sh) / 2, sw, sh, SRCCOPY);
        }
        SelectObject(src, old);
        let _ = DeleteDC(src);
    }

    /// Barra de título oscura (Windows 10 2004+ / 11) y del color del fondo en Windows 11.
    fn dark_title_bar(hwnd: HWND) {
        use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
        unsafe {
            let on: u32 = 1;
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &on as *const u32 as _, 4);
            let caption: u32 = SIDE;
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(35), &caption as *const u32 as _, 4);
        }
    }

    /// Miniatura del Explorador (usa su caché) ya escalada a w×h llenando la tarjeta, y la duración si es un video.
    /// El escalado de calidad se hace una sola vez acá (en el hilo de fondo), no en cada dibujo.
    unsafe fn make_thumb(path: &Path, w: i32, h: i32) -> (Option<HBITMAP>, Option<u32>) {
        use windows::core::Interface;
        let p = wide(&path.to_string_lossy());
        let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem2>(PCWSTR(p.as_ptr()), None) else { return (None, None) };
        let dur = item
            .GetUInt64(&windows::Win32::Storage::EnhancedStorage::PKEY_Media_Duration)
            .ok()
            .map(|d| ((d + 5_000_000) / 10_000_000) as u32)
            .filter(|&d| d > 0);
        let Ok(f) = item.cast::<IShellItemImageFactory>() else { return (None, dur) };
        let Ok(src) = f.GetImage(SIZE { cx: w, cy: h }, SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY) else { return (None, dur) };
        let mut bm = BITMAP::default();
        GetObjectW(src.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as _));
        let (bw, bh) = (bm.bmWidth.max(1), bm.bmHeight.abs().max(1));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(dst) = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteObject(src.into());
            return (None, dur);
        };
        let d1 = CreateCompatibleDC(None);
        let d2 = CreateCompatibleDC(None);
        let o1 = SelectObject(d1, dst.into());
        let o2 = SelectObject(d2, src.into());
        fill(d1, rc(0, 0, w, h), THUMB_BG);
        // "cover": llenar la tarjeta recortando lo que sobra, sin deformar
        let (mut sw, mut sh) = (bw, bh);
        if sw * h > sh * w {
            sw = sh * w / h.max(1);
        } else {
            sh = sw * h / w.max(1);
        }
        SetStretchBltMode(d1, HALFTONE);
        let _ = SetBrushOrgEx(d1, 0, 0, None);
        let _ = StretchBlt(d1, 0, 0, w, h, Some(d2), (bw - sw) / 2, (bh - sh) / 2, sw, sh, SRCCOPY);
        SelectObject(d1, o1);
        SelectObject(d2, o2);
        let _ = DeleteDC(d1);
        let _ = DeleteDC(d2);
        let _ = DeleteObject(src.into());
        (Some(dst), dur)
    }

    /// Hilo de miniaturas con prioridad de fondo (CPU y disco bajos). Termina cuando se cierra la ventana.
    fn start_thumb_thread(hwnd: HWND, rx: std::sync::mpsc::Receiver<ThumbReq>) {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
        let hw = hwnd.0 as isize;
        std::thread::spawn(move || unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            while let Ok((gen, idx, path, w, h)) = rx.recv() {
                let (bmp, dur) = make_thumb(&path, w, h);
                let msg = Box::new((gen, idx, bmp.map_or(0, |b| b.0 as isize), dur, (w, h)));
                let raw = Box::into_raw(msg);
                if PostMessageW(Some(HWND(hw as _)), WM_THUMB, WPARAM(0), LPARAM(raw as isize)).is_err() {
                    let m = Box::from_raw(raw);
                    if m.2 != 0 {
                        let _ = DeleteObject(HBITMAP(m.2 as _).into());
                    }
                    break;
                }
            }
            CoUninitialize();
        });
    }

    /// Submenú "Vertical" (sound = false) o "Sonido" (sound = true).
    unsafe fn tool_menu(sound: bool) -> HMENU {
        let m = CreatePopupMenu().unwrap_or_default();
        let item = |id: usize, t: &str| {
            let t = wide(t);
            let _ = AppendMenuW(m, MF_STRING, id, PCWSTR(t.as_ptr()));
        };
        if sound {
            for (i, (label, _)) in logic::SOUND_MODES.iter().enumerate() {
                item(290 + i, label);
            }
            let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
            let t = wide("(para clips grabados con el micrófono; crea una copia)");
            let _ = AppendMenuW(m, MF_STRING | MF_GRAYED, 0, PCWSTR(t.as_ptr()));
        } else {
            item(262, "Recortado al centro (llena la pantalla)");
            item(263, "Entero, con el fondo desenfocado");
        }
        m
    }

    /// El submenú de vertical o de sonido solo (botones del visor). Devuelve el comando elegido.
    unsafe fn popup_tool(hwnd: HWND, sound: bool) -> usize {
        let m = tool_menu(sound);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, None, hwnd, None);
        let _ = DestroyMenu(m);
        cmd.0 as usize
    }

    /// Menú del clic derecho sobre una tarjeta.
    unsafe fn card_menu(hwnd: HWND, video: bool, fav: bool, marked: bool, joins: usize) -> usize {
        let m = CreatePopupMenu().unwrap_or_default();
        let item = |menu: HMENU, id: usize, t: &str| {
            let t = wide(t);
            let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(t.as_ptr()));
        };
        item(m, 201, if video { "Ver y editar\tEnter" } else { "Abrir\tEnter" });
        if video {
            item(m, 272, "Abrir en el reproductor de Windows");
        }
        item(m, 202, "Copiar link para compartir (se borra en 3 días)");
        item(m, 270, "Copiar link permanente");
        item(m, 203, "Copiar el archivo (para pegar en Discord o WhatsApp)\tCtrl+C");
        item(m, 271, "Compartir con Windows…");
        item(m, 208, if fav { "Quitar de favoritos" } else { "Agregar a favoritos ⭐" });
        item(m, 209, "Renombrar…\tF2");
        if video {
            item(m, 260, "Achicar para Discord (menos de 10 MB)");
            let sub = CreatePopupMenu().unwrap_or_default();
            for (i, t) in ["Dejar los últimos 10 segundos", "Dejar los últimos 15 segundos", "Dejar los últimos 30 segundos", "Dejar el último minuto"].iter().enumerate() {
                item(sub, 250 + i, t);
            }
            let _ = AppendMenuW(sub, MF_SEPARATOR, 0, PCWSTR::null());
            for (i, t) in ["Sacar los primeros 5 segundos", "Sacar los primeros 10 segundos", "Sacar los primeros 30 segundos"].iter().enumerate() {
                item(sub, 254 + i, t);
            }
            let t = wide("Recortar (crea una copia)");
            let _ = AppendMenuW(m, MF_POPUP, sub.0 as usize, PCWSTR(t.as_ptr()));
            let t = wide("Vertical para TikTok, Reels y Shorts");
            let _ = AppendMenuW(m, MF_POPUP, tool_menu(false).0 as usize, PCWSTR(t.as_ptr()));
            let t = wide("Sonido: mi voz y el juego por separado");
            let _ = AppendMenuW(m, MF_POPUP, tool_menu(true).0 as usize, PCWSTR(t.as_ptr()));
            let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
            item(m, 280, if marked { "No juntar este clip" } else { "Marcar para juntar con otros\tCtrl+clic" });
            if joins >= 2 {
                item(m, 281, &format!("Juntar los {joins} clips marcados en un video"));
            }
            if joins >= 1 {
                item(m, 282, "Desmarcar todos");
            }
        }
        item(m, 204, "Mostrar en carpeta");
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
        item(m, 205, "Eliminar\tSupr");
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, None, hwnd, None);
        let _ = DestroyMenu(m);
        cmd.0 as usize
    }

    const WM_RENAME: u32 = WM_APP + 20;
    const WM_RENAME_DONE: u32 = WM_APP + 21;

    /// Cuadro de texto de renombrar: Enter guarda, Esc cancela.
    unsafe extern "system" fn edit_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
        if msg == WM_KEYDOWN && (wp.0 == 0x0D || wp.0 == 0x1B) {
            if let Ok(parent) = GetParent(hwnd) {
                let _ = PostMessageW(Some(parent), WM_RENAME_DONE, WPARAM((wp.0 == 0x0D) as usize), LPARAM(0));
            }
            return LRESULT(0);
        }
        if msg == WM_CHAR && (wp.0 == 0x0D || wp.0 == 0x1B) {
            return LRESULT(0); // sin el "ding" de Windows
        }
        if msg == WM_GETDLGCODE {
            return LRESULT(DLGC_WANTALLKEYS as isize);
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }

    unsafe extern "system" fn gallery_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        let pt = || ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32);
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut r = RECT::default();
                let _ = GetClientRect(hwnd, &mut r);
                let (w, h) = (r.right.max(1), r.bottom.max(1));
                // doble búfer: se dibuja todo en memoria y se copia de una (sin parpadeo)
                let mem = CreateCompatibleDC(Some(dc));
                let bmp = CreateCompatibleBitmap(dc, w, h);
                let old = SelectObject(mem, bmp.into());
                if with_app(|a| a.paint_gallery(mem, w, h)).is_none() {
                    fill(mem, rc(0, 0, w, h), BG_COLOR);
                }
                let _ = BitBlt(dc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
                SelectObject(mem, old);
                let _ = DeleteObject(bmp.into());
                let _ = DeleteDC(mem);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_SIZE => {
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as i32;
                let mi = &mut *(lp.0 as *mut MINMAXINFO);
                mi.ptMinTrackSize = POINT { x: 780 * dpi / 96, y: 560 * dpi / 96 };
                LRESULT(0)
            }
            WM_TIMER => {
                if wp.0 == 2 {
                    with_app(|a| {
                        if let Some(g) = a.gallery.as_mut() {
                            g.scroll_step();
                        }
                    });
                    return LRESULT(0);
                }
                // reloj de la grabación y estado
                with_app(|a| {
                    if a.manual.is_some() {
                        a.redraw_gallery();
                    }
                });
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = pt();
                with_app(|a| {
                    let Some(g) = a.gallery.as_mut() else { return };
                    if g.drag.is_some() {
                        g.drag_to(y);
                        return;
                    }
                    if !g.tracking {
                        let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                        let _ = TrackMouseEvent(&mut tme);
                        g.tracking = true;
                    }
                    let h = g.hit_at(x, y);
                    if h != g.hover {
                        g.hover = h;
                        g.redraw();
                    }
                });
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                with_app(|a| {
                    if let Some(g) = a.gallery.as_mut() {
                        g.tracking = false;
                        if g.hover != Hit::None {
                            g.hover = Hit::None;
                            g.redraw();
                        }
                    }
                });
                LRESULT(0)
            }
            WM_SETCURSOR => {
                if (lp.0 & 0xFFFF) as u32 == HTCLIENT && with_app(|a| a.gallery.as_ref().is_some_and(|g| g.hover != Hit::None)).unwrap_or(false) {
                    SetCursor(LoadCursorW(None, IDC_HAND).ok());
                    return LRESULT(1);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let _ = SetFocus(Some(hwnd));
                let (x, y) = pt();
                if let Some(Some(Outside::Menu)) = with_app(|a| a.gallery_click(x, y, msg == WM_LBUTTONDBLCLK)) {
                    let main = HWND(MAIN_HWND.load(Ordering::Relaxed) as _);
                    show_menu(main);
                    with_app(|a| {
                        a.fill_gallery();
                    });
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                with_app(|a| {
                    if let Some(g) = a.gallery.as_mut() {
                        if g.drag.take().is_some() {
                            let _ = ReleaseCapture();
                            g.redraw();
                        }
                    }
                });
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                let (x, y) = pt();
                let on_card = with_app(|a| {
                    let g = a.gallery.as_mut()?;
                    match g.hit_at(x, y) {
                        Hit::Card(i) | Hit::Play(i) | Hit::Act(i, _) => {
                            g.sel = Some(i);
                            g.redraw();
                            Some(g.menu_info(i))
                        }
                        _ => None,
                    }
                })
                .flatten();
                if let Some((video, fav, marked, joins)) = on_card {
                    let _ = UpdateWindow(hwnd);
                    let cmd = card_menu(hwnd, video, fav, marked, joins);
                    if cmd != 0 {
                        with_app(|a| a.gallery_cmd(cmd));
                    }
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xFFFF) as i16 as i32;
                let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as i32;
                with_app(|a| {
                    if let Some(g) = a.gallery.as_mut() {
                        g.scroll_by(-delta * 140 * dpi / 96 / 120);
                    }
                });
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let vk = wp.0 as u16;
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                // tecla de menú o Shift+F10: el menú de la tarjeta elegida
                if vk == 0x5D || (shift && vk == 0x79) {
                    let info = with_app(|a| a.gallery.as_ref().and_then(|g| g.sel.map(|i| g.menu_info(i)))).flatten();
                    if let Some((video, fav, marked, joins)) = info {
                        let cmd = card_menu(hwnd, video, fav, marked, joins);
                        if cmd != 0 {
                            with_app(|a| a.gallery_cmd(cmd));
                        }
                    }
                    return LRESULT(0);
                }
                let out = with_app(|a| {
                    let g = a.gallery.as_mut()?;
                    if vk == 0x09 {
                        g.tab(shift);
                        return None;
                    }
                    // Enter/Espacio sobre una sección o un botón con foco
                    if matches!(vk, 0x0D | 0x20) {
                        if let Some(f) = g.focus.filter(|f| *f != Hit::Card(usize::MAX)) {
                            return a.gallery_activate(f, false, 0);
                        }
                    }
                    if g.focus.is_some_and(|f| f != Hit::Card(usize::MAX)) && matches!(vk, 0x25..=0x28) {
                        g.focus = Some(Hit::Card(usize::MAX));
                    }
                    let cmd = if ctrl && vk == b'C' as u16 { Some(203) } else { g.key(vk) };
                    if let Some(c) = cmd {
                        a.gallery_cmd(c);
                    }
                    None
                })
                .flatten();
                if let Some(Outside::Menu) = out {
                    show_menu(HWND(MAIN_HWND.load(Ordering::Relaxed) as _));
                    with_app(|a| a.fill_gallery());
                }
                LRESULT(0)
            }
            WM_GETDLGCODE => LRESULT(DLGC_WANTALLKEYS as isize),
            WM_COMMAND => {
                let id = wp.0 & 0xFFFF;
                if lp.0 != 0 {
                    // aviso del cuadro de renombrar: si pierde el foco, se guarda
                    if id == 400 && ((wp.0 >> 16) & 0xFFFF) as u32 == EN_KILLFOCUS {
                        let _ = PostMessageW(Some(hwnd), WM_RENAME_DONE, WPARAM(1), LPARAM(0));
                    }
                    return LRESULT(0);
                }
                with_app(|a| a.gallery_cmd(id));
                LRESULT(0)
            }
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetTextColor(dc, COLORREF(TEXT));
                SetBkColor(dc, COLORREF(rgb(57, 57, 60)));
                let br = with_app(|a| a.gallery.as_ref().map(|g| g.edit_brush)).flatten().unwrap_or_default();
                LRESULT(br.0 as isize)
            }
            WM_RENAME => {
                with_app(|a| a.begin_rename());
                LRESULT(0)
            }
            WM_RENAME_DONE => {
                with_app(|a| a.end_rename(wp.0 == 1));
                LRESULT(0)
            }
            WM_THUMB => {
                let m = Box::from_raw(lp.0 as *mut (u64, usize, isize, Option<u32>, (i32, i32)));
                let (gen, idx, bmp, dur, size) = *m;
                let bmp = HBITMAP(bmp as _);
                if with_app(|a| a.add_thumb(gen, idx, bmp, dur, size)).is_none() && !bmp.is_invalid() {
                    let _ = DeleteObject(bmp.into());
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // cerrar con la X: se libera la ventana entera (miniaturas, fuentes, hilo) y Jugadón sigue en la bandeja
                let _ = KillTimer(Some(hwnd), 1);
                with_app(|a| {
                    a.gallery = None;
                    a.share_token = None;
                });
                trim_ram();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    // ---------- visor / editor de un clip ----------
    // Reproduce con Media Foundation de Windows (decodifica la placa de video, casi nada de procesador).
    // Solo existe mientras está abierto; la barra de tiempo se redibuja sola (10 veces por segundo y solo
    // mientras reproduce). Recortar es una copia sin recomprimir: instantáneo hasta en la peor PC.

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum VHit {
        None,
        Track,
        In,
        Out,
        Play,
        /// 0 guardar recorte, 1 copiar link, 2 discord, 3 reproductor de Windows
        Btn(u8),
    }

    struct Viewer {
        hwnd: HWND,
        video: HWND,
        player: Option<windows::Win32::Media::MediaFoundation::IMFPMediaPlayer>,
        path: PathBuf,
        title: String,
        /// duración en segundos (0 = todavía no se sabe)
        dur: f64,
        pos: f64,
        playing: bool,
        inp: f64,
        out: f64,
        drag: VHit,
        hover: VHit,
        hits: Vec<(RECT, VHit)>,
        fonts: [HFONT; 4],
    }

    impl Viewer {
        fn s(&self, v: i32) -> i32 {
            let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(self.hwnd) }.max(96) as i32;
            v * dpi / 96
        }

        fn panel_h(&self) -> i32 {
            self.s(132)
        }

        fn layout(&self) {
            unsafe {
                let mut r = RECT::default();
                let _ = GetClientRect(self.hwnd, &mut r);
                let top = self.s(58);
                let _ = MoveWindow(self.video, 0, top, r.right, (r.bottom - top - self.panel_h()).max(1), true);
                if let Some(p) = &self.player {
                    let _ = p.UpdateVideo();
                }
            }
        }

        fn redraw_panel(&self) {
            unsafe {
                let mut r = RECT::default();
                let _ = GetClientRect(self.hwnd, &mut r);
                r.top = r.bottom - self.panel_h();
                let _ = InvalidateRect(Some(self.hwnd), Some(&r), false);
            }
        }

        /// Lee posición y duración del reproductor.
        fn poll(&mut self) {
            use windows::Win32::Media::MediaFoundation::MFP_POSITIONTYPE_100NS;
            let Some(p) = &self.player else { return };
            unsafe {
                if self.dur <= 0.0 {
                    if let Ok(v) = p.GetDuration(&MFP_POSITIONTYPE_100NS) {
                        if let Ok(d) = u64::try_from(&v) {
                            self.dur = d as f64 / 1e7;
                            if self.out <= 0.0 {
                                self.out = self.dur;
                            }
                        }
                    }
                }
                if let Ok(v) = p.GetPosition(&MFP_POSITIONTYPE_100NS) {
                    if let Ok(t) = i64::try_from(&v) {
                        self.pos = t as f64 / 1e7;
                    }
                }
            }
            // con un recorte marcado, la reproducción da la vuelta dentro del recorte (para ver cómo queda)
            if self.playing && self.trimmed() && self.pos >= self.out - 0.05 {
                self.seek(self.inp);
            }
        }

        fn trimmed(&self) -> bool {
            self.dur > 0.0 && (self.inp > 0.05 || self.out < self.dur - 0.05)
        }

        fn seek(&mut self, t: f64) {
            use windows::Win32::Media::MediaFoundation::MFP_POSITIONTYPE_100NS;
            let t = t.clamp(0.0, if self.dur > 0.0 { self.dur } else { t.max(0.0) });
            self.pos = t;
            if let Some(p) = &self.player {
                let v = windows::Win32::System::Com::StructuredStorage::PROPVARIANT::from((t * 1e7) as i64);
                unsafe {
                    let _ = p.SetPosition(&MFP_POSITIONTYPE_100NS, &v);
                }
            }
            self.redraw_panel();
        }

        fn toggle_play(&mut self) {
            let Some(p) = &self.player else { return };
            unsafe {
                if self.playing {
                    let _ = p.Pause();
                    let _ = KillTimer(Some(self.hwnd), 1);
                } else {
                    let _ = p.Play();
                    SetTimer(Some(self.hwnd), 1, 100, None);
                }
            }
            self.playing = !self.playing;
            self.redraw_panel();
        }

        fn track_rect(&self) -> RECT {
            let mut r = RECT::default();
            unsafe {
                let _ = GetClientRect(self.hwnd, &mut r);
            }
            let y = r.bottom - self.panel_h() + self.s(18);
            rc(self.s(24), y, r.right - self.s(24), y + self.s(34))
        }

        fn time_at(&self, x: i32) -> f64 {
            let tr = self.track_rect();
            let f = ((x - tr.left) as f64 / (tr.right - tr.left).max(1) as f64).clamp(0.0, 1.0);
            f * self.dur.max(0.0)
        }

        fn x_at(&self, t: f64) -> i32 {
            let tr = self.track_rect();
            tr.left + ((tr.right - tr.left) as f64 * if self.dur > 0.0 { (t / self.dur).clamp(0.0, 1.0) } else { 0.0 }) as i32
        }

        fn hit(&self, x: i32, y: i32) -> VHit {
            self.hits.iter().rev().find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map_or(VHit::None, |h| h.1)
        }

        /// Mover el inicio/fin del recorte o la posición (arrastrando).
        fn drag_to(&mut self, x: i32) {
            let t = self.time_at(x);
            match self.drag {
                VHit::In => {
                    self.inp = t.min(self.out - 0.5).max(0.0);
                    self.seek(self.inp);
                }
                VHit::Out => {
                    self.out = t.max(self.inp + 0.5).min(self.dur);
                    self.seek(self.out);
                }
                VHit::Track => self.seek(t),
                _ => {}
            }
        }

        unsafe fn paint(&mut self, dc: HDC, w: i32, h: i32) {
            let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(self.hwnd).max(96) as i32;
            let s = move |v: i32| v * dpi / 96;
            let [title_f, body, bold, icon] = self.fonts;
            self.hits.clear();
            SetBkMode(dc, TRANSPARENT);
            // encabezado
            fill(dc, rc(0, 0, w, s(58)), SIDE);
            round(dc, rc(s(20), s(23), s(23), s(35)), s(3), ORANGE);
            text(dc, &self.title, rc(s(36), 0, w - s(20), s(58)), title_f, TEXT, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            if self.player.is_none() {
                let vr = rc(0, s(58), w, h - self.panel_h());
                fill(dc, vr, rgb(9, 9, 12));
                text(dc, "No se puede reproducir acá. Usá \"Reproductor de Windows\".", vr, body, MUTED, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
            }
            // panel de abajo
            let py = h - self.panel_h();
            fill(dc, rc(0, py, w, h), SIDE);
            let tr = self.track_rect();
            let mid = (tr.top + tr.bottom) / 2;
            round(dc, rc(tr.left, mid - s(4), tr.right, mid + s(4)), s(8), rgb(57, 57, 60));
            if self.dur > 0.0 {
                let (xi, xo, xp) = (self.x_at(self.inp), self.x_at(self.out), self.x_at(self.pos));
                // parte que queda en el recorte
                round(dc, rc(xi, mid - s(5), xo.max(xi + 1), mid + s(5)), s(10), ORANGE);
                // lo que se saca, más oscuro
                if xi > tr.left {
                    round(dc, rc(tr.left, mid - s(4), xi, mid + s(4)), s(8), rgb(41, 41, 44));
                }
                if xo < tr.right {
                    round(dc, rc(xo, mid - s(4), tr.right, mid + s(4)), s(8), rgb(41, 41, 44));
                }
                // manijas del recorte
                for (x, which) in [(xi, VHit::In), (xo, VHit::Out)] {
                    let hr = rc(x - s(7), tr.top, x + s(7), tr.bottom);
                    let on = self.hover == which || self.drag == which;
                    round(dc, hr, s(6), if on { TEXT } else { rgb(255, 214, 0) });
                    self.hits.push((rc(x - s(14), tr.top - s(6), x + s(14), tr.bottom + s(6)), which));
                }
                // posición
                ellipse(dc, rc(xp - s(9), mid - s(9), xp + s(9), mid + s(9)), TEXT);
            }
            self.hits.insert(0, (rc(tr.left - s(8), tr.top - s(6), tr.right + s(8), tr.bottom + s(6)), VHit::Track));
            // botón play/pausa
            let by = py + s(64);
            let pr = rc(s(24), by, s(24) + s(52), by + s(52));
            ellipse(dc, pr, if self.hover == VHit::Play { rgb(255, 144, 70) } else { ORANGE });
            glyph_at(dc, if self.playing { 0xE769 } else { 0xE768 }, rc(pr.left + if self.playing { 0 } else { s(2) }, pr.top, pr.right, pr.bottom), icon, TEXT);
            self.hits.push((pr, VHit::Play));
            let t = if self.trimmed() {
                format!("{} / {}   ·   recorte: {}", logic::fmt_elapsed(self.pos as u64), logic::fmt_elapsed(self.dur.round() as u64), logic::fmt_elapsed((self.out - self.inp).round() as u64))
            } else {
                format!("{} / {}", logic::fmt_elapsed(self.pos as u64), logic::fmt_elapsed(self.dur.round() as u64))
            };
            // botones a la derecha (el de abrir afuera, solo con el ícono)
            let mut x = w - s(24);
            for (b, label, glyph) in [(3u8, "", 0xE8A7u16), (2, "Discord", 0xE8BD), (1, "Copiar link", 0xE71B), (5, "Sonido", 0xE767), (4, "Vertical", 0xE717), (0, "Guardar recorte", 0xE8C6)] {
                let tw = if label.is_empty() { s(44) } else { text_w(dc, bold, label) + s(52) };
                let r = rc(x - tw, by + s(4), x, by + s(48));
                let on = self.hover == VHit::Btn(b);
                let accent = b == 0 && self.trimmed();
                if accent {
                    round(dc, r, s(22), if on { rgb(255, 144, 70) } else { ORANGE });
                } else {
                    round(dc, r, s(22), if on { rgb(72, 72, 75) } else { rgb(48, 48, 51) });
                }
                let gr = if label.is_empty() { r } else { rc(r.left + s(10), r.top, r.left + s(38), r.bottom) };
                glyph_at(dc, glyph, gr, icon, if accent { TEXT } else { ORANGE });
                text(dc, label, rc(r.left + s(40), r.top, r.right, r.bottom), bold, if b == 0 && !self.trimmed() { MUTED } else { TEXT }, DT_SINGLELINE | DT_VCENTER);
                self.hits.push((r, VHit::Btn(b)));
                if self.hover == VHit::Btn(b) && label.is_empty() {
                    // qué hace el botón de solo ícono
                    let tip = "Abrir en el reproductor de Windows";
                    let tw2 = text_w(dc, body, tip) + s(16);
                    let tr2 = rc(r.right - tw2, r.top - s(30), r.right, r.top - s(6));
                    round(dc, tr2, s(10), rgb(48, 48, 51));
                    text(dc, tip, tr2, body, TEXT, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                }
                x = r.left - s(8);
            }
            text(dc, &t, rc(pr.right + s(14), by, x - s(6), by + s(52)), bold, TEXT, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            if !self.trimmed() && self.dur > 0.0 {
                text(dc, "Arrastrá las manijas amarillas para recortar  ·  Espacio: pausa  ·  I / O: marcar inicio y fin", rc(s(24), h - s(22), w, h - s(4)), body, MUTED, DT_SINGLELINE | DT_VCENTER);
            }
        }
    }

    impl Drop for Viewer {
        fn drop(&mut self) {
            unsafe {
                if let Some(p) = self.player.take() {
                    let _ = p.Shutdown();
                }
                let _ = windows::Win32::Media::MediaFoundation::MFShutdown();
                for f in self.fonts {
                    let _ = DeleteObject(f.into());
                }
            }
        }
    }

    impl App {
        fn open_viewer(&mut self, path: PathBuf, title: String, dur: Option<u32>) {
            unsafe {
                if let Some(v) = self.viewer.take() {
                    let h = v.hwnd;
                    drop(v);
                    let _ = DestroyWindow(h);
                }
                let hinst: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
                let dpi = windows::Win32::UI::HiDpi::GetDpiForSystem().max(96) as i32;
                let sc = |v: i32| v * dpi / 96;
                let t = wide(&format!("Jugadón – {title}"));
                let Ok(hwnd) = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("jugadon_viewer"),
                    PCWSTR(t.as_ptr()),
                    WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    sc(1040),
                    sc(720),
                    None,
                    None,
                    Some(hinst),
                    None,
                ) else {
                    return;
                };
                dark_title_bar(hwnd);
                let _ = SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_SMALL as usize)), Some(LPARAM(self.icon_on.0 as isize)));
                let video = CreateWindowExW(WINDOW_EX_STYLE(0), w!("jugadon_video"), PCWSTR::null(), WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS, 0, 0, 10, 10, Some(hwnd), None, Some(hinst), None).unwrap_or_default();
                let font = |px: i32, weight: i32, face: PCWSTR| {
                    CreateFontW(-sc(px), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, face)
                };
                let fonts = [font(18, 700, ui_face()), font(13, 400, ui_face()), font(14, 600, ui_face()), font(18, 400, icon_face())];
                // reproductor de Windows (Media Foundation): decodifica la placa de video
                let _ = windows::Win32::Media::MediaFoundation::MFStartup(0x0002_0070, 1);
                let url = wide(&path.to_string_lossy());
                let mut player = None;
                let created = windows::Win32::Media::MediaFoundation::MFPCreateMediaPlayer(
                    PCWSTR(url.as_ptr()),
                    true,
                    windows::Win32::Media::MediaFoundation::MFP_CREATION_OPTIONS(0),
                    None,
                    Some(video),
                    Some(&mut player),
                );
                if let Err(e) = &created {
                    log_line(&self.log_path, &format!("visor: no se pudo reproducir ({e})"));
                }
                let d = dur.map_or(0.0, |d| d as f64);
                let v = Viewer {
                    hwnd,
                    video,
                    player: if created.is_ok() { player } else { None },
                    path,
                    title,
                    dur: d,
                    pos: 0.0,
                    playing: created.is_ok(),
                    inp: 0.0,
                    out: d,
                    drag: VHit::None,
                    hover: VHit::None,
                    hits: Vec::new(),
                    fonts,
                };
                v.layout();
                if v.playing {
                    SetTimer(Some(hwnd), 1, 100, None);
                }
                self.viewer = Some(v);
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
                log_line(&self.log_path, "visor abierto");
            }
        }

        /// Vertical (262/263) o sonido (290..) del clip que se está viendo.
        fn viewer_cmd(&mut self, id: usize) {
            let Some(path) = self.viewer.as_ref().map(|v| v.path.clone()) else { return };
            match id {
                262 | 263 => self.special_copy(path, id - 262),
                290..=294 => self.special_copy(path, 10 + id - 290),
                _ => {}
            }
        }

        fn viewer_btn(&mut self, b: u8) {
            let Some(v) = self.viewer.as_ref() else { return };
            let (path, inp, out, dur, trimmed) = (v.path.clone(), v.inp, v.out, v.dur, v.trimmed());
            match b {
                0 if trimmed => self.copy_of(path, 3, inp, out, Some(dur as u32)),
                0 => self.notify("Arrastrá las manijas amarillas de la barra para elegir qué parte guardar.", false),
                1 => self.share_link(path, false),
                2 => self.copy_of(path, 2, 0.0, 0.0, (dur > 0.0).then_some(dur.round() as u32)),
                _ => {
                    if let Some(v) = self.viewer.as_mut() {
                        if v.playing {
                            v.toggle_play();
                        }
                    }
                    let p = wide(&path.to_string_lossy());
                    unsafe {
                        ShellExecuteW(None, w!("open"), PCWSTR(p.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
                    }
                }
            }
        }
    }

    unsafe extern "system" fn video_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_ERASEBKGND => {
                let mut r = RECT::default();
                let _ = GetClientRect(hwnd, &mut r);
                FillRect(HDC(wp.0 as _), &r, HBRUSH(GetStockObject(BLACK_BRUSH).0));
                LRESULT(1)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let _ = BeginPaint(hwnd, &mut ps);
                with_app(|a| {
                    if let Some(p) = a.viewer.as_ref().and_then(|v| v.player.as_ref()) {
                        let _ = p.UpdateVideo();
                    }
                });
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            // clic en el video: pausa / sigue
            WM_LBUTTONUP => {
                with_app(|a| {
                    if let Some(v) = a.viewer.as_mut() {
                        v.toggle_play();
                    }
                });
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    unsafe extern "system" fn viewer_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        let pt = || ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32);
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut r = RECT::default();
                let _ = GetClientRect(hwnd, &mut r);
                let (w, h) = (r.right.max(1), r.bottom.max(1));
                let mem = CreateCompatibleDC(Some(dc));
                let bmp = CreateCompatibleBitmap(dc, w, h);
                let old = SelectObject(mem, bmp.into());
                if with_app(|a| a.viewer.as_mut().map(|v| v.paint(mem, w, h))).flatten().is_none() {
                    fill(mem, rc(0, 0, w, h), SIDE);
                }
                // el video lo dibuja su propia ventana: solo se copian el encabezado y el panel
                let top = 58 * windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as i32 / 96;
                let panel = 132 * windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as i32 / 96;
                let _ = BitBlt(dc, 0, 0, w, top, Some(mem), 0, 0, SRCCOPY);
                let _ = BitBlt(dc, 0, h - panel, w, panel, Some(mem), 0, h - panel, SRCCOPY);
                SelectObject(mem, old);
                let _ = DeleteObject(bmp.into());
                let _ = DeleteDC(mem);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_SIZE => {
                with_app(|a| {
                    if let Some(v) = &a.viewer {
                        v.layout();
                    }
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as i32;
                let mi = &mut *(lp.0 as *mut MINMAXINFO);
                mi.ptMinTrackSize = POINT { x: 980 * dpi / 96, y: 460 * dpi / 96 };
                LRESULT(0)
            }
            WM_TIMER => {
                with_app(|a| {
                    if let Some(v) = a.viewer.as_mut() {
                        v.poll();
                        v.redraw_panel();
                    }
                });
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = pt();
                with_app(|a| {
                    let Some(v) = a.viewer.as_mut() else { return };
                    if v.drag != VHit::None {
                        v.drag_to(x);
                        return;
                    }
                    let h = v.hit(x, y);
                    if h != v.hover {
                        v.hover = h;
                        v.redraw_panel();
                    }
                });
                LRESULT(0)
            }
            WM_SETCURSOR => {
                if (lp.0 & 0xFFFF) as u32 == HTCLIENT && with_app(|a| a.viewer.as_ref().is_some_and(|v| v.hover != VHit::None)).unwrap_or(false) {
                    SetCursor(LoadCursorW(None, IDC_HAND).ok());
                    return LRESULT(1);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = pt();
                let btn = with_app(|a| {
                    let v = a.viewer.as_mut()?;
                    match v.hit(x, y) {
                        VHit::Play => v.toggle_play(),
                        h @ (VHit::Track | VHit::In | VHit::Out) => {
                            v.drag = h;
                            v.drag_to(x);
                            SetCapture(hwnd);
                        }
                        VHit::Btn(b) => return Some(b),
                        VHit::None => {}
                    }
                    None
                })
                .flatten();
                if let Some(b) = btn {
                    if b == 4 || b == 5 {
                        // el menú va fuera del préstamo de App (tiene su propio bucle de mensajes)
                        let cmd = popup_tool(hwnd, b == 5);
                        if cmd != 0 {
                            with_app(|a| a.viewer_cmd(cmd));
                        }
                    } else {
                        with_app(|a| a.viewer_btn(b));
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                with_app(|a| {
                    if let Some(v) = a.viewer.as_mut() {
                        if v.drag != VHit::None {
                            v.drag = VHit::None;
                            let _ = ReleaseCapture();
                            v.redraw_panel();
                        }
                    }
                });
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let vk = wp.0 as u32;
                if vk == 0x1B {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                let btn = with_app(|a| {
                    let v = a.viewer.as_mut()?;
                    match vk {
                        0x20 => v.toggle_play(),
                        0x25 => v.seek(v.pos - 5.0),
                        0x27 => v.seek(v.pos + 5.0),
                        0x49 if v.dur > 0.0 => {
                            v.inp = v.pos.min(v.out - 0.5).max(0.0);
                            v.redraw_panel();
                        }
                        0x4F if v.dur > 0.0 => {
                            v.out = v.pos.max(v.inp + 0.5).min(v.dur);
                            v.redraw_panel();
                        }
                        0x0D => return Some(0),
                        0x4C => return Some(1),
                        _ => {}
                    }
                    None
                })
                .flatten();
                if let Some(b) = btn {
                    with_app(|a| a.viewer_btn(b));
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                // prueba automática: vertical / sonido desde el visor
                if wp.0 & 0xFFFF >= 200 {
                    with_app(|a| a.viewer_cmd(wp.0 & 0xFFFF));
                }
                // prueba automática: marcar un recorte de 1 a 4 s y guardarlo
                if wp.0 & 0xFFFF == 1 {
                    with_app(|a| {
                        if let Some(v) = a.viewer.as_mut() {
                            if v.dur <= 0.0 {
                                v.dur = 10.0;
                            }
                            v.inp = 1.0;
                            v.out = 4.0f64.min(v.dur);
                        }
                        a.viewer_btn(0);
                    });
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // cerrar: se apaga el reproductor y se devuelve todo
                let _ = KillTimer(Some(hwnd), 1);
                let v = with_app(|a| a.viewer.take()).flatten();
                drop(v);
                trim_ram();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    // ---------- cartelito arriba del juego ("Clip guardado ✓") ----------
    // Ventana chiquita, siempre arriba, que no roba el foco ni los clics y no sale en las grabaciones.
    // Se dibuja una sola vez por aviso y se desvanece sola (un temporizador de 16 ms por 2,7 s).

    struct Ovl {
        hwnd: HWND,
        title: String,
        sub: String,
        kind: u8,
        t0: Instant,
        /// título, texto chico, íconos
        fonts: [HFONT; 3],
        dpi: i32,
    }

    thread_local! {
        static OVL: RefCell<Option<Ovl>> = const { RefCell::new(None) };
    }

    /// Devuelve false si este Windows no deja ocultarlo de las capturas (entonces no se muestra).
    unsafe fn overlay_show(title: &str, sub: &str, kind: u8) -> bool {
        let existing = OVL.with(|o| o.borrow().as_ref().map(|v| v.hwnd)).filter(|h| IsWindow(Some(*h)).as_bool());
        let hwnd = match existing {
            Some(h) => h,
            None => {
                let hinst = GetModuleHandleW(None).unwrap_or_default();
                let Ok(h) = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    w!("jugadon_aviso"),
                    w!("Jugadón"),
                    WS_POPUP,
                    0,
                    0,
                    1,
                    1,
                    None,
                    None,
                    Some(hinst.into()),
                    None,
                ) else {
                    return false;
                };
                // WDA_EXCLUDEFROMCAPTURE (Windows 10 2004+): se ve en pantalla pero no en los clips
                if SetWindowDisplayAffinity(h, WINDOW_DISPLAY_AFFINITY(0x11)).is_err() {
                    let _ = DestroyWindow(h);
                    return false;
                }
                let dpi = (windows::Win32::UI::HiDpi::GetDpiForSystem() as i32).max(96);
                let font = |px: i32, weight: i32, face: PCWSTR| {
                    CreateFontW(-(px * dpi / 96), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, face)
                };
                let fonts = [font(17, 700, ui_face()), font(13, 400, ui_face()), font(18, 400, icon_face())];
                OVL.with(|o| *o.borrow_mut() = Some(Ovl { hwnd: h, title: String::new(), sub: String::new(), kind: 0, t0: Instant::now(), fonts, dpi }));
                h
            }
        };
        let Some((fonts, dpi)) = OVL.with(|o| {
            let mut o = o.borrow_mut();
            let v = o.as_mut()?;
            v.title = title.to_string();
            v.sub = sub.to_string();
            v.kind = kind;
            v.t0 = Instant::now();
            Some((v.fonts, v.dpi))
        }) else {
            return false;
        };
        let s = |v: i32| v * dpi / 96;
        // ancho según el texto
        let dc = GetDC(Some(hwnd));
        let tw = text_w(dc, fonts[0], title).max(text_w(dc, fonts[1], sub));
        ReleaseDC(Some(hwnd), dc);
        let (w, h) = ((tw + s(88)).clamp(s(220), s(520)), s(66));
        // arriba a la derecha del monitor donde está el juego
        let mon = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let r = if GetMonitorInfoW(mon, &mut mi).as_bool() { mi.rcMonitor } else { rc(0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, s(20), s(20));
        SetWindowRgn(hwnd, Some(rgn), false);
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), r.right - w - s(28), r.top + s(28), w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        let _ = InvalidateRect(Some(hwnd), None, false);
        SetTimer(Some(hwnd), 1, 16, None);
        true
    }

    unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_NCHITTEST => LRESULT(-1), // HTTRANSPARENT: los clics pasan al juego
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut r = RECT::default();
                let _ = GetClientRect(hwnd, &mut r);
                let (w, h) = (r.right.max(1), r.bottom.max(1));
                let mem = CreateCompatibleDC(Some(dc));
                let bmp = CreateCompatibleBitmap(dc, w, h);
                let old = SelectObject(mem, bmp.into());
                SetBkMode(mem, TRANSPARENT);
                OVL.with(|o| {
                    let o = o.borrow();
                    let Some(v) = o.as_ref() else { return };
                    let s = |x: i32| x * v.dpi / 96;
                    fill(mem, rc(0, 0, w, h), rgb(22, 22, 25));
                    let (a, b) = match v.kind {
                        1 => (RED, rgb(255, 90, 110)),
                        2 => (rgb(255, 176, 0), rgb(255, 120, 0)),
                        _ => (ORANGE, PINK),
                    };
                    let _ = b;
                    fill(mem, rc(0, 0, s(4), h), a);
                    let d = s(36);
                    let cr = rc(s(20), (h - d) / 2, s(20) + d, (h + d) / 2);
                    ellipse(mem, cr, a);
                    match v.kind {
                        1 => ellipse(mem, rc(cr.left + s(12), cr.top + s(12), cr.right - s(12), cr.bottom - s(12)), TEXT),
                        2 => glyph_at(mem, 0xE7BA, cr, v.fonts[2], TEXT),
                        _ => glyph_at(mem, 0xE73E, cr, v.fonts[2], TEXT),
                    }
                    let x0 = cr.right + s(14);
                    text(mem, &v.title, rc(x0, s(10), w - s(14), s(36)), v.fonts[0], TEXT, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
                    text(mem, &v.sub, rc(x0, s(35), w - s(14), h - s(10)), v.fonts[1], MUTED, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
                });
                let _ = BitBlt(dc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
                SelectObject(mem, old);
                let _ = DeleteObject(bmp.into());
                let _ = DeleteDC(mem);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_TIMER => {
                let t = OVL.with(|o| o.borrow().as_ref().map(|v| v.t0.elapsed().as_secs_f32())).unwrap_or(9.0);
                // aparece en 0,18 s, queda 2,1 s y se va en 0,4 s
                let a = if t < 0.18 {
                    t / 0.18
                } else if t < 2.3 {
                    1.0
                } else {
                    1.0 - (t - 2.3) / 0.4
                };
                if a <= 0.0 {
                    let _ = KillTimer(Some(hwnd), 1);
                    let _ = ShowWindow(hwnd, SW_HIDE);
                } else {
                    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), (a.min(1.0) * 242.0) as u8, LWA_ALPHA);
                }
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
            cmd.0 as usize
        };
        run_command(hwnd, quit);
    }

    /// Ejecuta un comando del menú (también llega como WM_COMMAND, así la prueba automática del CI
    /// puede usar cada opción sin el mouse).
    fn run_command(hwnd: HWND, id: usize) {
        let quit = id != 0 && with_app(|a| a.command(id)).unwrap_or(false);
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
                    with_app(|a| if a.click_update.get() { a.open_update() } else { a.show_last_file() });
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = wp.0 & 0xFFFF;
                if id == 99 {
                    // grabar la ventana cuyo handle viene en lParam (para la prueba automática)
                    with_app(|a| a.start_manual(Source::Window(lp.0 as u64), "la ventana".to_string()));
                } else {
                    if (100..120).contains(&id) {
                        // la lista de ventanas se arma al construir el menú
                        if let Some(m) = with_app(|a| a.build_menu()) {
                            let _ = DestroyMenu(m);
                        }
                    }
                    run_command(hwnd, id);
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
                } else if wp.0 == TIMER_UPDATE {
                    SetTimer(Some(hwnd), TIMER_UPDATE, 12 * 3600 * 1000, None);
                    with_app(|a| a.check_updates());
                }
                LRESULT(0)
            }
            WM_LINK => {
                let (name, url, exp, hours) = *Box::from_raw(lp.0 as *mut (String, String, u64, u32));
                with_app(|a| a.link_done(name, url, exp, hours));
                LRESULT(0)
            }
            WM_PROBED => {
                with_app(|a| a.probe_done(wp.0));
                LRESULT(0)
            }
            WM_UPDATE => {
                with_app(|a| a.update_found(wp.0 as u32));
                LRESULT(0)
            }
            WM_DONE => {
                let (text, file) = *Box::from_raw(lp.0 as *mut (String, Option<String>));
                with_app(|a| {
                    a.notify(&text, text.starts_with("No "));
                    a.overlay_for(&text);
                    if file.is_some() {
                        a.last_file = file;
                        a.fill_gallery();
                        a.auto_cleanup();
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
                style: CS_DBLCLKS,
                ..Default::default()
            };
            for (name, proc_) in [(w!("jugadon_viewer"), viewer_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT), (w!("jugadon_video"), video_proc)] {
                let vc = WNDCLASSW {
                    lpfnWndProc: Some(proc_),
                    hInstance: hinst.into(),
                    lpszClassName: name,
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                    ..Default::default()
                };
                RegisterClassW(&vc);
            }
            let owc = WNDCLASSW { lpfnWndProc: Some(overlay_proc), hInstance: hinst.into(), lpszClassName: w!("jugadon_aviso"), ..Default::default() };
            RegisterClassW(&owc);
            if RegisterClassW(&gwc) == 0 {
                log_line(&log_path, &format!("no se pudo registrar la clase de la ventana de clips: {:?}", GetLastError()));
            }
            TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
            let Ok(hwnd) = CreateWindowExW(WS_EX_TOOLWINDOW, class, w!("Jugadón"), WS_POPUP, 0, 0, 0, 0, None, None, Some(hinst.into()), None) else {
                return;
            };
            MAIN_HWND.store(hwnd.0 as isize, Ordering::Relaxed);

            let enc = logic::first_encoder(cfg.encoder, cfg.gpu_only);
            let cfg_sound_style = cfg.sound_style;
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
                viewer: None,
                share_token: None,
                tick_ms: 2000,
                probing: false,
                uploading: std::collections::HashSet::new(),
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
                sound: logic::clip_sound_wav(cfg_sound_style),
                update: None,
                click_update: std::cell::Cell::new(false),
            };
            APP.with(|a| *a.borrow_mut() = Some(app));
            with_app(|a| {
                a.add_icon();
                a.register_keys();
                a.recover_recordings();
                a.start_probe();
                a.auto_cleanup();
                a.tick(); // arranca a grabar ya si no está en "solo juegos" o si ya hay un juego abierto
                // "jugadon.exe --clips" abre directo la ventana (sirve para un acceso directo)
                if std::env::args().any(|x| x == "--clips") {
                    a.open_gallery();
                }
            });
            SetTimer(Some(hwnd), TIMER_WATCHDOG, 2000, None);
            SetTimer(Some(hwnd), TIMER_UPDATE, 20_000, None);
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
        /// micrófono en canales aparte (después del juego): ffmpeg arma las 3 pistas
        pub split: bool,
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
            Ok(Some((rate, channels, split))) => Some(Audio { rate, channels, split, tx }),
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

    unsafe fn run(ftx: mpsc::SyncSender<Option<(u32, u16, bool)>>, rx: mpsc::Receiver<File>, mic: bool) {
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
        // con el micrófono: juego y voz en canales separados (se pueden mezclar distinto después)
        let split = mic && sys.ch <= 2;
        let _ = ftx.send(Some((fmt.0, fmt.1, split)));
        let width = if split { 2 } else { 1 };
        let Ok(mut pipe) = rx.recv() else { return };
        let (rate, chs) = (fmt.0 as u64, fmt.1 as usize);
        let ms = |n: u64| rate * n / 1000;

        // Pre-roll: el pipe tiene ~4 KB; la 2da escritura de 10 ms bloquea hasta que ffmpeg empieza a leer.
        let silence10 = vec![0f32; ms(10) as usize * chs * width];
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
                out.resize(n * chs * width, 0.0);
                if split {
                    let (a, b) = sources.split_at_mut(1);
                    crate::logic::interleave2(&mut out, &mut a[0].q, &mut b[0].q, chs);
                } else {
                    for s in sources.iter_mut() {
                        crate::logic::mix_into(&mut out, &mut s.q, 1.0);
                    }
                }
                if sources.len() > 1 && !split {
                    for v in out.iter_mut() {
                        *v = v.clamp(-1.0, 1.0);
                    }
                }
                if !write_f32(&mut pipe, &out) {
                    break; // ffmpeg terminó
                }
                written += n as u64;
                if out.capacity() > ms(500) as usize * chs * width {
                    out = Vec::new(); // después de un bloqueo largo no quedarse con un buffer grande
                }
            }
            for s in sources.iter_mut() {
                crate::logic::trim_queue(&mut s.q, max_q);
            }
        }
    }
}
