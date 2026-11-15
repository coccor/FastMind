//! Speech to text: record from the microphone and turn it into text, as you talk, with NVIDIA's
//! Nemotron streaming model running on this computer through sherpa-onnx. Nothing is sent
//! anywhere except the one-time download of the model.

use bzip2::read::BzDecoder;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use sha2::{Digest, Sha256};
use sherpa_onnx::{OnlineModelConfig, OnlineRecognizer, OnlineRecognizerConfig};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// The model: `sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8`, English, which
/// writes punctuation and capitals as it goes. It's downloaded from the sherpa-onnx releases as
/// one archive, pinned by the size and SHA-256 GitHub publishes for that file, and unpacked
/// into the app's data folder. NVIDIA's model licence applies to it.
const MODEL_NAME: &str = "sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25";
const ARCHIVE_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25.tar.bz2";
const ARCHIVE_SIZE: u64 = 463_945_051;
const ARCHIVE_SHA256: &str = "78e2b79fcf7271553a74402a76b771b09ea40117a39566a79f52235b23db6358";
/// The files the recogniser needs, and their sizes (how an installed model is recognised).
const MODEL_FILES: [(&str, u64); 4] = [
    ("encoder.int8.onnx", 652_916_849),
    ("decoder.int8.onnx", 7_257_753),
    ("joiner.int8.onnx", 1_735_862),
    ("tokens.txt", 8_952),
];
/// Two threads were as fast as four on a 4-core laptop, and leave the window responsive.
const THREADS: i32 = 2;
/// How often the words so far are looked at while talking.
const LIVE_EVERY: Duration = Duration::from_millis(100);
/// Silence fed in after the last word so the model finishes what it's holding back.
const TAIL_SECONDS: f32 = 0.8;

/// The open stream, the samples it fills (mono, at the microphone's own rate), that rate, and
/// the loudness of the latest sound (an `f32` between 0 and 1, stored as bits).
type Started = (cpal::Stream, Arc<Mutex<Vec<f32>>>, u32, Arc<AtomicU32>);

/// A running microphone: how to stop it, its thread, the samples it fills and their rate.
type Microphone = (Sender<()>, JoinHandle<()>, Arc<Mutex<Vec<f32>>>, u32);

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
    recognizer: Mutex<Option<Arc<OnlineRecognizer>>>,
    /// One download or load at a time.
    setup: Mutex<()>,
}

fn models_dir() -> PathBuf {
    directories::ProjectDirs::from("dev", "coccor", "FastMind")
        .map(|dirs| dirs.data_dir().join("models"))
        .unwrap_or_else(|| PathBuf::from("fastmind-models"))
}

fn model_dir() -> PathBuf {
    models_dir().join(MODEL_NAME)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether the model is unpacked, every file at its size. The files only appear after the
/// archive they came from matched its pinned SHA-256 (see `ensure_model`).
fn model_is_present(dir: &std::path::Path) -> bool {
    MODEL_FILES
        .iter()
        .all(|(name, size)| std::fs::metadata(dir.join(name)).is_ok_and(|m| m.len() == *size))
}

/// Downloads the model archive the first time, checking it against the pinned size and SHA-256
/// while it arrives, then unpacks just the files the recogniser uses.
fn ensure_model(
    folder: &std::path::Path,
    mut progress: impl FnMut(u64),
) -> Result<PathBuf, String> {
    let dir = folder.join(MODEL_NAME);
    if model_is_present(&dir) {
        return Ok(dir);
    }
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let archive = folder.join(format!("{MODEL_NAME}.tar.bz2.part"));
    let response = ureq::get(ARCHIVE_URL)
        .call()
        .map_err(|e| format!("Couldn't download the speech model: {e}"))?;
    let mut file = std::fs::File::create(&archive).map_err(|e| e.to_string())?;
    let mut reader = response.into_reader();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut hash = Sha256::new();
    let (mut done, mut last) = (0u64, 0u64);
    let downloaded = (|| -> Result<(), String> {
        loop {
            let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            hash.update(&buffer[..n]);
            done += n as u64;
            if done > ARCHIVE_SIZE {
                break;
            }
            if done - last > ARCHIVE_SIZE / 100 {
                last = done;
                progress(done * 100 / ARCHIVE_SIZE);
            }
        }
        file.flush().map_err(|e| e.to_string())
    })();
    drop(file);
    let result = downloaded.and_then(|()| {
        if done != ARCHIVE_SIZE || hex(&hash.finalize()) != ARCHIVE_SHA256 {
            return Err(
                "The speech model download was incomplete or didn't match; try again.".into(),
            );
        }
        unpack_model(&archive, &dir)
    });
    let _ = std::fs::remove_file(&archive);
    result?;
    progress(100);
    Ok(dir)
}

/// Takes the model files out of the archive by name (nothing else, and never by a path from
/// the archive), into `dir`.
fn unpack_model(archive: &std::path::Path, dir: &std::path::Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(BzDecoder::new(file));
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let name = entry
            .path()
            .map_err(|e| e.to_string())?
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !MODEL_FILES.iter().any(|(wanted, _)| *wanted == name) {
            continue;
        }
        let partial = dir.join(format!("{name}.part"));
        let mut out = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
        std::fs::rename(&partial, dir.join(&name)).map_err(|e| e.to_string())?;
    }
    if model_is_present(dir) {
        Ok(())
    } else {
        Err("The speech model archive didn't contain what was expected.".into())
    }
}

fn create_recognizer(dir: &std::path::Path) -> Result<OnlineRecognizer, String> {
    let file = |name: &str| Some(dir.join(name).to_string_lossy().into_owned());
    let mut model_config = OnlineModelConfig {
        tokens: file("tokens.txt"),
        num_threads: THREADS,
        ..Default::default()
    };
    model_config.transducer.encoder = file("encoder.int8.onnx");
    model_config.transducer.decoder = file("decoder.int8.onnx");
    model_config.transducer.joiner = file("joiner.int8.onnx");
    let config = OnlineRecognizerConfig {
        model_config,
        decoding_method: Some("greedy_search".into()),
        ..Default::default()
    };
    OnlineRecognizer::create(&config).ok_or_else(|| {
        "Couldn't load the speech model; delete the models folder and try again.".into()
    })
}

/// The recogniser, loaded once (about 5 seconds the first time) and kept.
fn load_recognizer(app: &AppHandle, voice: &Voice) -> Result<Arc<OnlineRecognizer>, String> {
    let _one_at_a_time = voice.setup.lock().expect("setup lock");
    if let Some(recognizer) = voice.recognizer.lock().expect("recognizer lock").clone() {
        return Ok(recognizer);
    }
    let dir = ensure_model(&models_dir(), |percent| {
        let _ = app.emit("voice-model-progress", percent);
    })?;
    let recognizer = Arc::new(create_recognizer(&dir)?);
    *voice.recognizer.lock().expect("recognizer lock") = Some(recognizer.clone());
    Ok(recognizer)
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

/// Tells the stream the speech is over. The model holds back its last words until it hears a
/// little more, so it gets a moment of silence first. A stream keeps the sample rate of the
/// first audio it was given, and sherpa-onnx *exits the whole process* if a later chunk says
/// otherwise: the silence must use the same `rate` as everything before it.
fn end_of_speech(stream: &sherpa_onnx::OnlineStream, rate: u32) {
    stream.accept_waveform(
        rate as i32,
        &vec![0.0; (rate as f32 * TAIL_SECONDS) as usize],
    );
    stream.input_finished();
}

/// Runs while the person talks: feeds what the microphone hears to the model and tells the page
/// the words so far (`voice-partial`) whenever they change, then lets the model finish what it
/// is holding back once recording is over and returns all the words.
fn write_down(
    app: AppHandle,
    voice: Arc<Voice>,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    done: Arc<AtomicBool>,
) -> Result<String, String> {
    // The first time this downloads the model; the microphone keeps listening meanwhile and the
    // words catch up (the model runs about twice as fast as speech).
    let recognizer = load_recognizer(&app, &voice).inspect_err(|error| {
        let _ = app.emit("voice-model-error", error.clone());
    })?;
    let stream = recognizer.create_stream();
    let (mut fed, mut shown) = (0usize, String::new());
    loop {
        let finished = done.load(Ordering::SeqCst);
        let heard: Vec<f32> = {
            let all = samples.lock().expect("samples lock");
            all[fed..].to_vec()
        };
        fed += heard.len();
        if !heard.is_empty() {
            stream.accept_waveform(rate as i32, &heard);
        }
        if finished {
            end_of_speech(&stream, rate);
        }
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }
        let text = recognizer
            .get_result(&stream)
            .map(|result| result.text.trim().to_string())
            .unwrap_or_default();
        if finished {
            return Ok(text);
        }
        if text != shown {
            let _ = app.emit("voice-partial", &text);
            shown = text;
        }
        let waited = std::time::Instant::now();
        while waited.elapsed() < LIVE_EVERY && !done.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(20));
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

/// Ends the recording and returns everything that was said.
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

/// Loads the model ahead of time (when the capture window opens), so the first words don't wait
/// for it. Does nothing if the model hasn't been downloaded: that happens on the first use.
#[tauri::command]
pub async fn voice_warm(app: AppHandle, voice: tauri::State<'_, Arc<Voice>>) -> Result<(), String> {
    let voice = voice.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if model_is_present(&model_dir()) {
            let _ = load_recognizer(&app, &voice);
        }
    })
    .await
    .map_err(|e| e.to_string())
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
    fn the_model_files_are_listed_with_sizes() {
        assert_eq!(MODEL_FILES.len(), 4);
        assert!(
            MODEL_FILES
                .iter()
                .all(|(name, size)| !name.is_empty() && *size > 0)
        );
        assert_eq!(ARCHIVE_SHA256.len(), 64);
        assert!(ARCHIVE_URL.ends_with(&format!("{MODEL_NAME}.tar.bz2")));
    }

    #[test]
    fn unpacking_takes_only_the_model_files_by_name() {
        let work = tempfile::tempdir().unwrap();
        let archive = work.path().join("m.tar.bz2");
        {
            let encoder = bzip2::write::BzEncoder::new(
                std::fs::File::create(&archive).unwrap(),
                bzip2::Compression::fast(),
            );
            let mut tar = tar::Builder::new(encoder);
            for (path, bytes) in [
                (format!("{MODEL_NAME}/encoder.int8.onnx"), b"e".as_slice()),
                (format!("{MODEL_NAME}/tokens.txt"), b"t".as_slice()),
                (format!("{MODEL_NAME}/README.md"), b"readme".as_slice()),
                (format!("{MODEL_NAME}/test_wavs/0.wav"), b"wav".as_slice()),
            ] {
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                tar.append_data(&mut header, path, bytes).unwrap();
            }
            tar.into_inner().unwrap().finish().unwrap();
        }
        let dir = work.path().join("out");
        // Not the real sizes, so the final check complains, but the right files are the only
        // ones taken.
        assert!(unpack_model(&archive, &dir).is_err());
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["encoder.int8.onnx", "tokens.txt"]);
    }

    /// The real thing, end to end: download, check, unpack, load, and hear a recording.
    /// `FASTMIND_TEST_WAV=/path/to.wav cargo test -p fastmind-desktop -- --ignored --nocapture`
    #[test]
    #[ignore = "downloads 460 MB"]
    fn the_real_model_downloads_and_hears_speech() {
        let folder = tempfile::tempdir().unwrap();
        let mut last = 0;
        let dir = ensure_model(folder.path(), |percent| last = percent).unwrap();
        assert_eq!(last, 100);
        assert!(model_is_present(&dir));
        // Already there: no second download.
        assert_eq!(
            ensure_model(folder.path(), |_| panic!("downloaded again")).unwrap(),
            dir
        );

        let recognizer = create_recognizer(&dir).unwrap();
        let Ok(path) = std::env::var("FASTMIND_TEST_WAV") else {
            return;
        };
        let wave = sherpa_onnx::Wave::read(&path).expect("a readable wav");
        let stream = recognizer.create_stream();
        let began = std::time::Instant::now();
        for chunk in wave.samples().chunks(wave.sample_rate() as usize / 10) {
            stream.accept_waveform(wave.sample_rate(), chunk);
            while recognizer.is_ready(&stream) {
                recognizer.decode(&stream);
            }
        }
        end_of_speech(&stream, wave.sample_rate() as u32);
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }
        let text = recognizer.get_result(&stream).unwrap().text;
        eprintln!("HEARD ({:?}): {text}", began.elapsed());
        assert!(!text.trim().is_empty());

        // Microphones rarely record at 16 kHz. A stream fed at 44.1 kHz (as the app does) must
        // also finish cleanly: getting this wrong ends the whole process.
        let up: Vec<f32> = (0..wave.samples().len() * 441 / 160)
            .map(|i| wave.samples()[(i * 160 / 441).min(wave.samples().len() - 1)])
            .collect();
        let stream = recognizer.create_stream();
        for chunk in up.chunks(4410) {
            stream.accept_waveform(44_100, chunk);
            while recognizer.is_ready(&stream) {
                recognizer.decode(&stream);
            }
        }
        end_of_speech(&stream, 44_100);
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }
        let text = recognizer.get_result(&stream).unwrap().text;
        eprintln!("HEARD at 44.1 kHz: {text}");
        assert!(!text.trim().is_empty());
    }
}
