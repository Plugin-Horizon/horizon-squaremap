//! The internal web server: the frontend and tiles from the web folder, the JSON documents
//! from memory (squaremap's `EmbeddedWebServer` and `WebJsonStore`). GET and HEAD only, one
//! short-lived thread per connection, at most [`MAX_CONNECTIONS`] at once.
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub const MAX_CONNECTIONS: usize = 32;

/// JSON documents by URL path (`/tiles/players.json`).
#[derive(Default)]
pub struct JsonStore {
    docs: Mutex<HashMap<String, String>>,
}

impl JsonStore {
    pub fn put(&self, path: &str, json: String) {
        self.docs.lock().unwrap().insert(path.to_owned(), json);
    }
    pub fn get(&self, path: &str) -> Option<String> {
        self.docs.lock().unwrap().get(path).cloned()
    }
    /// Writes every document into the web folder (on shutdown, or with the server off).
    pub fn flush(&self, web: &Path) -> io::Result<()> {
        for (path, json) in self.docs.lock().unwrap().iter() {
            let file = web.join(path.trim_start_matches('/'));
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let tmp = file.with_extension("json.tmp");
            std::fs::write(&tmp, json)?;
            std::fs::rename(tmp, file)?;
        }
        Ok(())
    }
}

pub struct WebServer {
    stop: Arc<AtomicBool>,
    addr: std::net::SocketAddr,
    thread: Option<thread::JoinHandle<()>>,
}

impl WebServer {
    pub fn start(bind: &str, port: u16, web: PathBuf, json: Arc<JsonStore>) -> io::Result<Self> {
        let listener = TcpListener::bind((bind, port))?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::Builder::new().name("squaremap-httpd".into()).spawn(move || {
            let active = Arc::new(AtomicUsize::new(0));
            for stream in listener.incoming() {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                if active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    continue; // dropped: the client retries
                }
                active.fetch_add(1, Ordering::Relaxed);
                let (web, json, done) = (web.clone(), json.clone(), active.clone());
                let spawned = thread::Builder::new().name("squaremap-http".into()).spawn(move || {
                    let _ = serve(stream, &web, &json);
                    done.fetch_sub(1, Ordering::Relaxed);
                });
                if spawned.is_err() {
                    active.fetch_sub(1, Ordering::Relaxed);
                }
            }
        })?;
        Ok(Self { stop, addr, thread: Some(thread) })
    }

    pub fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }
}

impl Drop for WebServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the accept loop.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn serve(stream: TcpStream, web: &Path, json: &JsonStore) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream.try_clone()?.take(16 * 1024));
    let mut request = String::new();
    reader.read_line(&mut request)?;
    // Skip the headers.
    let mut line = String::new();
    while reader.read_line(&mut line)? > 2 {
        line.clear();
    }
    let mut parts = request.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut out = io::BufWriter::new(stream);
    if method != "GET" && method != "HEAD" {
        return respond(&mut out, "405 Method Not Allowed", "text/plain", b"", method == "HEAD", "");
    }
    let head = method == "HEAD";
    let Some(path) = clean_path(target) else {
        return respond(&mut out, "400 Bad Request", "text/plain", b"", head, "");
    };
    let tiles = path.starts_with("/tiles/");
    let cache = if tiles { "Cache-Control: max-age=0, must-revalidate, no-cache\r\n" } else { "" };
    if let Some(doc) = json.get(&path) {
        return respond(&mut out, "200 OK", "application/json", doc.as_bytes(), head, cache);
    }
    let rel = if path.ends_with('/') { format!("{path}index.html") } else { path.clone() };
    let file = web.join(rel.trim_start_matches('/'));
    match std::fs::read(&file) {
        Ok(body) => respond(&mut out, "200 OK", content_type(&file), &body, head, cache),
        // squaremap answers a tile that isn't rendered yet with an empty 200.
        Err(_) if tiles && path.ends_with(".png") => respond(&mut out, "200 OK", "image/png", b"", head, cache),
        Err(_) => respond(&mut out, "404 Not Found", "text/plain", b"Not Found", head, ""),
    }
}

fn respond(out: &mut impl Write, status: &str, kind: &str, body: &[u8], head: bool, extra: &str) -> io::Result<()> {
    write!(
        out,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{extra}X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if !head {
        out.write_all(body)?;
    }
    out.flush()
}

/// The request path, percent-decoded, without query, and only with normal segments.
pub fn clean_path(target: &str) -> Option<String> {
    let raw = target.split(['?', '#']).next()?;
    if !raw.starts_with('/') {
        return None;
    }
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    let path = String::from_utf8(decoded).ok()?;
    if path.contains('\\') || path.contains('\0') {
        return None;
    }
    let ok = Path::new(path.trim_start_matches('/'))
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    ok.then_some(path)
}

fn content_type(file: &Path) -> &'static str {
    match file.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_the_web_folder() {
        assert_eq!(clean_path("/tiles/a.png?x=1").as_deref(), Some("/tiles/a.png"));
        assert_eq!(clean_path("/").as_deref(), Some("/"));
        assert_eq!(clean_path("/%2e%2e/secret"), None);
        assert_eq!(clean_path("/a/../b"), None);
        assert_eq!(clean_path("/a\\b"), None);
        assert_eq!(clean_path("relative"), None);
    }

    fn get(addr: std::net::SocketAddr, req: &str) -> String {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn serves_json_files_and_empty_tiles() {
        let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-scratch/web");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("index.html"), "hi").unwrap();
        let json = Arc::new(JsonStore::default());
        json.put("/tiles/players.json", "{\"players\":[]}".into());
        let server = WebServer::start("127.0.0.1", 0, web.clone(), json).unwrap();
        let a = server.addr();
        assert!(get(a, "GET / HTTP/1.1\r\n\r\n").ends_with("\r\n\r\nhi"));
        let p = get(a, "GET /tiles/players.json HTTP/1.1\r\n\r\n");
        assert!(p.contains("application/json") && p.contains("no-cache") && p.ends_with("{\"players\":[]}"));
        assert!(get(a, "GET /tiles/w/3/0_0.png HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 200 OK"));
        assert!(get(a, "GET /nope.txt HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
        assert!(get(a, "GET /../x HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 400"));
        assert!(get(a, "POST / HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 405"));
        drop(server);
    }
}
