//! A minimal HTTP/1.1 server toolkit for local development.
//!
//! It handles exactly what a dev server needs: `GET`/`HEAD` requests, static
//! files with correct MIME types (browsers require `application/wasm` for
//! streaming compilation) and one-response-per-connection semantics. It is not
//! meant to face the internet; production bundles are plain static files for
//! any real web server or CDN.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};

pub struct Request {
    pub method: String,
    /// Decoded path without the query string, e.g. `/app.wasm`.
    pub path: String,
}

/// Reads the request line and headers. Returns `None` for malformed or empty
/// requests (e.g. a browser's speculative connection that sent nothing).
pub fn read_request(stream: &TcpStream) -> io::Result<Option<Request>> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Ok(None);
    };
    // Drain headers; none are needed.
    let mut header = String::new();
    let mut total = 0;
    loop {
        header.clear();
        let n = reader.read_line(&mut header)?;
        total += n;
        if n == 0 || header == "\r\n" || header == "\n" || total > 64 * 1024 {
            break;
        }
    }
    let raw_path = target.split(['?', '#']).next().unwrap_or("/");
    Ok(Some(Request { method: method.to_string(), path: percent_decode(raw_path) }))
}

/// Writes a complete response and closes the exchange.
pub fn respond(stream: &mut TcpStream, req: &Request, status: u16, content_type: &str, body: &[u8]) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if req.method != "HEAD" {
        stream.write_all(body)?;
    }
    stream.flush()
}

pub fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Maps a URL path to a file under `root`, rejecting anything that would
/// escape it (`..`, absolute components). `/` maps to `index.html`.
pub fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    let relative = url_path.trim_start_matches('/');
    let relative = if relative.is_empty() { "index.html" } else { relative };
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(path)
}

/// Binds to `port` on localhost, or the next free port (up to 20 tries).
pub fn bind(port: u16) -> io::Result<TcpListener> {
    let mut last_err = None;
    for p in port..port.saturating_add(20) {
        match TcpListener::bind(("127.0.0.1", p)) {
            Ok(listener) => return Ok(listener),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::other("no port available")))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_inside_root_only() {
        let root = Path::new("/srv/app");
        assert_eq!(resolve(root, "/"), Some(PathBuf::from("/srv/app/index.html")));
        assert_eq!(resolve(root, "/assets/a.css"), Some(PathBuf::from("/srv/app/assets/a.css")));
        assert_eq!(resolve(root, "/../etc/passwd"), None);
        assert_eq!(resolve(root, "/a/../../x"), None);
    }

    #[test]
    fn mime_types() {
        assert_eq!(content_type(Path::new("app.wasm")), "application/wasm");
        assert_eq!(content_type(Path::new("x/pulse.js")), "text/javascript; charset=utf-8");
    }

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(percent_decode("/my%20file.txt"), "/my file.txt");
        assert_eq!(percent_decode("/100%"), "/100%");
    }
}
