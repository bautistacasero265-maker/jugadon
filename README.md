<p align="center"><img src="assets/banner.png" alt="Jugadón" width="100%"></p>

# Jugadón

Clips de juegos para Windows 10/11, pensado para PCs de pocos recursos. Vive en la bandeja del sistema. Mientras jugás, guarda siempre los últimos segundos; cuando apretás **F9**, se guardan como `.mp4` con un sonidito de confirmación. **F10** saca una captura en `.png`. No tiene ventanas, galería ni cuenta: es lo mínimo para que el juego no lo sienta.

- 🎮 **Graba solo cuando estás jugando.** Cuando hay un juego en pantalla completa arranca solo. En el escritorio queda en pausa y no gasta nada.
- ⚡ **Hecho en Rust, todo el video en la placa.** El procesador queda para el juego.
- 🔊 **Sonido del juego y, si querés, tu micrófono.**
- 📋 **Copiar último clip**: lo pegás con Ctrl+V en Discord o WhatsApp.
- 🔔 Aviso al guardar: tocalo y se abre el clip.

## Bajar y usar

1. Entrá a **[Releases](../../releases/latest)** y bajá `Jugadon-windows-x64.zip`.
2. Descomprimilo en una carpeta (por ejemplo `C:\Jugadon`). Tienen que quedar juntos `jugadon.exe` y `ffmpeg.exe`.
3. Abrí `jugadon.exe`. Aparece el ícono de Jugadón en la bandeja, al lado del reloj. Si no lo ves, tocá la flechita `^`.
   - **Gris**: en pausa, esperando un juego.
   - **Naranja** (<img src="assets/icon.png" width="16">): grabando.

El exe no está firmado: Windows SmartScreen puede decir "Windows protegió su PC". Tocá **Más información → Ejecutar de todas formas**.

| Tecla (por defecto) | Qué hace |
|---|---|
| **F9** | Guarda los últimos 30 s como clip |
| **F10** | Saca una captura de pantalla (resolución completa) |

Los clips y capturas van a `Videos\Jugadon`. Al guardar suena un "tin-tin" y aparece un aviso. Si tocás el aviso, se abre la carpeta con el clip marcado.

**¿Qué cuenta como "juego"?** Cualquier ventana que tape toda la pantalla principal: pantalla completa o "ventana sin bordes". Una ventana maximizada común (con la barra de tareas a la vista) no cuenta. Después de salir del juego sigue grabando 1 minuto, así un alt-tab rápido no te borra el clip. Si querés que grabe siempre, desactivá **Opciones → Grabar solo en juegos**.

### Menú (clic en el ícono)

- Arriba de todo, el **estado**: grabando (y con qué encoder), en pausa o error.
- **Guardar clip** / **Captura**: lo mismo que las teclas.
- **Copiar último clip**: después pegalo con Ctrl+V en Discord, WhatsApp, etc.
- **Duración**: 15 s, 30 s, 1 min, 2 min o 5 min.
- **Calidad**: 720p 30 fps (por defecto) o 720p 60 fps.
- **Opciones**:
  - **Grabar solo en juegos**: activado por defecto.
  - **Grabar micrófono**: desactivado por defecto.
  - **Sonido al guardar**.
  - **Tecla de clip…** / **Tecla de captura…**: apretá la tecla nueva (se puede combinar con Ctrl, Alt o Shift). Esc cancela.
  - **Iniciar con Windows**.
- **Abrir carpeta**.
- **Salir**.

La configuración está en `%APPDATA%\Jugadon\config.ini`. Ahí también se puede cambiar la carpeta de salida (`folder=`).

## Por qué no baja los FPS ni consume

- **En reposo no hay nada grabando.** Sin juego abierto, ffmpeg ni siquiera está corriendo. Solo queda `jugadon.exe` esperando y revisando cada 2 s si apareció un juego. Además devuelve su RAM a Windows. En el build, el CI mide su RAM en reposo y la muestra en el log.
- **Todo el video se procesa en la placa de video, no en el procesador.** Captura con Desktop Duplication (`ddagrab`), escala a 720p en la GPU (`scale_d3d11`) y comprime con el chip de video dedicado de la placa: **NVENC** (NVIDIA), **AMF** (AMD) o **QSV** (Intel). Ese chip es aparte de lo que usa el juego para dibujar.
- **El juego siempre tiene prioridad.** ffmpeg graba con prioridad "debajo de lo normal". Guardar un clip y sacar una captura van con prioridad "baja". Si la CPU está al 100 %, el que cede es el clip (algún cuadro repetido), no el juego.
- **Guardar no recomprime.** Los últimos segundos están en el disco en pedazos de 2 s (`%TEMP%\jugadon_buf`). Al apretar F9 solo se unen en un `.mp4` (copia directa). Por eso no hay tirón al guardar.
- **Poco trabajo de fondo.** Hay un solo hilo de filtros y un hilo de audio que se despierta cada 20 ms (solo mientras graba). El chequeo de que ffmpeg siga vivo corre cada 2 s.
- **Nada en RAM.** El buffer está en el disco: ocupa unos 1,5 MB por cada 2 s a 30 fps (unos 230 MB con 5 min) y se borra al salir.
- Si la GPU no puede codificar, Jugadón pasa solo a otra opción. La última es x264 en CPU, y esa **sí** consume CPU. Si pasa eso, el ícono lo dice.

Para menos impacto todavía, dejá **720p 30 fps** (el de fábrica). 60 fps duplica el trabajo de captura.

**RAM:**
- **En reposo** (sin juego): solo corre `jugadon.exe`, y apunta a menos de 4 MB.
- **Mientras grabás**: se suma `ffmpeg.exe`, que tiene abierta la captura y el encoder de la placa. Eso ocupa bastante más que 4 MB, y le pasa a cualquier programa que grabe la pantalla (Medal, ShadowPlay, OBS).

Mirá los números reales en el Administrador de tareas: "Jugadón" y "ffmpeg.exe".

## Límites conocidos

- Solo graba el **monitor principal**.
- Audio: el **sonido del sistema** y, si lo activás, el **micrófono**, mezclados en una sola pista.
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
