<p align="center"><img src="assets/banner.png" alt="Jugadón" width="100%"></p>

# Jugadón

Clips de juegos para Windows 10/11, pensado para PCs de pocos recursos. Vive en la bandeja del sistema. Mientras jugás, guarda siempre los últimos segundos; cuando apretás **F9**, se guardan como `.mp4` con un sonidito de confirmación. **F10** saca una captura en `.png`. No tiene ventanas, galería ni cuenta: es lo mínimo para que el juego no lo sienta.

- 🎮 **Clips en segundo plano, como vos quieras.** Pueden estar activos solo mientras jugás (viene así), siempre o apagados.
- ⏺️ **Grabación normal**: **F8** empieza a grabar la pantalla completa y **F8** la frena y la guarda.
- 🪟 **Grabar una sola ventana** (el juego, el navegador, Discord, la que sea). Sale solo esa ventana, aunque tengas otra cosa adelante.
- ⚡ **Hecho en Rust, todo el video en la placa.** El procesador queda para el juego.
- 🔊 **Sonido del juego + tu micrófono** (el mic viene activado; se apaga en Opciones).
- 🗂️ **Ventana "Mis clips"**: clic en el ícono y ves todos tus clips. Desde ahí los abrís, los compartís (con el menú Compartir de Windows), los copiás para Discord o los borrás.
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
| **F8** | Empieza a grabar la pantalla completa / la frena y la guarda |

Los clips y capturas van a `Videos\Jugadon`. Al guardar suena un "tin-tin" y aparece un aviso. Si tocás el aviso, se abre la carpeta con el clip marcado.

### Dos formas de grabar

1. **Clips en segundo plano.** Jugadón guarda siempre los últimos segundos y vos decidís cuándo quedártelos con **F9**. Es lo de Medal. En el menú → **Clips en segundo plano** elegís:
   - **Solo mientras juego** (recomendado, viene así): arranca solo cuando hay un juego en pantalla completa y se pausa en el escritorio. En pausa no consume nada.
   - **Siempre**: graba los últimos segundos todo el tiempo.
   - **Apagado**: no graba nada por su cuenta. Solo grabás cuando apretás F8.
2. **Grabación de principio a fin.**
   - **F8** (o menú → **Grabar pantalla completa**) empieza.
   - Para una ventana: menú → **Grabar una ventana** y elegís cuál de la lista (aparecen con el nombre que muestran en la barra de título). Se graba **solo esa ventana**, aunque la tapes con otra o estés usando otra cosa adelante. Si cerrás la ventana, la grabación se frena y se guarda sola.
   - **F8** de nuevo (o menú → **Frenar y guardar**) la frena y la guarda como `grabacion_….mp4`.
   - Mientras graba, el ícono se pone **rojo** y al pasar el mouse muestra el tiempo.
   - Si se corta la luz o se cuelga la PC, lo grabado se recupera solo la próxima vez que abrís Jugadón.

**Los clips no se sacan solos.** Jugadón nunca decide qué es una buena jugada: el clip se guarda solo cuando vos apretás F9. "Solo mientras juego" decide únicamente *cuándo* está activa la memoria de los últimos segundos.

**¿Cómo sabe que estás jugando?** Cada 2 segundos mira cuál es la ventana activa. Le alcanza con eso; no lee la memoria del juego ni se mete en él (por eso ningún anticheat lo marca). - Si la ventana activa tapa toda la pantalla principal (pantalla completa o "ventana sin bordes"), cuenta como juego y empiezan los clips.
- Una ventana maximizada común (con la barra de tareas a la vista) no cuenta.
- Después de salir del juego sigue 1 minuto más, así un alt-tab rápido no te borra el clip.

### Ventana "Mis clips" (clic izquierdo en el ícono)

Lista tus clips, grabaciones y capturas, del más nuevo al más viejo. Botones:
- **Abrir**: también con doble clic.
- **Compartir…**: abre el menú Compartir de Windows (WhatsApp, correo, Compartir con dispositivos cercanos, etc.).
- **Copiar**: después pegás con Ctrl+V en Discord o WhatsApp.
- **Mostrar en carpeta**.
- **Eliminar**: va a la papelera.
- **Abrir carpeta**.

Está hecha con los controles propios de Windows: no usa un navegador por dentro, como Electron. Solo existe mientras la tenés abierta: **al cerrarla con la X se libera entera** y Jugadón sigue en la bandeja sin consumir. No lee los videos para armar la lista; solo los nombres y las fechas.

### Menú (clic derecho en el ícono)

- Arriba de todo, el **estado**: grabando (y con qué encoder), en pausa o error.
- **Ver mis clips…**: abre la ventana de arriba.
- **Guardar clip** / **Captura**: lo mismo que las teclas.
- **Grabar pantalla completa** / **Grabar una ventana ▸** / **Frenar y guardar grabación**.
- **Copiar último clip**: después pegalo con Ctrl+V en Discord, WhatsApp, etc.
- **Clips en segundo plano ▸**: Solo mientras juego / Siempre / Apagado.
- **Duración del clip**: 15 s, 30 s, 1 min, 2 min o 5 min.
- **Calidad**:
  - **Automática** (recomendada, viene así): usa lo justo para tu PC. Si comprime la placa, graba en 720p 30 fps. Si tiene que usar el procesador, baja a 480p 30 fps con un solo hilo.
  - Si no, la elegís vos: 480p 30 fps (ahorro máximo), 720p 30 fps o 720p 60 fps.
  - Vale para clips y grabaciones. Una ventana más chica se graba en su tamaño, sin agrandarla.
- **Opciones**:
  - **Grabar micrófono**: activado por defecto.
  - **Sonido al guardar**.
  - **Usar solo la placa de video**: activado por defecto. Nunca usa el procesador para comprimir.
  - **Tecla de clip…** / **Tecla de captura…** / **Tecla de grabar/frenar…**: apretá la tecla nueva (se puede combinar con Ctrl, Alt o Shift). Esc cancela.
  - **Iniciar con Windows**.
- **Abrir carpeta**.
- **Salir**.

La configuración está en `%APPDATA%\Jugadon\config.ini`. Ahí también se puede cambiar la carpeta de salida (`folder=`).

## Por qué no baja los FPS ni consume

- **En reposo no hay nada grabando.** Sin juego abierto (o con los clips apagados), ffmpeg ni siquiera está corriendo. Solo queda `jugadon.exe` esperando, y además devuelve su RAM a Windows. Medido en el CI de GitHub (no en una PC gamer): **120 KB** de RAM privada (lo que muestra el Administrador de tareas) y 16 ms de CPU en 25 segundos.
- **Nunca graba dos cosas a la vez.** Mientras grabás con F8, los clips en segundo plano se pausan.
- **Todo el video se procesa en la placa de video, no en el procesador.** Captura con Desktop Duplication (`ddagrab`), escala a 720p en la GPU (`scale_d3d11`) y comprime con el chip de video dedicado de la placa: **NVENC** (NVIDIA), **AMF** (AMD) o **QSV** (Intel). Ese chip es aparte de lo que usa el juego para dibujar.
- **El juego siempre tiene prioridad.** ffmpeg graba con prioridad "debajo de lo normal". Guardar un clip y sacar una captura van con prioridad "baja". Si la CPU está al 100 %, el que cede es el clip (algún cuadro repetido), no el juego.
- **Guardar no recomprime.** Los últimos segundos están en el disco en pedazos de 2 s (`%TEMP%\jugadon_buf`). Al apretar F9 solo se unen en un `.mp4` (copia directa). Por eso no hay tirón al guardar.
- **Audio liviano.** El audio se comprime con el modo rápido de AAC, que usa mucho menos procesador. El hilo de audio de Jugadón se despierta 25 veces por segundo y solo copia datos.
- **Poco trabajo de fondo.** Hay un solo hilo de filtros y un hilo de audio que se despierta cada 40 ms (solo mientras graba). El chequeo de que ffmpeg siga vivo corre cada 2 s.
- **Nada en RAM.** El buffer está en el disco: ocupa unos 1,5 MB por cada 2 s a 30 fps (unos 230 MB con 5 min) y se borra al salir.
- Si la GPU no puede codificar, Jugadón pasa solo a otra opción. La última es x264 en CPU, y esa **sí** consume CPU. Si pasa eso, el ícono lo dice.

### PC de bajos recursos (por ejemplo i3-2100 con 4 GB)

Jugadón está pensado para estas PCs. Lo que define si se nota o no es **quién comprime el video**:

| Tu PC | Quién comprime | Qué esperar en 720p 30 fps |
|---|---|---|
| Placa con encoder: NVIDIA GTX 600 o más nueva (no la GT 710/1030), AMD con VCE/VCN | El chip de video de la placa, aparte de lo que usa el juego | No deberías notar bajón de FPS |
| Gráficos Intel con Quick Sync (el i3-2100 lo tiene, HD 2000) | Intel Quick Sync, si el driver lo deja usar | Bajo impacto. Con drivers muy viejos puede no estar disponible |
| Sin encoder disponible | El procesador (x264 "ultrafast", 2 hilos, prioridad baja) | Funciona, pero en un procesador de 2 núcleos **se nota algo**. Si te molesta, pasá a **480p** |

Jugadón prueba solo, en ese orden, y se queda con lo más liviano que funcione en tu PC.

**Opciones → "Usar solo la placa de video"** viene **activada**. Así Jugadón comprime **solo** con la placa (NVENC, AMF o Quick Sync) y **nunca** usa el procesador para eso. Si tu placa no puede, no graba y te avisa: el juego no pierde nada. Si preferís que grabe igual usando el procesador, desactivala.

Sobre los "decodificadores": para grabar, la placa **codifica** (comprime) el video. Los decodificadores sirven para **reproducir**, y ahí Windows ya usa la placa solo cuando abrís un clip. Guardar un clip no decodifica ni recodifica nada: solo une los pedazos ya comprimidos. Si pasás el mouse por el ícono, te dice cuál usa; si dice "usa CPU", estás en la última fila.

Con drivers viejos (Windows 10 con el driver de Intel HD 2000/3000) puede no estar la captura moderna de Windows. En ese caso graba por **GDI**, que anda en cualquier PC pero carga más la CPU.

Siempre, en cualquier PC:
- **Clips en segundo plano → Solo mientras juego** (viene así): fuera del juego no corre nada.
- Guardar un clip **nunca** recomprime: une los pedazos en un mp4, con prioridad baja. No debería trabar el juego.
- Con 4 GB de RAM: en reposo Jugadón no ocupa casi nada. Mientras graba, el que ocupa RAM es `ffmpeg.exe`; miralo en el Administrador de tareas.

Con **Calidad → Automática** (la de fábrica) Jugadón ya usa lo justo. Para gastar todavía menos, elegí **480p 30 fps**. Evitá 60 fps en PCs lentas: duplica el trabajo de la placa.

**RAM:**
- **En reposo** (sin juego): solo corre `jugadon.exe`, y apunta a menos de 4 MB.
- **Mientras grabás**: se suma `ffmpeg.exe`, que tiene abierta la captura y el encoder de la placa. Eso ocupa bastante más que 4 MB, y le pasa a cualquier programa que grabe la pantalla (Medal, ShadowPlay, OBS).

Mirá los números reales en el Administrador de tareas: "Jugadón" y "ffmpeg.exe".

## Límites conocidos

- Solo graba el **monitor principal**.
- **Grabar una ventana**:
  - Usa la captura de ventanas de Windows (Windows Graphics Capture), que necesita **Windows 10 versión 2004 o más nuevo**.
  - Si **minimizás** la ventana, Windows deja de mandar imagen y el video queda congelado hasta que la volvés a mostrar.
  - Algunos juegos en pantalla completa *exclusiva* no se pueden grabar como ventana. Para esos usá F8 (pantalla completa) o ponelos en "ventana sin bordes".
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
- baja FFmpeg 8.1 y verifica que tenga `ddagrab`, `gfxcapture`, `scale_d3d11`, NVENC, AMF, QSV y x264,
- publica el zip en Releases.

El ícono y el banner se generan con `assets/make_assets.py`.

## Licencias

Jugadón: MIT. `ffmpeg.exe` es FFmpeg (GPLv3). El zip incluye `FFMPEG-LICENCIA.txt`, con la licencia y el link al código fuente.
