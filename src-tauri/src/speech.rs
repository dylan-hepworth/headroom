// Talking instead of typing (see Voice.tsx): macOS's own speech recognition, on the Mac itself where it can be, fed
// from the microphone through an audio engine. While it listens it reports how loud it's hearing, the words so far,
// which can still change, and at the end what was said.
//
// macOS asks the user, once each, to allow the microphone and speech recognition (see the usage descriptions in
// Info.plist, and the audio input entitlement a signed copy needs).

use std::{cell::RefCell, ptr::NonNull};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_avf_audio::{AVAudioEngine, AVAudioPCMBuffer, AVAudioTime};
use objc2_foundation::NSError;
use objc2_speech::{
    SFSpeechAudioBufferRecognitionRequest, SFSpeechRecognitionResult, SFSpeechRecognitionTask, SFSpeechRecognizer,
    SFSpeechRecognizerAuthorizationStatus,
};

/// What's heard, as it's heard.
pub enum Heard {
    /// How loud, from 0 to 1, about every 20 milliseconds
    Level(f32),
    /// The words so far
    Words(String),
    /// What was said, all of it, once the listening's stopped
    Done(String),
    Failed(String),
}

/// Listening now: kept on the main thread, which is where it's started and stopped.
struct Now {
    engine: Retained<AVAudioEngine>,
    request: Retained<SFSpeechAudioBufferRecognitionRequest>,
    task: Retained<SFSpeechRecognitionTask>,
}

thread_local! {
    static NOW: RefCell<Option<Now>> = const { RefCell::new(None) };
}

/// Has the user let Headroom use speech recognition? Asks them the first time, and says with `then`.
pub fn allowed(then: impl Fn(Result<(), String>) + 'static) {
    // SAFETY: a class method with no arguments
    let status = unsafe { SFSpeechRecognizer::authorizationStatus() };
    let said = |status: SFSpeechRecognizerAuthorizationStatus| match status {
        SFSpeechRecognizerAuthorizationStatus::Authorized => Ok(()),
        SFSpeechRecognizerAuthorizationStatus::Restricted => Err("Speech recognition isn't allowed on this Mac".into()),
        _ => Err("Headroom isn't allowed to recognize speech. Turn it on in System Settings → Privacy & Security → \
                  Speech Recognition."
            .into()),
    };
    if status != SFSpeechRecognizerAuthorizationStatus::NotDetermined {
        return then(said(status));
    }
    let block = RcBlock::new(move |status: SFSpeechRecognizerAuthorizationStatus| then(said(status)));
    // SAFETY: the block lives as long as macOS needs it; it copies it
    unsafe { SFSpeechRecognizer::requestAuthorization(&block) };
}

/// Start listening, on the main thread. What's heard goes to `heard`, from whichever thread macOS calls from.
pub fn start(heard: impl Fn(Heard) + Send + Sync + 'static) -> Result<(), String> {
    cancel();
    let heard = std::sync::Arc::new(heard);
    // SAFETY: the calls below are Speech and AVFAudio's documented ones, made with objects they returned
    unsafe {
        let recognizer = SFSpeechRecognizer::new();
        if !recognizer.isAvailable() {
            return Err("Speech recognition isn't available right now".into());
        }
        let request = SFSpeechAudioBufferRecognitionRequest::new();
        request.setShouldReportPartialResults(true);
        // Nothing leaves the Mac where it can be done on it
        if recognizer.supportsOnDeviceRecognition() {
            request.setRequiresOnDeviceRecognition(true);
        }

        let engine = AVAudioEngine::new();
        let input = engine.inputNode();
        let format = input.outputFormatForBus(0);
        let (fed, level) = (request.clone(), heard.clone());
        let tap = RcBlock::new(move |buffer: NonNull<AVAudioPCMBuffer>, _: NonNull<AVAudioTime>| {
            let buffer = buffer.as_ref();
            fed.appendAudioPCMBuffer(buffer);
            level(Heard::Level(loudness(buffer)));
        });
        input.installTapOnBus_bufferSize_format_block(0, 1024, Some(&format), RcBlock::as_ptr(&tap));
        engine.prepare();
        if let Err(e) = engine.startAndReturnError() {
            input.removeTapOnBus(0);
            return Err(format!("Couldn't start the microphone: {}", e.localizedDescription()));
        }

        let told = heard.clone();
        let handler = RcBlock::new(move |result: *mut SFSpeechRecognitionResult, error: *mut NSError| {
            if let Some(result) = result.as_ref() {
                let text = result.bestTranscription().formattedString().to_string();
                told(if result.isFinal() { Heard::Done(text) } else { Heard::Words(text) });
            } else if let Some(error) = error.as_ref() {
                told(Heard::Failed(error.localizedDescription().to_string()));
            }
        });
        let task = recognizer.recognitionTaskWithRequest_resultHandler(&request, &handler);
        NOW.with(|now| *now.borrow_mut() = Some(Now { engine, request, task }));
    }
    Ok(())
}

/// Stop listening, on the main thread. What was said comes as `Heard::Done` a moment later.
pub fn stop() {
    NOW.with(|now| {
        if let Some(now) = now.borrow().as_ref() {
            // SAFETY: stopping the engine and request this started
            unsafe {
                now.engine.stop();
                now.engine.inputNode().removeTapOnBus(0);
                now.request.endAudio();
            }
        }
    });
}

/// Stop listening, and forget what was said.
pub fn cancel() {
    NOW.with(|now| {
        if let Some(now) = now.borrow_mut().take() {
            // SAFETY: as in `stop`
            unsafe {
                now.engine.stop();
                now.engine.inputNode().removeTapOnBus(0);
                now.task.cancel();
            }
        }
    });
}

/// How loud a buffer is, from 0 to 1: the root mean square of its first channel, scaled so talking at a normal volume
/// fills most of the range.
fn loudness(buffer: &AVAudioPCMBuffer) -> f32 {
    // SAFETY: the buffer's first channel has `frameLength` samples, for as long as the tap block runs
    unsafe {
        let channels = buffer.floatChannelData();
        let frames = buffer.frameLength() as usize;
        if channels.is_null() || frames == 0 {
            return 0.0;
        }
        let samples = std::slice::from_raw_parts((*channels).as_ptr(), frames);
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / frames as f32).sqrt();
        (rms * 8.0).min(1.0)
    }
}
