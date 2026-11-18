# Prueba completa de Jugadón en una PC Windows de GitHub (sin placa de video).
# Abre la app de verdad, aprieta las teclas, usa cada opción del menú (con la ventana "Mis clips"
# cerrada y abierta), corta la app de golpe y revisa que todo lo que guarda sea un video válido.
# No para en el primer error: junta todos y falla al final con la lista.

$ErrorActionPreference = 'Continue'
$root = Resolve-Path "$PSScriptRoot\.."
$exe = Join-Path $root 'Jugadon\jugadon.exe'
$ffprobe = Join-Path $root 'ffmpeg\bin\ffprobe.exe'
$out = Join-Path $env:RUNNER_TEMP 'jugadon_e2e'
$cfgDir = Join-Path $env:APPDATA 'Jugadon'
$cfg = Join-Path $cfgDir 'config.ini'
$log = Join-Path $env:TEMP 'jugadon.log'
$shots = Join-Path $root 'e2e'
Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out, $cfgDir, $shots | Out-Null
Remove-Item $log, (Join-Path $env:TEMP 'jugadon_rec') -Recurse -Force -ErrorAction SilentlyContinue

Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class W {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeout(IntPtr h, uint m, IntPtr w, IntPtr l, uint f, uint t, out IntPtr r);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
  public struct RECT { public int L, T, R, B; }
  public struct POINT { public int X, Y; }
}
"@
Add-Type -AssemblyName System.Drawing

$script:fails = @()
function Check([bool]$ok, [string]$msg) {
  if ($ok) { Write-Host "  OK     $msg" } else { Write-Host "  FALLA  $msg" -ForegroundColor Red; $script:fails += $msg }
}
function Step([string]$t) { Write-Host ""; Write-Host "=== $t ===" -ForegroundColor Cyan }
function Wait-Until([scriptblock]$cond, [double]$secs) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  while ($sw.Elapsed.TotalSeconds -lt $secs) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 400 }
  return [bool](& $cond)
}
function Files([string]$pat) { @(Get-ChildItem $out -Filter $pat -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTime) }
function Log-Has([string]$s) { (Test-Path $log) -and ((Get-Content $log -Raw) -match [regex]::Escape($s)) }
function Log-Count([string]$s) { if (Test-Path $log) { ([regex]::Matches((Get-Content $log -Raw), [regex]::Escape($s))).Count } else { 0 } }
function Key([byte]$vk) { [W]::keybd_event($vk, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 80; [W]::keybd_event($vk, 0, 2, [IntPtr]::Zero) }
function Cmd([int]$id, [long]$lp = 0) { [void][W]::PostMessage($script:main, 0x111, [IntPtr]$id, [IntPtr]$lp); Start-Sleep -Milliseconds 300 }
function GCmd([int]$id) { [void][W]::PostMessage($script:gal, 0x111, [IntPtr]$id, [IntPtr]::Zero); Start-Sleep -Milliseconds 300 }
function Responding([IntPtr]$h) {
  $r = [IntPtr]::Zero
  ([W]::SendMessageTimeout($h, 0, [IntPtr]::Zero, [IntPtr]::Zero, 2, 5000, [ref]$r)) -ne [IntPtr]::Zero
}
function Alive([string]$what) {
  $ok = ($script:proc -and -not $script:proc.HasExited -and (Responding $script:main))
  Check $ok "la app sigue abierta y responde ($what)"
}
function Probe([string]$f) {
  $j = & $ffprobe -v error -show_entries format=duration:stream=codec_type,codec_name,width,height -of json $f | ConvertFrom-Json
  $v = @($j.streams | Where-Object codec_type -eq 'video')
  $a = @($j.streams | Where-Object codec_type -eq 'audio')
  [pscustomobject]@{ Dur = [double]$j.format.duration; Video = $v.Count; Audio = $a.Count; Size = "$($v[0].width)x$($v[0].height)"; Codec = $v[0].codec_name }
}
function Check-Video([string]$f, [double]$minDur, [string]$what) {
  if (-not $f) { Check $false "$what`: no se creó el archivo"; return }
  Start-Sleep -Seconds 1
  $p = Probe $f
  Write-Host ("         {0}: {1:N1} s, {2} {3}, video {4}, audio {5}, {6:N0} KB" -f (Split-Path $f -Leaf), $p.Dur, $p.Codec, $p.Size, $p.Video, $p.Audio, ((Get-Item $f).Length / 1KB))
  Check ($p.Video -eq 1 -and $p.Dur -ge $minDur) "$what es un video válido de al menos $minDur s"
}
function Wait-New([string]$pat, [int]$before, [double]$secs) {
  $ok = Wait-Until { (Files $pat).Count -gt $before } $secs
  if ($ok) {
    # esperar a que termine de escribirse
    $f = (Files $pat)[-1].FullName
    $last = -1
    for ($i = 0; $i -lt 20; $i++) { $len = (Get-Item $f).Length; if ($len -eq $last -and $len -gt 0) { break }; $last = $len; Start-Sleep -Milliseconds 500 }
    return $f
  }
  return $null
}
function Start-App {
  $script:proc = Start-Process -FilePath $exe -WorkingDirectory (Split-Path $exe) -PassThru
  $script:main = [IntPtr]::Zero
  [void](Wait-Until { $script:main = [W]::FindWindow('jugadon_tray', [NullString]::Value); $script:main -ne [IntPtr]::Zero } 20)
}
function Buffer-Running([int]$before, [double]$secs) { Wait-Until { (Log-Count 'grabando con') -gt $before } $secs }
function Cfg([string]$k) { ((Get-Content $cfg) | Where-Object { $_ -like "$k=*" } | Select-Object -First 1) -replace "^$k=", '' }
function Snap([IntPtr]$h, [string]$name) {
  $r = New-Object W+RECT
  if ([W]::DwmGetWindowAttribute($h, 9, [ref]$r, 16) -ne 0) { [void][W]::GetWindowRect($h, [ref]$r) }
  $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
  $bmp.Save((Join-Path $shots $name), [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
}
function Clipboard-Files {
  powershell.exe -NoProfile -Sta -Command "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard]::GetFileDropList() -join '|'"
}

# configuración: clips siempre (en el servidor no hay juegos), 15 s, carpeta propia
@"
version=3
seconds=15
fps=30
quality=0
bg=2
mic=1
sound=1
gpu_only=0
folder=$out
"@ | Set-Content $cfg -Encoding utf8

try {
  Step "1. Arranque y clips en segundo plano"
  Start-App
  Check ($script:main -ne [IntPtr]::Zero) "se abrió (ícono en la bandeja)"
  Check (Wait-Until { Log-Has 'prueba de grabación: se usa' } 120) "al abrir prueba sola cómo grabar y elige una forma que anda"
  Get-Content $log -ErrorAction SilentlyContinue | Select-String 'prueba de grabación' | ForEach-Object { Write-Host "         log: $($_.Line)" }
  Check ((Cfg 'probed') -ne '') "la prueba queda guardada (no se repite en cada inicio)"
  $bufOk = Buffer-Running 0 90
  Check $bufOk "empezó a grabar el buffer de clips"
  Get-Content $log -ErrorAction SilentlyContinue | Select-String 'iniciando|grabando con|terminó|audio' | Select-Object -Last 30 | ForEach-Object { Write-Host "         log: $($_.Line)" }
  Check (-not (Get-Process ffmpeg -ErrorAction SilentlyContinue | Where-Object { $_.PriorityClass -notin 'BelowNormal', 'Idle' })) "ffmpeg corre con prioridad baja"
  Alive "arranque"
  $np0 = @(Get-Process notepad -ErrorAction SilentlyContinue).Count
  Cmd 42
  Check (Wait-Until { @(Get-Process notepad -ErrorAction SilentlyContinue).Count -gt $np0 } 10) "menú → Ver registro abre el Bloc de notas"
  Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force

  Step "2. Teclas con la ventana cerrada: F9 clip, F10 captura, F8 grabar/frenar"
  Start-Sleep -Seconds 18
  $n = (Files 'clip_*.mp4').Count
  Key 0x78
  Check-Video (Wait-New 'clip_*.mp4' $n 40) 10 "F9: el clip"
  $n = (Files 'shot_*.png').Count
  Key 0x79
  $f = Wait-New 'shot_*.png' $n 30
  Check ($f -and (Get-Item $f).Length -gt 1000) "F10: la captura"
  $n = (Files 'grabacion_*.mp4').Count
  Key 0x77
  Start-Sleep -Seconds 12
  Check ([bool](Get-Process ffmpeg -ErrorAction SilentlyContinue)) "F8: ffmpeg está grabando"
  Key 0x77
  Check-Video (Wait-New 'grabacion_*.mp4' $n 60) 7 "F8: la grabación de pantalla"
  Alive "teclas"

  Step "3. Grabar una sola ventana (Bloc de notas)"
  $np = Start-Process notepad.exe -PassThru
  [void](Wait-Until { $np.Refresh(); $np.MainWindowHandle -ne [IntPtr]::Zero } 15)
  $n = (Files 'grabacion_*.mp4').Count
  Cmd 99 ([long]$np.MainWindowHandle)
  Start-Sleep -Seconds 14
  Key 0x77
  Check-Video (Wait-New 'grabacion_*.mp4' $n 60) 6 "la grabación de una ventana"
  Get-Content $log | Select-String 'iniciando|terminó' | Select-Object -Last 8 | ForEach-Object { Write-Host "         log: $($_.Line)" }
  Alive "grabar ventana"

  function Menu-Round([string]$cuando) {
    Step "Menú con la ventana Mis clips $cuando"
    # después de una grabación con F8 el buffer vuelve solo; se espera a que junte 15 s
    Start-Sleep -Seconds 22
    $n = (Files 'clip_*.mp4').Count
    Cmd 1
    Check-Video (Wait-New 'clip_*.mp4' $n 40) 10 "menú → Guardar clip ($cuando)"
    $n = (Files 'shot_*.png').Count
    Cmd 2
    Check ([bool](Wait-New 'shot_*.png' $n 30)) "menú → Captura ($cuando)"
    Set-Clipboard -Value 'x'
    Cmd 3
    Start-Sleep -Seconds 1
    $cf = Clipboard-Files
    Check ($cf -match '\.mp4') "menú → Copiar último clip deja el archivo para pegar ($cuando): $cf"
    $n = (Files 'grabacion_*.mp4').Count
    Cmd 4
    Start-Sleep -Seconds 10
    Cmd 4
    Check-Video (Wait-New 'grabacion_*.mp4' $n 60) 5 "menú → Grabar pantalla completa ($cuando)"
    $n = (Files 'grabacion_*.mp4').Count
    Cmd 100
    Start-Sleep -Seconds 10
    Cmd 4
    Check ([bool](Wait-New 'grabacion_*.mp4' $n 60)) "menú → Grabar una ventana (la primera de la lista) ($cuando)"
    # opciones: cada una se guarda en config.ini y la app sigue andando
    foreach ($d in @(@(10, 15), @(12, 60), @(14, 300), @(11, 30), @(10, 15))) { Cmd $d[0]; Check ((Cfg 'seconds') -eq "$($d[1])") "duración $($d[1]) s ($cuando)" }
    foreach ($q in @(@(22, '480', '30'), @(21, '720', '60'), @(20, '720', '30'), @(23, '0', '30'))) { Cmd $q[0]; Check ((Cfg 'quality') -eq $q[1] -and (Cfg 'fps') -eq $q[2]) "calidad $($q[1])p $($q[2]) fps ($cuando)" }
    foreach ($bg in @(@(45, '0'), @(46, '1'), @(47, '2'))) { Cmd $bg[0]; Check ((Cfg 'bg') -eq $bg[1]) "clips en segundo plano = $($bg[1]) ($cuando)" }
    $m = Cfg 'mic'; Cmd 43; Check ((Cfg 'mic') -ne $m) "micrófono se cambia ($cuando)"; Cmd 43
    $m = Cfg 'sound'; Cmd 44; Check ((Cfg 'sound') -ne $m) "sonido se cambia ($cuando)"; Cmd 44
    $run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    $a0 = [bool](Get-ItemProperty $run -Name Jugadon -ErrorAction SilentlyContinue)
    Cmd 41
    $a1 = [bool](Get-ItemProperty $run -Name Jugadon -ErrorAction SilentlyContinue)
    Check ($a0 -ne $a1) "iniciar con Windows se cambia ($cuando)"
    Cmd 41
    # cambiar la tecla de clip a F7, probarla y volver a F9
    Cmd 30
    Start-Sleep -Milliseconds 500
    Key 0x76
    Start-Sleep -Seconds 1
    Check ((Cfg 'clip_key') -eq '118') "cambiar tecla de clip a F7 ($cuando)"
    Cmd 30
    Start-Sleep -Milliseconds 500
    Key 0x78
    Start-Sleep -Seconds 1
    Check ((Cfg 'clip_key') -eq '120') "volver la tecla de clip a F9 ($cuando)"
    Cmd 40
    Alive "menú $cuando"
  }

  Step "4. Todas las opciones del menú, con la ventana cerrada"
  Menu-Round "cerrada"

  Step "5. Ventana Mis clips"
  Cmd 5
  $script:gal = [IntPtr]::Zero
  [void](Wait-Until { $script:gal = [W]::FindWindow('jugadon_gallery', [NullString]::Value); $script:gal -ne [IntPtr]::Zero } 15)
  Check ($script:gal -ne [IntPtr]::Zero) "se abre la ventana"
  Start-Sleep -Seconds 5
  [void][W]::SetForegroundWindow($script:gal)
  Snap $script:gal 'galeria_todo.png'
  foreach ($nav in 241, 242, 243, 240) { GCmd $nav; Start-Sleep -Milliseconds 700; Snap $script:gal "galeria_seccion_$nav.png" }
  Check (Responding $script:gal) "la ventana responde al cambiar de sección"
  GCmd 210
  Set-Clipboard -Value 'x'
  GCmd 203
  Start-Sleep -Seconds 1
  Check ((Clipboard-Files) -match '\.(mp4|png)') "botón Copiar de la tarjeta"
  GCmd 207
  # Acortar: dejar los últimos 10 s de un clip (crea una copia)
  GCmd 241
  GCmd 210
  $n = (Files '*_ultimos10s.mp4').Count
  GCmd 250
  $f = Wait-New '*_ultimos10s.mp4' $n 30
  Check-Video $f 6 "Acortar → últimos 10 s"
  if ($f) { $p = Probe $f; Check ($p.Dur -le 13) "el clip acortado dura como mucho ~10 s" }
  GCmd 240
  # Tab recorre secciones y botones; Enter sobre "Capturas" cambia de sección
  foreach ($i in 1..4) { [void][W]::PostMessage($script:gal, 0x100, [IntPtr]0x09, [IntPtr]::Zero) }
  [void][W]::PostMessage($script:gal, 0x100, [IntPtr]0x0D, [IntPtr]::Zero)
  Start-Sleep -Seconds 1
  Snap $script:gal 'galeria_teclado.png'
  Check (Responding $script:gal) "Tab y Enter en la ventana"
  GCmd 240
  # tecla Abajo/Fin (moverse entre tarjetas)
  foreach ($vk in 0x28, 0x27, 0x23, 0x24) { [void][W]::PostMessage($script:gal, 0x100, [IntPtr]$vk, [IntPtr]::Zero) }
  Start-Sleep -Seconds 1
  Check (Responding $script:gal) "la ventana responde al teclado"
  Menu-Round "abierta"
  Check ([W]::IsWindow($script:gal)) "la ventana sigue abierta después de usar el menú"
  [void][W]::SetForegroundWindow($script:gal)
  Start-Sleep -Seconds 2
  Snap $script:gal 'galeria_final.png'
  [void][W]::PostMessage($script:gal, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
  Start-Sleep -Seconds 2
  Check (-not [W]::IsWindow($script:gal)) "la X cierra la ventana"
  Alive "cerrar la ventana"

  Step "6. Solo placa (en este servidor sin placa no tiene que grabar ni usar el procesador)"
  $b = Log-Count 'ningún encoder funcionó'
  Cmd 48
  Check ((Cfg 'gpu_only') -eq '1') "opción guardada"
  $gave = Wait-Until { (Log-Count 'ningún encoder funcionó') -gt $b } 120
  Check $gave "se rinde sin usar el procesador"
  Start-Sleep -Seconds 3
  $x264 = Get-CimInstance Win32_Process -Filter "Name='ffmpeg.exe'" | Where-Object { $_.CommandLine -match 'libx264' }
  Check (-not $x264) "no quedó x264 corriendo"
  $b = Log-Count 'grabando con'
  Cmd 48
  Check (Buffer-Running $b 90) "al desactivarla vuelve a grabar"
  Alive "solo placa"

  Step "6b. Volver a probar cómo grabar (menú)"
  $b = Log-Count 'prueba de grabación: se usa'
  Cmd 49
  Check (Wait-Until { (Log-Count 'prueba de grabación: se usa') -gt $b } 120) "menú → Volver a probar cómo grabar"
  $b = Log-Count 'grabando con'
  Check (Buffer-Running ($b - 1) 60) "después vuelve a grabar clips"
  Alive "volver a probar"

  Step "7. Corte de golpe en medio de una grabación (como un corte de luz)"
  Cmd 4
  Start-Sleep -Seconds 10
  Stop-Process -Id $script:proc.Id -Force
  Start-Sleep -Seconds 3
  Check (-not (Get-Process ffmpeg -ErrorAction SilentlyContinue)) "ffmpeg se cierra junto con la app"
  $ts = @(Get-ChildItem (Join-Path $env:TEMP 'jugadon_rec') -Filter *.ts -ErrorAction SilentlyContinue)
  Check ($ts.Count -gt 0) "quedó la grabación a medias en el disco"
  $n = (Files 'grabacion_recuperada_*.mp4').Count
  Start-App
  Check-Video (Wait-New 'grabacion_recuperada_*.mp4' $n 60) 4 "al volver a abrir, la grabación se recupera"
  Alive "recuperación"

  Step "8. Abrirla dos veces"
  $p2 = Start-Process -FilePath $exe -WorkingDirectory (Split-Path $exe) -PassThru
  Start-Sleep -Seconds 4
  Check (@(Get-Process jugadon).Count -eq 2 -and -not $p2.HasExited) "la segunda avisa que ya está abierta"
  Stop-Process -Id $p2.Id -Force -ErrorAction SilentlyContinue
  Alive "segunda instancia"

  Step "9. Salir"
  Cmd 50
  $gone = Wait-Until { $script:proc.HasExited } 15
  Check $gone "Salir cierra la app"
  Start-Sleep -Seconds 2
  Check (-not (Get-Process ffmpeg -ErrorAction SilentlyContinue)) "no queda ningún ffmpeg"
  $buf = @(Get-ChildItem (Join-Path $env:TEMP 'jugadon_buf') -ErrorAction SilentlyContinue)
  Check ($buf.Count -eq 0) "se borró el buffer temporal"
}
finally {
  Get-Process jugadon, ffmpeg, notepad -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Step "Archivos generados"
  Get-ChildItem $out | Format-Table Name, Length | Out-String | Write-Host
  Step "jugadon.log"
  Get-Content $log -ErrorAction SilentlyContinue | Write-Host
  Remove-Item $cfg -ErrorAction SilentlyContinue
}

Write-Host ""
if ($script:fails.Count -gt 0) {
  Write-Host "Fallaron $($script:fails.Count) pruebas:" -ForegroundColor Red
  $script:fails | ForEach-Object { Write-Host " - $_" }
  exit 1
}
Write-Host "Todas las pruebas pasaron." -ForegroundColor Green
exit 0
