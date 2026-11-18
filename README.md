<p align="center"><img src="assets/banner.png" alt="Jugadón" width="100%"></p>

# Jugadón

Clips de juegos para Windows 10/11, desde la bandeja del sistema. Graba siempre los últimos segundos de la pantalla principal y, cuando apretás **F9**, los guarda como `.mp4`. **F10** saca una captura en `.png`. No tiene ventanas, galería ni cuenta: es lo mínimo para que el juego no lo sienta.

## Bajar y usar

1. Entrá a **[Releases](../../releases/latest)** y bajá `Jugadon-windows-x64.zip`.
2. Descomprimilo en una carpeta (por ejemplo `C:\Jugadon`). Tienen que quedar juntos `jugadon.exe` y `ffmpeg.exe`.
3. Abrí `jugadon.exe`. Aparece el ícono de Jugadón (<img src="assets/icon.png" width="16">) en la bandeja, al lado del reloj. Si no lo ves, tocá la flechita `^`.

El exe no está firmado: Windows SmartScreen puede decir "Windows protegió su PC". Tocá **Más información → Ejecutar de todas formas**.

| Tecla (por defecto) | Qué hace |
|---|---|
| **F9** | Guarda los últimos 30 s como clip |
| **F10** | Saca una captura de pantalla (resolución completa) |

Los clips y capturas van a `Videos\Jugadon`. Al guardar aparece un aviso chico, sin sonido.

### Menú (clic en el ícono)

- **Guardar clip** / **Captura**: lo mismo que las teclas.
- **Duración**: 15 s, 30 s, 1 min, 2 min o 5 min.
- **Calidad**: 720p 30 fps (por defecto) o 720p 60 fps.
- **Tecla de clip…** / **Tecla de captura…**: apretá la tecla nueva (se puede combinar con Ctrl, Alt o Shift). Esc cancela.
- **Abrir carpeta**.
- **Iniciar con Windows**.
- **Salir**.

Si pasás el mouse por el ícono, dice con qué encoder está grabando.

La configuración está en `%APPDATA%\Jugadon\config.ini`. Ahí también se puede cambiar la carpeta de salida (`folder=`).

## Por qué no baja los FPS

- **Todo el video se procesa en la placa de video, no en el procesador.** Captura con Desktop Duplication (`ddagrab`), escala a 720p en la GPU (`scale_d3d11`) y comprime con el chip de video dedicado de la placa: **NVENC** (NVIDIA), **AMF** (AMD) o **QSV** (Intel). Ese chip es aparte de lo que usa el juego para dibujar.
- **El juego siempre tiene prioridad.** ffmpeg graba con prioridad "debajo de lo normal". Guardar un clip y sacar una captura van con prioridad "baja". Si la CPU está al 100 %, el que cede es el clip (algún cuadro repetido), no el juego.
- **Guardar no recomprime.** Los últimos segundos están en el disco en pedazos de 2 s (`%TEMP%\jugadon_buf`). Al apretar F9 solo se unen en un `.mp4` (copia directa). Por eso no hay tirón al guardar.
- **Poco trabajo de fondo.** Hay un solo hilo de filtros y un hilo de audio que se despierta cada 20 ms. El chequeo de que ffmpeg siga vivo corre cada 2 s.
- **Nada en RAM.** El buffer está en el disco: ocupa unos 1,5 MB por cada 2 s a 30 fps (unos 230 MB con 5 min) y se borra al salir.
- Si la GPU no puede codificar, Jugadón pasa solo a otra opción. La última es x264 en CPU, y esa **sí** consume CPU. Si pasa eso, el ícono lo dice.

Para menos impacto todavía, dejá **720p 30 fps** (el de fábrica). 60 fps duplica el trabajo de captura.

**RAM:** `jugadon.exe` ocupa pocos MB. Menos de 4 MB en total es imposible para cualquier programa que grabe la pantalla: solo el driver de video y el encoder ya usan más. Fijate los números reales en el Administrador de tareas (`Jugadón` y `ffmpeg.exe`).

## Límites conocidos

- Solo graba el **monitor principal**.
- Audio: solo el **sonido del sistema**. **No graba el micrófono**.
- Sin HDR. Con HDR activado los colores pueden salir lavados.
- Algunos juegos en pantalla completa exclusiva no se capturan bien. En ese caso, usá "ventana sin bordes".
- NVENC con FFmpeg 8 necesita un driver de NVIDIA reciente. Si el tuyo es viejo, Jugadón pasa a otra opción más lenta. Actualizar el driver lo arregla.
- Si otro programa usa F9 o F10, Jugadón avisa. Cambiá la tecla desde el menú.
- Si algo falla, mirá `%TEMP%\jugadon.log`.

## Compilación

Todo se compila en GitHub Actions (`.github/workflows/release.yml`) cada vez que hay cambios en `main`. El workflow:

- corre los tests (incluida una grabación real con ffmpeg),
- compila `jugadon.exe` con su ícono,
- baja una versión fija de FFmpeg y verifica que tenga `ddagrab`, `scale_d3d11`, NVENC, AMF, QSV y x264,
- publica el zip en Releases.

El ícono y el banner se generan con `assets/make_assets.py`.

## Licencias

Jugadón: MIT. `ffmpeg.exe` es FFmpeg (GPLv3). El zip incluye `FFMPEG-LICENCIA.txt`, con la licencia y el link al código fuente.
