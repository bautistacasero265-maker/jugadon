# rclip

Clips de juegos para Windows 10/11, desde la bandeja del sistema. Graba siempre los últimos segundos de la pantalla principal y con una tecla los guarda como `.mp4`. Otra tecla saca una captura en `.png`. No tiene ventana, galería ni cuenta.

## Instalar

1. Entrá a **[Releases](../../releases)** y bajá `rclip-windows-x64.zip` de la versión más nueva.
2. Descomprimilo en una carpeta (por ejemplo `C:\rclip`). Tienen que quedar juntos `rclip.exe` y `ffmpeg.exe`.
3. Abrí `rclip.exe`. Aparece un **punto rojo** en la bandeja, al lado del reloj (si no lo ves, tocá la flechita `^`).

El exe no está firmado: Windows SmartScreen puede decir "Windows protegió su PC". Tocá **Más información → Ejecutar de todas formas**.

## Uso

| Tecla (por defecto) | Qué hace |
|---|---|
| **F9** | Guarda los últimos 30 s como clip |
| **F10** | Saca una captura de pantalla (resolución completa) |

Los clips y capturas van a `Videos\rclip` (`%USERPROFILE%\Videos\rclip`). Al guardar aparece un aviso chico, sin sonido.

### Menú (clic derecho o izquierdo en el punto rojo)

- **Guardar clip** / **Captura**: lo mismo que las teclas.
- **Duración**: 15 s, 30 s, 1 min, 2 min o 5 min.
- **Calidad**: 720p 30 fps (por defecto) o 720p 60 fps.
- **Tecla de clip…** / **Tecla de captura…**: apretá la tecla nueva (se puede combinar con Ctrl, Alt o Shift). Esc cancela.
- **Abrir carpeta**: abre la carpeta de clips.
- **Iniciar con Windows**: lo activa o desactiva.
- **Salir**.

Si pasás el mouse por el punto rojo, dice con qué encoder está grabando.

La configuración está en `%APPDATA%\rclip\config.ini`. Ahí también se puede cambiar la carpeta de salida (`folder=`).

## Cómo funciona y consumo

- `rclip.exe` (Rust) solo maneja la bandeja, las teclas y el audio. La captura y la compresión las hace `ffmpeg.exe`, que lanza rclip:
  - Captura con Desktop Duplication (`ddagrab`). Los cuadros quedan en la GPU.
  - Los escala a 720p en la GPU (`scale_d3d11`) y los comprime con el encoder de la placa: **NVENC** (NVIDIA), **AMF** (AMD) o **QSV** (Intel).
  - Si eso falla, prueba sola otras opciones. La última es x264 en CPU, que sí consume CPU.
- El "buffer" de los últimos segundos está en el disco (`%TEMP%\rclip_buf`), en pedazos de 2 s, no en RAM. Ocupa unos 1,5 MB por cada 2 s a 30 fps (unos 230 MB con 5 min) y se borra al salir.
- Guardar un clip no vuelve a comprimir nada: une esos pedazos en un `.mp4` (copia directa) con prioridad baja. Por eso no debería trabar el juego.
- **RAM:** rclip apunta a ocupar pocos MB. **Menos de 4 MB en total es imposible** para cualquier programa que grabe la pantalla: solo el driver de video y el encoder de la GPU ya usan más. Fijate los números reales en el Administrador de tareas (`rclip.exe` y `ffmpeg.exe`).
- Los errores de ffmpeg quedan en `%TEMP%\rclip.log`.

## Límites conocidos

- Solo graba el **monitor principal**.
- Audio: solo el **sonido del sistema** (lo que sale por los parlantes o auriculares). **No graba el micrófono**.
- Sin HDR. Con HDR activado los colores pueden salir lavados.
- Juegos en pantalla completa exclusiva pueden no capturarse bien. En ese caso, usá "ventana sin bordes".
- NVENC con FFmpeg 8 necesita un driver de NVIDIA reciente. Si el driver es viejo, rclip pasa solo a otra opción (más lenta). Actualizar el driver lo arregla.
- Si otro programa usa F9 o F10, rclip avisa. Cambiá la tecla desde el menú.

## Compilación

Todo se compila en GitHub Actions (`.github/workflows/release.yml`). El workflow:

- corre los tests,
- compila `rclip.exe`,
- baja una versión fija de FFmpeg y verifica que tenga `ddagrab`, `scale_d3d11`, NVENC, AMF, QSV y x264,
- arma el zip y lo publica en Releases.

## Licencias

rclip: MIT. `ffmpeg.exe` es FFmpeg (GPLv3). El zip incluye `FFMPEG-LICENCIA.txt`, con la licencia y el link al código fuente.
