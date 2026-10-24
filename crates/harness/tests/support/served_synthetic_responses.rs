// SPDX-License-Identifier: MIT

use serde::Deserialize;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use std::collections::BTreeMap;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::fixture::Result;
use super::http_contract::{header, read_request_with_body_limit, write_response};

const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
const PEER_LIFETIME: Duration = Duration::from_secs(60);
const CLEANUP_DEADLINE: Duration = Duration::from_secs(3);
const ACCEPT_POLL: Duration = Duration::from_millis(5);
const REQUEST_BODY_LIMIT: usize = 131_072;
const REQUEST_DEPTH_LIMIT: usize = 32;
const REQUEST_ITEM_LIMIT: usize = 32_768;
const RESPONSE_BODY: &[u8] = br#"{"id":"resp_synthetic","object":"response","created_at":0,"status":"completed","model":"o3-pro","output":[{"id":"msg_synthetic","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"{\"decision\":\"wait\",\"rationale\":\"synthetic\"}","annotations":[]}]}],"usage":{"input_tokens":11,"output_tokens":5,"total_tokens":16}}"#;

type WorkerResult = std::result::Result<(), &'static str>;

pub(crate) enum FinishFailure {
    Peer,
    Cleanup,
}
pub(crate) type FinishResult = std::result::Result<(), FinishFailure>;

pub(crate) struct Loopback {
    pub(crate) endpoint: String,
    request_count: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    result: Option<Receiver<WorkerResult>>,
    worker: Option<JoinHandle<()>>,
    cleanup_deadline: Option<Instant>,
}

impl Loopback {
    pub(crate) fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        if !address.ip().is_loopback() {
            return Err("Responses fixture did not bind numeric loopback".into());
        }
        listener.set_nonblocking(true)?;
        let endpoint = format!("http://{address}");
        let request_count = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_count = Arc::clone(&request_count);
        let worker_cancel = Arc::clone(&cancel);
        let deadline = Instant::now()
            .checked_add(PEER_LIFETIME)
            .ok_or("Responses peer deadline invalid")?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name(String::from("h391-synthetic-responses"))
            .spawn(move || {
                let outcome = serve(listener, &worker_count, &worker_cancel, deadline);
                let _ = sender.send(outcome);
            })?;
        Ok(Self {
            endpoint,
            request_count,
            cancel,
            result: Some(receiver),
            worker: Some(worker),
            cleanup_deadline: None,
        })
    }

    pub(crate) fn request_count(&self) -> usize {
        self.request_count.load(Ordering::Acquire)
    }

    pub(crate) fn result_class(&self, outcome: &FinishResult) -> &'static str {
        match outcome {
            Err(FinishFailure::Peer) => "peer_failed",
            Err(FinishFailure::Cleanup) => "cleanup_failed",
            Ok(()) => match self.request_count() {
                0 => "no_response",
                1 => "exactly_one_response",
                _ => "unexpected_response_count",
            },
        }
    }

    pub(crate) fn finish(&mut self) -> FinishResult {
        if self.worker.is_none() {
            return Ok(());
        }
        self.cancel.store(true, Ordering::Release);
        let deadline = match self.cleanup_deadline {
            Some(deadline) => deadline,
            None => {
                let deadline = Instant::now()
                    .checked_add(CLEANUP_DEADLINE)
                    .ok_or(FinishFailure::Cleanup)?;
                self.cleanup_deadline = Some(deadline);
                deadline
            }
        };
        let receiver = self.result.as_ref().ok_or(FinishFailure::Cleanup)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let outcome = match receiver.recv_timeout(remaining) {
            Ok(outcome) => outcome,
            Err(RecvTimeoutError::Disconnected) => {
                self.join_worker()?;
                return Err(FinishFailure::Cleanup);
            }
            Err(RecvTimeoutError::Timeout) => {
                self.join_worker()?;
                return Err(FinishFailure::Cleanup);
            }
        };
        self.result.take();
        self.join_worker()?;
        outcome.map_err(|_| FinishFailure::Peer)
    }

    fn join_worker(&mut self) -> FinishResult {
        let worker = self.worker.take().ok_or(FinishFailure::Cleanup)?;
        worker.join().map_err(|_| FinishFailure::Cleanup)?;
        Ok(())
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let _ = self.finish();
        }
    }
}

fn serve(
    listener: TcpListener,
    request_count: &AtomicUsize,
    cancel: &AtomicBool,
    peer_deadline: Instant,
) -> WorkerResult {
    let mut failure = None;
    loop {
        if cancel.load(Ordering::Acquire) {
            return failure.map_or(Ok(()), Err);
        }
        if Instant::now() >= peer_deadline {
            return Err(failure.unwrap_or("Responses accept lifetime elapsed"));
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                let count = request_count
                    .fetch_add(1, Ordering::AcqRel)
                    .saturating_add(1);
                if !peer.ip().is_loopback() {
                    retain_failure(&mut failure, "Responses peer address refused");
                    drop(stream);
                    continue;
                }
                if count != 1 {
                    retain_failure(&mut failure, "duplicate Responses connection observed");
                    drop(stream);
                    continue;
                }
                if let Err(code) = serve_request(stream, cancel, peer_deadline) {
                    retain_failure(&mut failure, code);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                pause_accept(peer_deadline, cancel)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(failure.unwrap_or("Responses accept failed")),
        }
    }
}

fn serve_request(
    mut stream: TcpStream,
    cancel: &AtomicBool,
    peer_deadline: Instant,
) -> std::result::Result<(), &'static str> {
    stream
        .set_nonblocking(true)
        .map_err(|_| "Responses socket setup failed")?;
    let deadline = Instant::now()
        .checked_add(REQUEST_DEADLINE)
        .ok_or("Responses request deadline invalid")?
        .min(peer_deadline);
    let request = read_request_with_body_limit(&mut stream, deadline, cancel, REQUEST_BODY_LIMIT)?;
    if request.path != "/responses"
        || header(&request.headers, "content-type")
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            != Some("application/json")
    {
        return Err("Responses route or content type refused");
    }
    let strict: StrictJson = serde_json::from_slice(&request.body)
        .map_err(|_| "Responses request JSON or duplicate key refused")?;
    let mut item_count = 0;
    if !strict.within_bounds(0, &mut item_count) {
        return Err("Responses request JSON depth or item bound exceeded");
    }
    let envelope: ResponsesRequest =
        serde_json::from_slice(&request.body).map_err(|_| "Responses request envelope refused")?;
    if envelope.model != "o3-pro"
        || !matches!(envelope.input, StrictJson::Array(_) | StrictJson::String(_))
        || envelope.extra.len() > 64
    {
        return Err("Responses request model envelope refused");
    }
    write_response(&mut stream, RESPONSE_BODY, deadline, cancel)
}

fn pause_accept(deadline: Instant, cancel: &AtomicBool) -> WorkerResult {
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    if Instant::now() >= deadline {
        return Err("Responses accept deadline elapsed");
    }
    thread::sleep(ACCEPT_POLL.min(deadline.saturating_duration_since(Instant::now())));
    Ok(())
}

fn retain_failure(slot: &mut Option<&'static str>, code: &'static str) {
    if slot.is_none() {
        *slot = Some(code);
    }
}

#[derive(Deserialize)]
struct ResponsesRequest {
    model: String,
    input: StrictJson,
    #[serde(flatten)]
    extra: BTreeMap<String, StrictJson>,
}

#[derive(Debug)]
enum StrictJson {
    Null,
    Boolean,
    Number(serde_json::Number),
    String(String),
    Array(Vec<StrictJson>),
    Object(BTreeMap<String, StrictJson>),
}

impl StrictJson {
    fn within_bounds(&self, depth: usize, count: &mut usize) -> bool {
        if depth > REQUEST_DEPTH_LIMIT {
            return false;
        }
        *count = count.saturating_add(1);
        if *count > REQUEST_ITEM_LIMIT {
            return false;
        }
        match self {
            Self::Null | Self::Boolean => true,
            Self::Number(number) => number.to_string().len() <= 64,
            Self::String(value) => value.len() <= REQUEST_BODY_LIMIT,
            Self::Array(values) => values
                .iter()
                .all(|value| value.within_bounds(depth + 1, count)),
            Self::Object(fields) => fields
                .iter()
                .all(|(key, value)| key.len() <= 256 && value.within_bounds(depth + 1, count)),
        }
    }
}

impl<'de> Deserialize<'de> for StrictJson {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictJsonVisitor)
    }
}

struct StrictJsonVisitor;

impl<'de> Visitor<'de> for StrictJsonVisitor {
    type Value = StrictJson;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a bounded JSON value without duplicate object keys")
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictJson::Null)
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictJson::Null)
    }

    fn visit_bool<E>(self, _value: bool) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictJson::Boolean)
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictJson::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictJson::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(StrictJson::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictJson::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictJson::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element()? {
            values.push(value);
        }
        Ok(StrictJson::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut fields = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            if fields.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON object key"));
            }
            fields.insert(key, map.next_value()?);
        }
        Ok(StrictJson::Object(fields))
    }
}
