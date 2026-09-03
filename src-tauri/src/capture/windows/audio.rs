//! WASAPI audio capture (design.md §3.2.1).
//!
//! - system: render-device loopback (polling)
//! - mic:    capture device (polling)
//! - per-app: process loopback via ActivateAudioInterfaceAsync (Win10 2004+)

use super::{co_init, err, AudioCapture, Result};
use eztopaz_core::audio::{resample_stereo, AudioSink, MIC_ID};
use eztopaz_core::ipc_types::AudioSelection;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TARGET_RATE: u32 = 48_000;
const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x2;

pub fn start_audio(selection: &AudioSelection, sink: AudioSink) -> super::Result<AudioCapture> {
    let mut cap = AudioCapture::new(sink.clone());

    if selection.mode == "system" {
        let sink2 = sink.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = std::thread::Builder::new()
            .name("wasapi-system".into())
            .spawn(move || {
                if let Err(e) = run_system_loopback(sink2, stop2) {
                    eprintln!("system loopback ended: {e}");
                }
            })
            .map_err(err)?;
        cap.add(stop, handle);
    }

    if selection.mic.enabled {
        let sink2 = sink.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = std::thread::Builder::new()
            .name("wasapi-mic".into())
            .spawn(move || {
                if let Err(e) = run_mic(sink2, stop2) {
                    eprintln!("mic capture ended: {e}");
                }
            })
            .map_err(err)?;
        cap.add(stop, handle);
    }

    if selection.mode == "apps" {
        for app_id in &selection.apps {
            let Some(pid) = app_id.rsplit(':').next().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let sink2 = sink.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let stop2 = stop.clone();
            let handle = std::thread::Builder::new()
                .name(format!("wasapi-app-{pid}"))
                .spawn(move || {
                    if let Err(e) = run_process_loopback(pid, sink2, stop2) {
                        eprintln!("process loopback ({pid}) ended: {e}");
                    }
                })
                .map_err(err)?;
            cap.add(stop, handle);
        }
    }

    Ok(cap)
}

fn run_system_loopback(sink: AudioSink, stop: Arc<AtomicBool>) -> Result<()> {
    use windows::Win32::Media::Audio::{eMultimedia, eRender};
    co_init();
    unsafe {
        let enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator =
            windows::Win32::System::Com::CoCreateInstance(
                &windows::Win32::Media::Audio::MMDeviceEnumerator,
                None,
                windows::Win32::System::Com::CLSCTX_ALL,
            )
            .map_err(err)?;
        let dev = enumerator
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .map_err(err)?;
        let client: windows::Win32::Media::Audio::IAudioClient =
            dev.Activate(windows::Win32::System::Com::CLSCTX_ALL, None).map_err(err)?;
        let fmt = client.GetMixFormat().map_err(err)?;
        let (rate, channels) = ((*fmt).nSamplesPerSec, (*fmt).nChannels.max(1) as usize);
        wasapi_polling(
            client,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            "system".into(),
            sink,
            stop,
            fmt,
            rate,
            channels,
        )
    }
}

fn run_mic(sink: AudioSink, stop: Arc<AtomicBool>) -> Result<()> {
    use windows::Win32::Media::Audio::{eCapture, eMultimedia};
    co_init();
    unsafe {
        let enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator =
            windows::Win32::System::Com::CoCreateInstance(
                &windows::Win32::Media::Audio::MMDeviceEnumerator,
                None,
                windows::Win32::System::Com::CLSCTX_ALL,
            )
            .map_err(err)?;
        let dev = enumerator
            .GetDefaultAudioEndpoint(eCapture, eMultimedia)
            .map_err(err)?;
        let client: windows::Win32::Media::Audio::IAudioClient =
            dev.Activate(windows::Win32::System::Com::CLSCTX_ALL, None).map_err(err)?;
        let fmt = client.GetMixFormat().map_err(err)?;
        let (rate, channels) = ((*fmt).nSamplesPerSec, (*fmt).nChannels.max(1) as usize);
        wasapi_polling(client, 0, MIC_ID.into(), sink, stop, fmt, rate, channels)
    }
}

use windows::Win32::Media::Audio::AUDCLNT_STREAMFLAGS_LOOPBACK;

/// Shared-mode WASAPI capture (polling). System loopback passes
/// AUDCLNT_STREAMFLAGS_LOOPBACK, mic passes 0. `fmt` is the format passed to
/// Initialize (mix format for devices; our float48k format for process loopback).
unsafe fn wasapi_polling(
    client: windows::Win32::Media::Audio::IAudioClient,
    extra_flags: u32,
    id: String,
    sink: AudioSink,
    stop: Arc<AtomicBool>,
    fmt: *const windows::Win32::Media::Audio::WAVEFORMATEX,
    rate: u32,
    channels: usize,
) -> Result<()> {
    use windows::Win32::Media::Audio::{
        IAudioCaptureClient, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
        AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    };

    client
        .Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            extra_flags
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            1_000_000, // 100ms
            0,
            fmt,
            None,
        )
        .map_err(err)?;
    let capture: IAudioCaptureClient = client.GetService().map_err(err)?;
    client.Start().map_err(err)?;

    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = client.Stop();
            return Ok(());
        }
        let Ok(mut packet) = capture.GetNextPacketSize() else {
            return Ok(());
        };
        while packet > 0 {
            let mut ptr: *mut u8 = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            if capture.GetBuffer(&mut ptr, &mut frames, &mut flags, None, None).is_err() {
                break;
            }
            if frames > 0 {
                if (flags & AUDCLNT_BUFFERFLAGS_SILENT) == 0 && !ptr.is_null() {
                    let raw =
                        std::slice::from_raw_parts(ptr as *const f32, frames as usize * channels);
                    let mut stereo = Vec::with_capacity(frames as usize * 2);
                    for i in 0..frames as usize {
                        // Mono mics (channels == 1) duplicate the channel;
                        // >2 channels fold down to the first two.
                        let l = raw[i * channels];
                        let r = if channels >= 2 { raw[i * channels + 1] } else { l };
                        stereo.push(l);
                        stereo.push(r);
                    }
                    let block = resample_stereo(&stereo, rate, TARGET_RATE);
                    if !sink.push(&id, block) {
                        let _ = client.Stop();
                        return Ok(());
                    }
                } else if !sink.push(&id, vec![0.0; frames as usize * 2]) {
                    let _ = client.Stop();
                    return Ok(());
                }
            }
            let _ = capture.ReleaseBuffer(frames);
            packet = capture.GetNextPacketSize().unwrap_or(0);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

// --- per-app process loopback ------------------------------------------------

#[windows::core::implement(
    windows::Win32::Media::Audio::IActivateAudioInterfaceCompletionHandler,
    windows::Win32::System::Com::IAgileObject
)]
struct LoopbackActivation {
    result: Arc<Mutex<Option<windows::core::IUnknown>>>,
    ready: Arc<(Mutex<bool>, std::sync::Condvar)>,
}

impl windows::Win32::Media::Audio::IActivateAudioInterfaceCompletionHandler_Impl
    for LoopbackActivation
{
    fn ActivateCompleted(
        &self,
        activateoperation: Option<&windows::Win32::Media::Audio::IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        if let Some(op) = activateoperation {
            let mut hr = windows::core::HRESULT::default();
            let mut unk: Option<windows::core::IUnknown> = None;
            unsafe {
                let _ = op.GetActivateResult(&mut hr, &mut unk);
            }
            *self.result.lock().unwrap() = unk;
        }
        let (lock, cvar) = &*self.ready;
        let mut done = lock.lock().unwrap();
        *done = true;
        cvar.notify_all();
        Ok(())
    }
}

impl windows::Win32::System::Com::IAgileObject_Impl for LoopbackActivation {}

fn run_process_loopback(pid: u32, sink: AudioSink, stop: Arc<AtomicBool>) -> Result<()> {
    use windows::Win32::Media::Audio::{
        ActivateAudioInterfaceAsync, IActivateAudioInterfaceCompletionHandler, IAudioClient,
        AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, PROCESS_LOOPBACK_MODE,
    };
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::BLOB;
    use windows::core::IUnknown;
    use windows::Win32::System::Variant::VT_BLOB;

    const PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE: PROCESS_LOOPBACK_MODE =
        PROCESS_LOOPBACK_MODE(0);

    co_init();
    unsafe {
        let params = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: windows::Win32::Media::Audio::AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: pid,
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                },
            },
        };
        let blob = BLOB {
            cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            pBlobData: &params as *const _ as *mut u8,
        };
        let mut var = PROPVARIANT::default();
        var.Anonymous = windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0 {
                Anonymous: std::mem::ManuallyDrop::new(
                    windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0_0 {
                        vt: VT_BLOB,
                        wReserved1: 0,
                        wReserved2: 0,
                        wReserved3: 0,
                        Anonymous: windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0_0_0 {
                            blob,
                        },
                    },
                ),
        };

        let result: Arc<Mutex<Option<windows::core::IUnknown>>> = Arc::new(Mutex::new(None));
        let ready = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let handler: IActivateAudioInterfaceCompletionHandler = LoopbackActivation {
            result: result.clone(),
            ready: ready.clone(),
        }
        .into();

        ActivateAudioInterfaceAsync(
            windows::core::w!("VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK"),
            &<IAudioClient as windows::core::ComInterface>::IID,
            Some(&var),
            &handler,
        )
        .map_err(err)?;

        // wait for activation (max 3s)
        let (lock, cvar) = &*ready;
        let guard = lock.lock().map_err(err)?;
        let (_guard, _timeout) = cvar
            .wait_timeout_while(guard, Duration::from_secs(3), |done| !*done)
            .map_err(err)?;
        let unk = result
            .lock()
            .map_err(err)?
            .take()
            .ok_or_else(|| err("process loopback activation timed out"))?;

        let client: IAudioClient =
            <IUnknown as windows::core::ComInterface>::cast(&unk).map_err(err)?;

        // float32 48kHz stereo
        let format = windows::Win32::Media::Audio::WAVEFORMATEX {
            wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
            nChannels: 2,
            nSamplesPerSec: TARGET_RATE,
            nAvgBytesPerSec: TARGET_RATE * 8,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        wasapi_polling(
            client,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            format!("pid:{pid}"),
            sink,
            stop,
            &format,
            TARGET_RATE,
            2,
        )
    }
}
