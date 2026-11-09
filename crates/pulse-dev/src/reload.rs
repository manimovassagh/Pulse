//! Pushes build events to connected browsers over Server-Sent Events.
//!
//! SSE is plain HTTP, reconnects automatically, and needs no WebSocket
//! handshake or framing code, which makes it the simplest reliable channel
//! for one-way "something changed" notifications.

use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Something the browser should react to.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A new `app.wasm` is ready; hot-swap it (and refresh `app.css` if the
    /// app has styles).
    Reload { version: u64, css: bool },
    /// Only stylesheets changed; refresh `app.css`, keep the running module.
    CssUpdate { version: u64 },
    /// The page itself changed (`index.html`); reload it.
    FullReload,
    /// The build failed; show the message in an overlay.
    BuildError(String),
}

impl Event {
    fn to_sse(&self) -> String {
        match self {
            Event::Reload { version, css } => {
                format!("event: reload\ndata: {{\"version\":{version},\"css\":{css}}}\n\n")
            }
            Event::CssUpdate { version } => format!("event: css-update\ndata: {{\"version\":{version}}}\n\n"),
            Event::FullReload => "event: full-reload\ndata: {}\n\n".to_string(),
            Event::BuildError(msg) => {
                format!("event: build-error\ndata: {{\"message\":{}}}\n\n", json_string(msg))
            }
        }
    }
}

/// Fan-out of events to all connected browsers.
#[derive(Clone, Default)]
pub struct Hub {
    inner: Arc<Mutex<HubState>>,
}

#[derive(Default)]
struct HubState {
    clients: Vec<Sender<Event>>,
    /// Sent to browsers that connect while the build is broken.
    current_error: Option<String>,
}

impl Hub {
    pub fn broadcast(&self, event: Event) {
        let mut state = self.inner.lock().unwrap();
        state.current_error = match &event {
            Event::BuildError(msg) => Some(msg.clone()),
            _ => None,
        };
        state.clients.retain(|c| c.send(event.clone()).is_ok());
    }

    pub fn client_count(&self) -> usize {
        self.inner.lock().unwrap().clients.len()
    }

    fn subscribe(&self) -> Receiver<Event> {
        let (tx, rx) = mpsc::channel();
        let mut state = self.inner.lock().unwrap();
        if let Some(msg) = &state.current_error {
            let _ = tx.send(Event::BuildError(msg.clone()));
        }
        state.clients.push(tx);
        rx
    }

    /// Serves one SSE connection until the browser goes away.
    pub fn stream(&self, mut stream: TcpStream) -> io::Result<()> {
        stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\
              Connection: keep-alive\r\n\r\n: connected\n\n",
        )?;
        let events = self.subscribe();
        loop {
            let chunk = match events.recv_timeout(Duration::from_secs(15)) {
                Ok(event) => event.to_sse(),
                // A comment line keeps proxies happy and detects closed tabs.
                Err(RecvTimeoutError::Timeout) => ": ping\n\n".to_string(),
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            };
            stream.write_all(chunk.as_bytes())?;
            stream.flush()?;
        }
    }
}

/// Encodes `s` as a JSON string literal.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_encoding() {
        assert_eq!(
            Event::Reload { version: 3, css: true }.to_sse(),
            "event: reload\ndata: {\"version\":3,\"css\":true}\n\n"
        );
        assert_eq!(Event::CssUpdate { version: 4 }.to_sse(), "event: css-update\ndata: {\"version\":4}\n\n");
        assert_eq!(
            Event::BuildError("bad \"x\"\n  ^".into()).to_sse(),
            "event: build-error\ndata: {\"message\":\"bad \\\"x\\\"\\n  ^\"}\n\n"
        );
    }

    #[test]
    fn late_subscribers_get_the_current_error() {
        let hub = Hub::default();
        hub.broadcast(Event::BuildError("oops".into()));
        let rx = hub.subscribe();
        assert_eq!(rx.try_recv(), Ok(Event::BuildError("oops".into())));
        hub.broadcast(Event::Reload { version: 2, css: false });
        assert_eq!(rx.try_recv(), Ok(Event::Reload { version: 2, css: false }));
        assert!(hub.subscribe().try_recv().is_err(), "error cleared after a good build");
    }

    #[test]
    fn disconnected_clients_are_dropped() {
        let hub = Hub::default();
        drop(hub.subscribe());
        let _alive = hub.subscribe();
        hub.broadcast(Event::FullReload);
        assert_eq!(hub.client_count(), 1);
    }
}
