//! Speech to text: record from the microphone, then turn it into text with whisper.cpp on this
//! computer. Nothing is sent anywhere except the one-time download of the model.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tauri::{AppHandle, Emitter};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const MODEL_FILE: &str = "ggml-base.en.bin";
const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";
/// What whisper expects.
const SAMPLE_RATE: u32 = 16_000;
/// Anything shorter is a mis-click, not speech.
const MIN_SECONDS: f32 = 0.4;

/// The open stream, the samples it fills, and the rate it records at.
type Started = (cpal::Stream, Arc<Mutex<Vec<f32>>>, u32);

struct Recording {
    stop: Sender<()>,
    thread: JoinHandle<Result<Vec<f32>, String>>,
}

#[derive(Default)]
pub struct Voice {
    recording: Mutex<Option<Recording>>,
    model: Mutex<Option<Arc<WhisperContext>>>,
    download: Mutex<()>,
}

fn model_path() -> PathBuf {
    directories::ProjectDirs::from("dev", "coccor", "FastMind")
        .map(|dirs| dirs.data_dir().join("models"))
        .unwrap_or_else(|| PathBuf::from("fastmind-models"))
        .join(MODEL_FILE)
}

/// Downloads the model the first time, telling the UI how far along it is.
fn ensure_model(app: &AppHandle, voice: &Voice) -> Result<PathBuf, String> {
    let path = model_path();
    let _one_at_a_time = voice.download.lock().expect("download lock");
    if path.exists() {
        return Ok(path);
    }
    std::fs::create_dir_all(path.parent().expect("model folder")).map_err(|e| e.to_string())?;
    let response = ureq::get(MODEL_URL)
        .call()
        .map_err(|e| format!("Couldn't download the speech model: {e}"))?;
    let total: u64 = response
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let partial = path.with_extension("part");
    let mut file = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
    let mut reader = response.into_reader();
    let mut buffer = vec![0u8; 256 * 1024];
    let (mut done, mut last) = (0u64, 0u64);
    loop {
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if total > 0 && done - last > total / 100 {
            last = done;
            let _ = app.emit("voice-model-progress", done * 100 / total);
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    let _ = app.emit("voice-model-progress", 100u64);
    Ok(path)
}

fn load_model(app: &AppHandle, voice: &Voice) -> Result<Arc<WhisperContext>, String> {
    if let Some(model) = voice.model.lock().expect("model lock").clone() {
        return Ok(model);
    }
    let path = ensure_model(app, voice)?;
    let context = WhisperContext::new_with_params(
        path.to_str().ok_or("The model path isn't valid text")?,
        WhisperContextParameters::default(),
    )
    .map_err(|e| format!("Couldn't load the speech model: {e}"))?;
    let context = Arc::new(context);
    *voice.model.lock().expect("model lock") = Some(context.clone());
    Ok(context)
}

/// Starts the microphone on its own thread (a `cpal` stream can't move between threads) and
/// returns once it's running, so a missing microphone is an error now, not after speaking.
fn start_recording() -> Result<Recording, String> {
    let (stop, stopped) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let thread = std::thread::spawn(move || {
        let setup = || -> Result<Started, String> {
            let device = cpal::default_host()
                .default_input_device()
                .ok_or("No microphone found")?;
            let config = device
                .default_input_config()
                .map_err(|e| format!("The microphone can't be used: {e}"))?;
            let channels = config.channels() as usize;
            let rate = config.sample_rate().0;
            let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
            let sink = samples.clone();
            let fail = |e| eprintln!("microphone: {e}");
            // Every format is turned into mono f32 here.
            let push = move |frames: &mut dyn Iterator<Item = f32>| {
                let mut out = sink.lock().expect("samples lock");
                let mut frame = Vec::with_capacity(channels);
                for sample in frames {
                    frame.push(sample);
                    if frame.len() == channels {
                        out.push(frame.iter().sum::<f32>() / channels as f32);
                        frame.clear();
                    }
                }
            };
            let stream = match config.sample_format() {
                cpal::SampleFormat::F32 => device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &_| push(&mut data.iter().copied()),
                    fail,
                    None,
                ),
                cpal::SampleFormat::I16 => device.build_input_stream(
                    &config.into(),
                    move |data: &[i16], _: &_| {
                        push(&mut data.iter().map(|s| *s as f32 / i16::MAX as f32))
                    },
                    fail,
                    None,
                ),
                cpal::SampleFormat::U16 => device.build_input_stream(
                    &config.into(),
                    move |data: &[u16], _: &_| {
                        push(&mut data.iter().map(|s| (*s as f32 - 32768.0) / 32768.0))
                    },
                    fail,
                    None,
                ),
                other => return Err(format!("Unsupported microphone format: {other:?}")),
            }
            .map_err(|e| format!("Couldn't open the microphone: {e}"))?;
            stream
                .play()
                .map_err(|e| format!("Couldn't start the microphone: {e}"))?;
            Ok((stream, samples, rate))
        };
        match setup() {
            Err(error) => {
                let _ = ready_tx.send(Err(error.clone()));
                Err(error)
            }
            Ok((stream, samples, rate)) => {
                let _ = ready_tx.send(Ok(()));
                let _ = stopped.recv();
                drop(stream);
                let mono = std::mem::take(&mut *samples.lock().expect("samples lock"));
                Ok(resample(&mono, rate))
            }
        }
    });
    ready_rx
        .recv()
        .map_err(|_| "The microphone thread stopped".to_string())??;
    Ok(Recording { stop, thread })
}

/// Linear resampling to 16 kHz: crude, but speech survives it.
fn resample(mono: &[f32], rate: u32) -> Vec<f32> {
    if rate == SAMPLE_RATE || mono.is_empty() {
        return mono.to_vec();
    }
    let ratio = rate as f64 / SAMPLE_RATE as f64;
    let length = (mono.len() as f64 / ratio) as usize;
    (0..length)
        .map(|i| {
            let at = i as f64 * ratio;
            let (index, fraction) = (at as usize, (at - at.floor()) as f32);
            let next = mono.get(index + 1).copied().unwrap_or(mono[index]);
            mono[index] * (1.0 - fraction) + next * fraction
        })
        .collect()
}

fn transcribe(model: &WhisperContext, samples: &[f32]) -> Result<String, String> {
    let mut state = model
        .create_state()
        .map_err(|e| format!("Speech recognition failed: {e}"))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    params.set_n_threads(std::thread::available_parallelism().map_or(4, |n| n.get().min(8)) as i32);
    state
        .full(params, samples)
        .map_err(|e| format!("Speech recognition failed: {e}"))?;
    let mut text = String::new();
    for segment in state.as_iter() {
        text.push_str(
            &segment
                .to_str_lossy()
                .map_err(|e| format!("Speech recognition failed: {e}"))?,
        );
    }
    Ok(text.trim().to_string())
}

/// Starts recording. Also begins fetching the model if it isn't here yet, so it's ready by the
/// time the person stops talking.
#[tauri::command]
pub fn voice_start(app: AppHandle, voice: tauri::State<'_, Arc<Voice>>) -> Result<(), String> {
    let mut slot = voice.recording.lock().expect("recording lock");
    if slot.is_some() {
        return Ok(());
    }
    *slot = Some(start_recording()?);
    if !model_path().exists() {
        let app = app.clone();
        std::thread::spawn(move || {
            use tauri::Manager;
            if let Err(error) = ensure_model(&app, &app.state::<Arc<Voice>>()) {
                let _ = app.emit("voice-model-error", error);
            }
        });
    }
    Ok(())
}

/// Stops recording and returns what was said ("" if nothing was).
#[tauri::command]
pub async fn voice_stop(
    app: AppHandle,
    voice: tauri::State<'_, Arc<Voice>>,
) -> Result<String, String> {
    let recording = voice.recording.lock().expect("recording lock").take();
    let Some(recording) = recording else {
        return Ok(String::new());
    };
    let _ = recording.stop.send(());
    let samples = recording
        .thread
        .join()
        .map_err(|_| "The microphone thread crashed".to_string())??;
    if (samples.len() as f32) < MIN_SECONDS * SAMPLE_RATE as f32 {
        return Ok(String::new());
    }
    let voice = voice.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let model = load_model(&app, &voice)?;
        transcribe(&model, &samples)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Stops recording and throws it away (the window was closed mid-sentence).
#[tauri::command]
pub fn voice_cancel(voice: tauri::State<'_, Arc<Voice>>) {
    if let Some(recording) = voice.recording.lock().expect("recording lock").take() {
        let _ = recording.stop.send(());
        let _ = recording.thread.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_the_length_in_seconds() {
        let one_second = vec![0.5f32; 48_000];
        assert_eq!(resample(&one_second, 48_000).len(), 16_000);
        assert_eq!(resample(&one_second, 16_000).len(), 48_000);
        assert!(resample(&[], 44_100).is_empty());
    }

    #[test]
    fn resampling_keeps_the_shape_of_a_ramp() {
        let ramp: Vec<f32> = (0..300).map(|i| i as f32 / 300.0).collect();
        let out = resample(&ramp, 48_000);
        assert_eq!(out.len(), 100);
        assert!(out.windows(2).all(|w| w[1] >= w[0]));
    }
}
