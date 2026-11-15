//! Speech to text: record from the microphone, then turn it into text with whisper.cpp on this
//! computer. Nothing is sent anywhere except the one-time download of the model.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const MODEL_FILE: &str = "ggml-small.en.bin";
/// Pinned to a commit of the model repository, with the size and SHA-256 of the file there, so
/// a changed or cut-short download is never loaded.
const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small.en.bin";
const MODEL_SIZE: u64 = 487_614_201;
const MODEL_SHA256: &str = "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d";
/// What whisper expects.
const SAMPLE_RATE: u32 = 16_000;
/// Anything shorter is a mis-click, not speech.
const MIN_SECONDS: f32 = 0.4;

/// The open stream, the samples it fills (mono, at the microphone's own rate), that rate, and
/// the loudness of the latest sound (an `f32` between 0 and 1, stored as bits).
type Started = (cpal::Stream, Arc<Mutex<Vec<f32>>>, u32, Arc<AtomicU32>);

/// A running microphone: how to stop it, its thread, the samples it fills and their rate.
type Microphone = (Sender<()>, JoinHandle<()>, Arc<Mutex<Vec<f32>>>, u32);

/// How long between tries at writing down what has been said so far.
const LIVE_EVERY: Duration = Duration::from_millis(700);
/// After this much speech in one piece, what's been written is kept and a new piece begins, so a
/// long dictation doesn't get slower and slower.
const PIECE_SECONDS: usize = 20;
/// Quieter than this is silence: no point asking the model, which makes up words for noise.
const SILENCE: f32 = 0.01;

struct Recording {
    stop_mic: Sender<()>,
    mic: JoinHandle<()>,
    done: Arc<AtomicBool>,
    /// Writes the words down while the person talks, and returns all of them at the end.
    writer: JoinHandle<Result<String, String>>,
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether the file at `path` is exactly the pinned model.
fn model_is_intact(path: &std::path::Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    if file.metadata().map_or(true, |m| m.len() != MODEL_SIZE) {
        return false;
    }
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash).is_ok() && hex(&hash.finalize()) == MODEL_SHA256
}

/// Downloads the model the first time, telling the UI how far along it is.
fn ensure_model(app: &AppHandle, voice: &Voice) -> Result<PathBuf, String> {
    let path = model_path();
    let _one_at_a_time = voice.download.lock().expect("download lock");
    if path.exists() {
        if model_is_intact(&path) {
            return Ok(path);
        }
        // Damaged or replaced: fetch it again rather than load it.
        let _ = std::fs::remove_file(&path);
    }
    std::fs::create_dir_all(path.parent().expect("model folder")).map_err(|e| e.to_string())?;
    let response = ureq::get(MODEL_URL)
        .call()
        .map_err(|e| format!("Couldn't download the speech model: {e}"))?;
    let total = MODEL_SIZE;
    let partial = path.with_extension("part");
    let mut file = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
    let mut reader = response.into_reader();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut hash = Sha256::new();
    let (mut done, mut last) = (0u64, 0u64);
    loop {
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        hash.update(&buffer[..n]);
        done += n as u64;
        if done > total {
            break;
        }
        if done - last > total / 100 {
            last = done;
            let _ = app.emit("voice-model-progress", done * 100 / total);
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    if done != total || hex(&hash.finalize()) != MODEL_SHA256 {
        let _ = std::fs::remove_file(&partial);
        return Err("The speech model download was incomplete or didn't match; try again.".into());
    }
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
/// The thread also reports how loud it is, 20 times a second, as `voice-level`.
fn start_microphone(app: AppHandle) -> Result<Microphone, String> {
    let (stop, stopped) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(Arc<Mutex<Vec<f32>>>, u32), String>>();
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
            let level = Arc::new(AtomicU32::new(0));
            let (sink, loudness) = (samples.clone(), level.clone());
            let fail = |e| eprintln!("microphone: {e}");
            // Every format is turned into mono f32 here.
            let push = move |frames: &mut dyn Iterator<Item = f32>| {
                let mut out = sink.lock().expect("samples lock");
                let before = out.len();
                let mut frame = Vec::with_capacity(channels);
                for sample in frames {
                    frame.push(sample);
                    if frame.len() == channels {
                        out.push(frame.iter().sum::<f32>() / channels as f32);
                        frame.clear();
                    }
                }
                let added = &out[before..];
                if !added.is_empty() {
                    let rms =
                        (added.iter().map(|s| s * s).sum::<f32>() / added.len() as f32).sqrt();
                    loudness.store(rms.to_bits(), Ordering::Relaxed);
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
            Ok((stream, samples, rate, level))
        };
        match setup() {
            Err(error) => {
                let _ = ready_tx.send(Err(error));
            }
            Ok((stream, samples, rate, level)) => {
                let _ = ready_tx.send(Ok((samples, rate)));
                while let Err(mpsc::RecvTimeoutError::Timeout) =
                    stopped.recv_timeout(Duration::from_millis(50))
                {
                    let rms = f32::from_bits(level.swap(0, Ordering::Relaxed));
                    // Speech sits around 0.02 to 0.2: stretch that to fill the bar.
                    let _ = app.emit("voice-level", (rms * 8.0).sqrt().min(1.0));
                }
                drop(stream);
                let _ = app.emit("voice-level", 0.0f32);
            }
        }
    });
    let (samples, rate) = ready_rx
        .recv()
        .map_err(|_| "The microphone thread stopped".to_string())??;
    Ok((stop, thread, samples, rate))
}

fn quiet(samples: &[f32]) -> bool {
    samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs())) < SILENCE
}

/// What whisper writes for silence and noise, which isn't speech: `[BLANK_AUDIO]`, `(music)`.
fn without_noises(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn joined(before: &str, after: &str) -> String {
    match (before.is_empty(), after.is_empty()) {
        (true, _) => after.to_string(),
        (_, true) => before.to_string(),
        _ => format!("{before} {after}"),
    }
}

/// Runs while the person talks: every so often, writes down what's been said since the last
/// piece was kept and tells the page (`voice-partial`), then does it a last time when told
/// recording is over.
fn write_down(
    app: AppHandle,
    voice: Arc<Voice>,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    done: Arc<AtomicBool>,
) -> Result<String, String> {
    // The first time this downloads the model; the microphone keeps listening meanwhile.
    let model = load_model(&app, &voice).inspect_err(|error| {
        let _ = app.emit("voice-model-error", error.clone());
    })?;
    let mut state = model
        .create_state()
        .map_err(|e| format!("Speech recognition failed: {e}"))?;
    let (mut kept, mut from) = (String::new(), 0usize);
    loop {
        let finished = done.load(Ordering::SeqCst);
        let piece: Vec<f32> = samples.lock().expect("samples lock")[from..].to_vec();
        let seconds = piece.len() as f32 / rate as f32;
        let enough = seconds >= if finished { MIN_SECONDS } else { 1.0 };
        let text = if enough && !quiet(&piece) {
            without_noises(&transcribe(&mut state, &resample(&piece, rate), finished)?)
        } else {
            String::new()
        };
        let all = joined(&kept, &text);
        if finished {
            return Ok(all);
        }
        if enough {
            let _ = app.emit("voice-partial", &all);
        }
        if piece.len() > PIECE_SECONDS * rate as usize {
            kept = all;
            from += piece.len();
        }
        // Wait, but notice quickly when recording ends.
        let waited = std::time::Instant::now();
        while waited.elapsed() < LIVE_EVERY && !done.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

fn start_recording(app: AppHandle, voice: Arc<Voice>) -> Result<Recording, String> {
    let (stop_mic, mic, samples, rate) = start_microphone(app.clone())?;
    let done = Arc::new(AtomicBool::new(false));
    let writer = {
        let done = done.clone();
        std::thread::spawn(move || write_down(app, voice, samples, rate, done))
    };
    Ok(Recording {
        stop_mic,
        mic,
        done,
        writer,
    })
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

/// `careful` (the last pass) tries several readings and keeps the best; the live passes take the
/// first, to keep up with the speaker.
fn transcribe(
    state: &mut whisper_rs::WhisperState,
    samples: &[f32],
    careful: bool,
) -> Result<String, String> {
    let mut params = FullParams::new(if careful {
        SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0,
        }
    } else {
        SamplingStrategy::Greedy { best_of: 1 }
    });
    params.set_language(Some("en"));
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    params.set_no_context(true);
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

/// Starts recording. These commands are `async` so that opening the microphone (which can take
/// a second while the sound system wakes up) never freezes the window.
#[tauri::command]
pub async fn voice_start(
    app: AppHandle,
    voice: tauri::State<'_, Arc<Voice>>,
) -> Result<(), String> {
    let voice = voice.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut slot = voice.recording.lock().expect("recording lock");
        if slot.is_none() {
            *slot = Some(start_recording(app, voice.clone())?);
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Ends the recording; returns `Ok` with whatever the writer produced.
fn finish(recording: Recording) -> Result<String, String> {
    recording.done.store(true, Ordering::SeqCst);
    let _ = recording.stop_mic.send(());
    let _ = recording.mic.join();
    recording
        .writer
        .join()
        .map_err(|_| "Speech recognition crashed".to_string())?
}

/// Stops recording and returns everything that was said ("" if nothing was).
#[tauri::command]
pub async fn voice_stop(voice: tauri::State<'_, Arc<Voice>>) -> Result<String, String> {
    let voice = voice.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let recording = voice.recording.lock().expect("recording lock").take();
        recording.map_or(Ok(String::new()), finish)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Stops recording and throws it away (the window was closed mid-sentence).
#[tauri::command]
pub async fn voice_cancel(voice: tauri::State<'_, Arc<Voice>>) -> Result<(), String> {
    let voice = voice.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(recording) = voice.recording.lock().expect("recording lock").take() {
            let _ = finish(recording);
        }
    })
    .await
    .map_err(|e| e.to_string())
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
    fn noises_are_not_words() {
        assert_eq!(without_noises("[BLANK_AUDIO]"), "");
        assert_eq!(without_noises(" Hello (music) there  [x] "), "Hello there");
        assert_eq!(joined("one", ""), "one");
        assert_eq!(joined("one", "two"), "one two");
    }

    #[test]
    fn silence_is_not_sent_to_the_model() {
        assert!(quiet(&[0.001, -0.002, 0.0]));
        assert!(!quiet(&[0.001, -0.2]));
    }

    #[test]
    fn resampling_keeps_the_shape_of_a_ramp() {
        let ramp: Vec<f32> = (0..300).map(|i| i as f32 / 300.0).collect();
        let out = resample(&ramp, 48_000);
        assert_eq!(out.len(), 100);
        assert!(out.windows(2).all(|w| w[1] >= w[0]));
    }
}
