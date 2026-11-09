//! # pulse-dev
//!
//! The development server behind `pulse dev`, plus `pulse serve` for
//! previewing production builds.
//!
//! `pulse dev` loop:
//!
//! 1. Build the app with hot-reload hooks into `<cache>/dev`.
//! 2. Serve it, injecting a small client script into `index.html`.
//! 3. Poll the source tree; on change, rebuild and push an event over SSE:
//!    `reload` hot-swaps the WebAssembly module in place, keeping global
//!    signal state, `full-reload` refreshes the page (HTML changed), and
//!    `build-error` shows the compiler error in an overlay.
//!
//! Only the Rust standard library is used.

mod http;
mod reload;
mod watch;

use pulse_build::BuildOptions;
use reload::{Event, Hub};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{fs, thread};

/// Browser side of the dev server.
const CLIENT_JS: &str = include_str!("client.js");
const CLIENT_TAG: &str = r#"<script type="module" src="/__pulse/client.js"></script>"#;
const POLL_INTERVAL: Duration = Duration::from_millis(150);
/// Extra wait after the first change, so a save that touches several files
/// (or an editor's write-rename dance) results in one rebuild.
const DEBOUNCE: Duration = Duration::from_millis(60);

pub struct DevOptions {
    pub entry: PathBuf,
    pub port: u16,
    pub cache_dir: PathBuf,
    pub runtime_path: PathBuf,
}

/// Runs the dev server until the process is stopped.
pub fn dev(opts: DevOptions) -> Result<(), String> {
    let out_dir = opts.cache_dir.join("dev");
    let build_opts = BuildOptions {
        entry: opts.entry.clone(),
        out_dir: out_dir.clone(),
        cache_dir: opts.cache_dir.clone(),
        runtime_path: opts.runtime_path,
        release: true,
        hot: true,
    };
    let hub = Hub::default();

    let started = Instant::now();
    match pulse_build::build(&build_opts) {
        Ok(_) => log(&format!("built in {}", elapsed(started))),
        Err(e) => {
            eprintln!("{e}");
            hub.broadcast(Event::BuildError(e));
        }
    }

    let listener = http::bind(opts.port).map_err(|e| format!("cannot start server: {e}"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    log(&format!("dev server running at http://localhost:{}/  (Ctrl+C to stop)", addr.port()));

    let site = DevSite { out_dir, entry: opts.entry.clone() };
    spawn_server(listener, {
        let hub = hub.clone();
        move |stream| site.handle(stream, &hub)
    });

    let mut roots = vec![opts.entry.parent().unwrap_or(Path::new(".")).to_path_buf()];
    roots.extend(pulse_build::index_html_path(&opts.entry));
    let mut watcher = watch::Watcher::new(roots);
    let mut version = 0;
    loop {
        thread::sleep(POLL_INTERVAL);
        let mut changed = watcher.changes();
        if changed.is_empty() {
            continue;
        }
        thread::sleep(DEBOUNCE);
        changed.extend(watcher.changes());

        version += 1;
        let started = Instant::now();
        match pulse_build::build(&build_opts) {
            Ok(_) => {
                let html_changed = changed.iter().any(|p| p.extension().is_some_and(|e| e == "html"));
                let event = if html_changed { Event::FullReload } else { Event::Reload { version } };
                log(&format!(
                    "rebuilt in {} ({}) → {} browser(s)",
                    elapsed(started),
                    describe(&changed),
                    hub.client_count()
                ));
                hub.broadcast(event);
            }
            Err(e) => {
                eprintln!("{e}");
                hub.broadcast(Event::BuildError(e));
            }
        }
    }
}

/// Serves a directory of static files (e.g. a production build).
pub fn serve(dir: PathBuf, port: u16) -> Result<(), String> {
    if !dir.join("index.html").is_file() {
        return Err(format!("{} has no index.html; run `pulse build` first", dir.display()));
    }
    let listener = http::bind(port).map_err(|e| format!("cannot start server: {e}"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    log(&format!("serving {} at http://localhost:{}/  (Ctrl+C to stop)", dir.display(), addr.port()));
    spawn_server(listener, move |mut stream| {
        let Ok(Some(req)) = http::read_request(&stream) else { return };
        let _ = serve_file(&mut stream, &req, &dir);
    })
    .join()
    .map_err(|_| "server thread panicked".to_string())
}

/// Accepts connections on a background thread, one thread per connection.
fn spawn_server<F>(listener: TcpListener, handler: F) -> thread::JoinHandle<()>
where
    F: Fn(TcpStream) + Send + Sync + 'static,
{
    let handler = std::sync::Arc::new(handler);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handler = handler.clone();
            thread::spawn(move || handler(stream));
        }
    })
}

/// Request routing for `pulse dev`.
struct DevSite {
    out_dir: PathBuf,
    entry: PathBuf,
}

impl DevSite {
    fn handle(&self, mut stream: TcpStream, hub: &Hub) {
        let Ok(Some(req)) = http::read_request(&stream) else { return };
        let result = match req.path.as_str() {
            "/__pulse/events" => hub.stream(stream),
            "/__pulse/client.js" => http::respond(&mut stream, &req, 200, "text/javascript; charset=utf-8", CLIENT_JS.as_bytes()),
            // Always the loader this server was built with, even before the
            // first successful build has written one to disk.
            "/pulse.js" => {
                http::respond(&mut stream, &req, 200, "text/javascript; charset=utf-8", pulse_build::LOADER_JS.as_bytes())
            }
            "/" | "/index.html" => {
                let html = with_dev_client(&self.index_html());
                http::respond(&mut stream, &req, 200, "text/html; charset=utf-8", html.as_bytes())
            }
            _ => serve_file(&mut stream, &req, &self.out_dir),
        };
        // Errors here are browsers closing connections; nothing to do.
        let _ = result;
    }

    /// The built page, or the app's source page if no build succeeded yet.
    fn index_html(&self) -> String {
        fs::read_to_string(self.out_dir.join("index.html"))
            .ok()
            .or_else(|| pulse_build::index_html_path(&self.entry).and_then(|p| fs::read_to_string(p).ok()))
            .unwrap_or_else(|| pulse_build::DEFAULT_INDEX_HTML.to_string())
    }
}

fn serve_file(stream: &mut TcpStream, req: &http::Request, root: &Path) -> std::io::Result<()> {
    if req.method != "GET" && req.method != "HEAD" {
        return http::respond(stream, req, 405, "text/plain", b"method not allowed");
    }
    match http::resolve(root, &req.path).and_then(|p| fs::read(&p).ok().map(|body| (p, body))) {
        Some((path, body)) => http::respond(stream, req, 200, http::content_type(&path), &body),
        None => http::respond(stream, req, 404, "text/plain", b"not found"),
    }
}

/// Inserts the dev client script before `</body>` (or at the end).
fn with_dev_client(html: &str) -> String {
    match html.rfind("</body>") {
        Some(i) => format!("{}{CLIENT_TAG}\n{}", &html[..i], &html[i..]),
        None => format!("{html}\n{CLIENT_TAG}\n"),
    }
}

fn describe(changed: &[PathBuf]) -> String {
    let names: Vec<String> =
        changed.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
    names.join(", ")
}

fn elapsed(since: Instant) -> String {
    format!("{:.2}s", since.elapsed().as_secs_f64())
}

fn log(msg: &str) {
    eprintln!("[pulse] {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_client_before_body_end() {
        let html = with_dev_client("<html><body><div id=\"app\"></div></body></html>");
        assert!(html.ends_with(&format!("{CLIENT_TAG}\n</body></html>")), "{html}");
        assert!(with_dev_client("<p>bare</p>").contains(CLIENT_TAG));
    }
}
