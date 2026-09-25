// SPDX-License-Identifier: MIT

//! Synthetic loopback Responses endpoint for the `#148` bound/budget oracle.
//!
//! The endpoint's reply and hold time are test-controlled: `hold_millis` leaves a request
//! outstanding after it has been read in full, so the executor's own turn deadline is the only
//! thing that can end a held turn. `one_shot_wait` and `truncated_at_budget` are the two synthetic
//! replies this oracle needs — a terminal `wait` decision, and a reply the model's output budget
//! truncated so no decision exists to fabricate.
//!
//! Split out of `support/bounds.rs` so each helper file stays inside the package's preferred line
//! budget without an exemption; the writer-side and process helpers stay there and re-export this
//! module's model and replies.

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::Result;

/// A loopback Responses endpoint whose reply and hold time the test controls.
///
/// `hold_millis` is what makes the mid-turn deadline measurable: the request is read in full and
/// then left outstanding, so the executor's own deadline is the only thing that can end the turn.
pub struct Loopback {
    connections: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<Value>>>,
    reply: Arc<Mutex<(Value, u64)>>,
    holding: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    pub endpoint: String,
}

impl Loopback {
    pub fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let endpoint = format!("http://{}", listener.local_addr()?);
        let connections = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let reply = Arc::new(Mutex::new((one_shot_wait(), 0)));
        let holding = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (counted, recorded, answering, held, finished) = (
            connections.clone(),
            bodies.clone(),
            reply.clone(),
            holding.clone(),
            stop.clone(),
        );
        let worker = std::thread::spawn(move || {
            while !finished.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        counted.fetch_add(1, Ordering::SeqCst);
                        let _ = serve(&mut stream, &recorded, &answering, &held, &finished);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        Ok(Self {
            connections,
            bodies,
            reply,
            holding,
            stop,
            worker: Some(worker),
            endpoint,
        })
    }

    pub fn request_count(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    pub fn reset(&self) -> Result {
        self.connections.store(0, Ordering::SeqCst);
        self.bodies.lock().map_err(|_| "bodies poisoned")?.clear();
        Ok(())
    }

    /// Replaces the reply, optionally holding the request outstanding for `hold_millis`.
    ///
    /// Configuring a new reply also releases any request still being held, so a case that aborts a
    /// held turn does not make the next case wait out the abandoned hold.
    pub fn set_reply(&self, payload: Value, hold_millis: u64) -> Result {
        *self.reply.lock().map_err(|_| "reply poisoned")? = (payload, hold_millis);
        self.holding.store(hold_millis > 0, Ordering::SeqCst);
        Ok(())
    }

    /// The parsed request bodies the endpoint received, in arrival order: how a case proves the
    /// declared budget reached the wire instead of being dropped before dispatch.
    pub fn bodies(&self) -> Result<Vec<Value>> {
        Ok(self.bodies.lock().map_err(|_| "bodies poisoned")?.clone())
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The synthetic reply a real Exo turn accepts as one terminal `wait` decision.
pub fn one_shot_wait() -> Value {
    let text = json!({"decision": "wait", "rationale": "synthetic"}).to_string();
    json!({
        "id": "resp_synthetic", "object": "response", "created_at": 0,
        "status": "completed", "model": "o3-pro",
        "output": [{
            "id": "msg_synthetic", "type": "message", "status": "completed",
            "role": "assistant",
            "content": [{"type": "output_text", "text": text, "annotations": []}]
        }],
        "usage": {"input_tokens": 11, "output_tokens": 5, "total_tokens": 16}
    })
}

/// A provider reply the model budget truncated: no terminal decision and no Exo-side failure.
pub fn truncated_at_budget() -> Value {
    json!({
        "id": "resp_synthetic", "object": "response", "created_at": 0,
        "status": "incomplete", "model": "o3-pro",
        "incomplete_details": {"reason": "max_output_tokens"},
        "output": [{
            "id": "msg_synthetic", "type": "message", "status": "incomplete",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "", "annotations": []}]
        }],
        "usage": {"input_tokens": 11, "output_tokens": 0, "total_tokens": 11}
    })
}

fn serve(
    stream: &mut std::net::TcpStream,
    bodies: &Mutex<Vec<Value>>,
    reply: &Mutex<(Value, u64)>,
    holding: &AtomicBool,
    stop: &AtomicBool,
) -> Result {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        header.push(byte[0]);
        if header.len() > 16_384 {
            return Err("header bound".into());
        }
    }
    let header = String::from_utf8_lossy(&header).to_string();
    let length = header
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or("missing length")?;
    // The model side is not the admission boundary under test here: an executor handoff sitting
    // exactly on its own 160 KiB read bound projects to the model at about the same size, so this
    // endpoint needs headroom above that bound to answer the at-bound case at all.
    if length > 1024 * 1024 {
        return Err("request bound".into());
    }
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body)?;
    if let Ok(parsed) = serde_json::from_slice::<Value>(&body) {
        bodies.lock().map_err(|_| "bodies poisoned")?.push(parsed);
    }
    let (payload, hold_millis) = reply.lock().map_err(|_| "reply poisoned")?.clone();
    // Hold in bounded slices, so neither a released hold nor a dropped endpoint waits out the
    // abandoned request: a turn the executor aborted is not worth answering.
    let mut held = 0;
    while held < hold_millis && holding.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
        let slice = (hold_millis - held).min(50);
        std::thread::sleep(Duration::from_millis(slice));
        held += slice;
    }
    let bytes = serde_json::to_vec(&payload)?;
    let _ = write!(
        stream,
        "HTTP/1.1 200 Synthetic\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    let _ = stream.write_all(&bytes);
    Ok(())
}
