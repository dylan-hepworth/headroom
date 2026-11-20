// Talking instead of typing, on Windows: not there yet. The calls are the same as on a Mac (see speech.rs), so the
// panel can say why rather than Headroom leaving them out.

/// What's heard, as it's heard.
#[allow(dead_code)]
pub enum Heard {
    Level(f32),
    Words(String),
    Done(String),
    Failed(String),
}

const NOT_YET: &str = "Talking to Claude isn't available on Windows yet";

pub fn allowed(then: impl Fn(Result<(), String>) + 'static) {
    then(Err(NOT_YET.into()));
}

pub fn start(_heard: impl Fn(Heard) + Send + Sync + 'static) -> Result<(), String> {
    Err(NOT_YET.into())
}

pub fn stop() {}

pub fn cancel() {}
