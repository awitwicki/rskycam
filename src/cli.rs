//! `rskycam cli` — a console client for a running rskycam, for SSH sessions
//! where the web panel can't be opened (or isn't wanted).
//!
//! It runs alongside the service as a separate process and talks to it over
//! the same HTTP API the web UI uses, so settings go through the server's
//! persist-then-adopt path and take effect immediately. Authentication: on
//! the Pi itself the CLI signs its own session cookie with
//! `$RSKYCAM_DATA/secret` (whoever can read that file can already rewrite
//! config.toml, so this grants nothing new); otherwise it logs in with the
//! admin password (`RSKYCAM_PASSWORD` or an interactive prompt).
//!
//! When the service isn't running, settings commands fall back to editing
//! config.toml directly and `logs` reads the log files; `status` needs the
//! service.

use std::io::{BufRead, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use serde_json::Value;

use crate::settings::{Settings, SettingsStore};
use crate::web::auth_layer::{ADMIN_USERNAME, SESSION_COOKIE};

const IO_TIMEOUT: Duration = Duration::from_secs(20);

const USAGE: &str = "\
rskycam cli — console client for a running rskycam

USAGE:
    rskycam cli [OPTIONS] [COMMAND [ARGS...]]

With no COMMAND an interactive shell starts (type `help` inside it).

OPTIONS:
    --data-dir <DIR>   data directory   [env RSKYCAM_DATA, default /var/lib/rskycam]
    --port <PORT>      local web port   [env RSKYCAM_PORT, default 8080]
    --url <URL>        server to talk to, e.g. http://rpi.local:8080
                       (default http://127.0.0.1:<port>)
    --password         log in with the admin password instead of the secret
                       file (password from RSKYCAM_PASSWORD or a prompt)
    -h, --help         show this help";

const COMMANDS: &str = "\
COMMANDS:
    status [--json]              camera, sky, sensor, system and RTSP status
    watch [SECONDS]              refresh status every SECONDS (default 5), Ctrl-C stops
    settings [FILTER] [--json]   list all settings as `path = value`
                                 (FILTER keeps paths containing that text)
    get <PATH>                   show one setting, e.g. `get camera.gainMax`
    set <PATH> <VALUE> [...]     change one or more settings, e.g.
                                 `set camera.autoExposure false camera.manualGain 4`
                                 VALUE is JSON (numbers, true/false, null, [..], {..});
                                 anything else is taken as a string
    logs [LINES]                 tail of the service log (default 50)
    help                         this list
    quit | exit                  leave the shell";

/// Entry point for `rskycam cli ...` (args after `cli`). Synchronous: it is a
/// short-lived client process, no async runtime needed. Returns the process
/// exit code; errors are printed as one line, not as a backtrace.
pub fn main(args: Vec<String>) -> i32 {
    match run(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e:#}");
            1
        }
    }
}

fn run(args: Vec<String>) -> anyhow::Result<()> {
    let mut data_dir = std::env::var("RSKYCAM_DATA").unwrap_or_else(|_| "/var/lib/rskycam".into());
    let mut port: u16 = std::env::var("RSKYCAM_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let mut url: Option<String> = None;
    let mut use_password = false;
    let mut rest = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" if rest.is_empty() => {
                println!("{USAGE}\n\n{COMMANDS}");
                return Ok(());
            }
            "--data-dir" if rest.is_empty() => {
                data_dir = it.next().context("--data-dir needs a value")?
            }
            "--port" if rest.is_empty() => {
                port = it
                    .next()
                    .context("--port needs a value")?
                    .parse()
                    .context("--port must be a number")?
            }
            "--url" if rest.is_empty() => url = Some(it.next().context("--url needs a value")?),
            "--password" if rest.is_empty() => use_password = true,
            _ => rest.push(a),
        }
    }
    let target = match url {
        Some(u) => Target::parse_url(&u)?,
        None => Target {
            host: "127.0.0.1".into(),
            port,
        },
    };
    let mut session = Session {
        target,
        data_dir: PathBuf::from(data_dir),
        use_password,
        cookie: None,
        offline_noted: false,
    };
    if rest.is_empty() {
        repl(&mut session)
    } else {
        session.exec(&rest)
    }
}

fn repl(s: &mut Session) -> anyhow::Result<()> {
    println!(
        "rskycam cli {} — connected to {} (type `help`, `quit` to leave)",
        crate::version::full(),
        s.target
    );
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        print!("rskycam> ");
        std::io::stdout().flush()?;
        line.clear();
        if stdin.lock().read_line(&mut line)? == 0 {
            println!();
            return Ok(()); // EOF / Ctrl-D
        }
        let words = match split_words(&line) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("error: {e}");
                continue;
            }
        };
        match words.first().map(String::as_str) {
            None => continue,
            Some("quit" | "exit" | "q") => return Ok(()),
            _ => {
                if let Err(e) = s.exec(&words) {
                    eprintln!("error: {e:#}");
                }
            }
        }
    }
}

/// Split a shell line into words; single or double quotes group spaces.
fn split_words(line: &str) -> anyhow::Result<Vec<String>> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                in_word = true;
            }
            None if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            None => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        bail!("unterminated quote");
    }
    if in_word {
        words.push(cur);
    }
    Ok(words)
}

#[derive(Clone, Debug, PartialEq)]
struct Target {
    host: String,
    port: u16,
}

impl Target {
    fn parse_url(url: &str) -> anyhow::Result<Target> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| anyhow!("only http:// URLs are supported: {url}"))?;
        let authority = rest.split('/').next().unwrap_or_default();
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !h.ends_with(']') || p.parse::<u16>().is_ok() => (
                h,
                p.parse()
                    .with_context(|| format!("bad port in --url {url}"))?,
            ),
            _ => (authority, 80),
        };
        if host.is_empty() {
            bail!("no host in --url {url}");
        }
        Ok(Target {
            host: host.trim_start_matches('[').trim_end_matches(']').into(),
            port,
        })
    }

    fn is_local(&self) -> bool {
        matches!(self.host.as_str(), "127.0.0.1" | "localhost" | "::1")
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.host.contains(':') {
            write!(f, "http://[{}]:{}", self.host, self.port)
        } else {
            write!(f, "http://{}:{}", self.host, self.port)
        }
    }
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// The server isn't listening (as opposed to answering with an error).
#[derive(Debug)]
struct Unreachable(String);

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rskycam is not reachable at {}", self.0)
    }
}

impl std::error::Error for Unreachable {}

/// Minimal HTTP/1.1 client (one request per connection) — enough for the
/// JSON API without pulling in a client stack.
fn http(
    target: &Target,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<&str>,
) -> anyhow::Result<HttpResponse> {
    let addrs: Vec<_> = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .with_context(|| format!("resolving {}", target.host))?
        .collect();
    let mut stream = addrs
        .iter()
        .find_map(|a| TcpStream::connect_timeout(a, IO_TIMEOUT).ok())
        .ok_or_else(|| Unreachable(target.to_string()))?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nAccept: application/json\r\n",
        target.host, target.port
    );
    if let Some(c) = cookie {
        req.push_str(&format!("Cookie: {c}\r\n"));
    }
    let body = body.unwrap_or("");
    if !body.is_empty() || method != "GET" {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    req.push_str("\r\n");
    req.push_str(body);
    stream.write_all(req.as_bytes())?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .context("reading the server's response")?;
    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> anyhow::Result<HttpResponse> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("malformed HTTP response"))?;
    let head = std::str::from_utf8(&raw[..split]).context("non-UTF-8 response headers")?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow!("malformed status line: {status_line}"))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let mut resp = HttpResponse {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    };
    if resp
        .header("transfer-encoding")
        .is_some_and(|v| v.eq_ignore_ascii_case("chunked"))
    {
        resp.body = dechunk(&resp.body)?;
    }
    Ok(resp)
}

fn dechunk(mut data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = data
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or_else(|| anyhow!("truncated chunked body"))?;
        let size_str = std::str::from_utf8(&data[..eol])?;
        let size_hex = size_str.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_hex, 16)
            .with_context(|| format!("bad chunk size {size_hex:?}"))?;
        data = &data[eol + 2..];
        if size == 0 {
            return Ok(out);
        }
        if data.len() < size {
            bail!("truncated chunked body");
        }
        out.extend_from_slice(&data[..size]);
        data = data.get(size + 2..).unwrap_or_default();
    }
}

/// A session cookie signed with the server's own key — exactly what
/// `/api/login` would have handed out.
fn mint_cookie(secret: &[u8]) -> anyhow::Result<String> {
    use axum::response::IntoResponse;
    use axum_extra::extract::cookie::{Cookie, Key, SignedCookieJar};
    if secret.len() < 64 {
        bail!("secret file is too short");
    }
    let jar =
        SignedCookieJar::new(Key::from(secret)).add(Cookie::new(SESSION_COOKIE, ADMIN_USERNAME));
    let resp = (jar, ()).into_response();
    let set = resp
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| anyhow!("could not sign a session cookie"))?;
    Ok(set.split(';').next().unwrap_or_default().to_string())
}

fn read_password() -> anyhow::Result<String> {
    if let Ok(p) = std::env::var("RSKYCAM_PASSWORD") {
        return Ok(p);
    }
    eprint!("admin password: ");
    std::io::stderr().flush()?;
    // Hide the typed password on a terminal; harmless if stty isn't there.
    let echo_off = std::process::Command::new("stty")
        .arg("-echo")
        .stdin(std::process::Stdio::inherit())
        .status()
        .is_ok_and(|s| s.success());
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line);
    if echo_off {
        let _ = std::process::Command::new("stty")
            .arg("echo")
            .stdin(std::process::Stdio::inherit())
            .status();
        eprintln!();
    }
    read?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

struct Session {
    target: Target,
    data_dir: PathBuf,
    use_password: bool,
    cookie: Option<String>,
    offline_noted: bool,
}

impl Session {
    fn exec(&mut self, words: &[String]) -> anyhow::Result<()> {
        let cmd = words[0].as_str();
        let args = &words[1..];
        match cmd {
            "help" | "-h" | "--help" => {
                println!("{COMMANDS}");
                Ok(())
            }
            "status" => {
                let v = self.get_json("/api/status")?;
                if args.iter().any(|a| a == "--json") {
                    println!("{}", serde_json::to_string_pretty(&v)?);
                } else {
                    print!("{}", format_status(&v));
                }
                Ok(())
            }
            "watch" => {
                let secs: u64 = match args.first() {
                    Some(a) => a.parse().context("watch interval must be whole seconds")?,
                    None => 5,
                };
                let secs = secs.max(1);
                loop {
                    let v = self.get_json("/api/status")?;
                    // Clear screen + home, then redraw.
                    print!(
                        "\x1b[2J\x1b[H{}\n(every {secs}s — Ctrl-C to stop)\n",
                        format_status(&v)
                    );
                    std::io::stdout().flush()?;
                    std::thread::sleep(Duration::from_secs(secs));
                }
            }
            "settings" | "config" => {
                let json = args.iter().any(|a| a == "--json");
                let filter = args
                    .iter()
                    .find(|a| *a != "--json")
                    .map(|f| f.to_lowercase());
                let v = self.load_settings()?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&v)?);
                } else {
                    for (path, val) in flatten(&v) {
                        if filter
                            .as_ref()
                            .is_none_or(|f| path.to_lowercase().contains(f.as_str()))
                        {
                            println!("{path} = {val}");
                        }
                    }
                }
                Ok(())
            }
            "get" => {
                let path = args.first().context("usage: get <PATH>")?;
                let v = self.load_settings()?;
                let (canon, val) = lookup(&v, path)?;
                if val.is_object() || val.is_array() {
                    for (p, x) in flatten(val) {
                        println!("{canon}.{p} = {x}");
                    }
                } else {
                    println!("{canon} = {val}");
                }
                Ok(())
            }
            "set" => {
                if args.is_empty() || !args.len().is_multiple_of(2) {
                    bail!("usage: set <PATH> <VALUE> [<PATH> <VALUE> ...]");
                }
                let mut v = self.load_settings()?;
                let mut changed = Vec::new();
                for pair in args.chunks(2) {
                    changed.push(assign(&mut v, &pair[0], &pair[1])?);
                }
                let after = self.save_settings(&v)?;
                for path in changed {
                    let wanted = lookup(&v, &path)?.1.clone();
                    let got = lookup(&after, &path)?.1.clone();
                    let same = got == wanted
                        || matches!((got.as_f64(), wanted.as_f64()), (Some(a), Some(b)) if a == b);
                    if same {
                        println!("{path} = {got}");
                    } else {
                        println!(
                            "{path} = {got}   (requested {wanted}, adjusted to a valid value)"
                        );
                    }
                }
                Ok(())
            }
            "logs" | "log" => {
                let n: usize = match args.first() {
                    Some(a) => a.parse().context("logs takes a line count")?,
                    None => 50,
                };
                let n = n.min(crate::logs::MAX_LINES);
                let lines = match self.get_json(&format!("/api/logs?lines={n}")) {
                    Ok(v) => serde_json::from_value::<Vec<String>>(v["lines"].clone())?,
                    Err(e) if e.is::<Unreachable>() => {
                        self.note_offline();
                        crate::logs::read_tail(&self.data_dir, n)
                    }
                    Err(e) => return Err(e),
                };
                for l in lines {
                    println!("{l}");
                }
                Ok(())
            }
            other => bail!("unknown command `{other}` (try `help`)"),
        }
    }

    fn cookie(&mut self) -> anyhow::Result<String> {
        if let Some(c) = &self.cookie {
            return Ok(c.clone());
        }
        let c = if !self.use_password && self.target.is_local() {
            match std::fs::read(self.data_dir.join("secret")) {
                Ok(secret) => mint_cookie(&secret)?,
                Err(e) => {
                    eprintln!(
                        "note: can't read {} ({e}); falling back to password login \
                         (run as the service user, e.g. `sudo -u rskycam rskycam cli`, to skip this)",
                        self.data_dir.join("secret").display()
                    );
                    self.login()?
                }
            }
        } else {
            self.login()?
        };
        self.cookie = Some(c.clone());
        Ok(c)
    }

    fn login(&self) -> anyhow::Result<String> {
        let password = read_password()?;
        let body = serde_json::json!({"username": ADMIN_USERNAME, "password": password});
        let resp = http(
            &self.target,
            "POST",
            "/api/login",
            None,
            Some(&body.to_string()),
        )?;
        if resp.status == 401 {
            bail!("wrong password");
        }
        if resp.status / 100 != 2 {
            bail!("login failed: HTTP {}", resp.status);
        }
        let set = resp
            .header("set-cookie")
            .ok_or_else(|| anyhow!("login succeeded but no session cookie was returned"))?;
        Ok(set.split(';').next().unwrap_or_default().to_string())
    }

    fn request(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> anyhow::Result<HttpResponse> {
        let cookie = self.cookie()?;
        let resp = http(&self.target, method, path, Some(&cookie), body)?;
        if resp.status == 401 {
            self.cookie = None;
            bail!("the server rejected the session (HTTP 401) — is --data-dir pointing at the service's data directory?");
        }
        Ok(resp)
    }

    fn get_json(&mut self, path: &str) -> anyhow::Result<Value> {
        // Probe reachability before asking for a password.
        if self.cookie.is_none() {
            http(&self.target, "GET", "/api/status", None, None)?;
        }
        let resp = self.request("GET", path, None)?;
        if resp.status != 200 {
            bail!("GET {path} failed: HTTP {}", resp.status);
        }
        serde_json::from_slice(&resp.body).with_context(|| format!("parsing {path} response"))
    }

    fn note_offline(&mut self) {
        if !self.offline_noted {
            eprintln!(
                "note: rskycam is not running at {} — working on files in {}",
                self.target,
                self.data_dir.display()
            );
            self.offline_noted = true;
        }
    }

    fn store(&self) -> anyhow::Result<SettingsStore> {
        let path = self.data_dir.join("config.toml");
        if !path.is_file() {
            bail!(
                "rskycam is not running at {} and {} doesn't exist (wrong --data-dir?)",
                self.target,
                path.display()
            );
        }
        Ok(SettingsStore::new(&self.data_dir))
    }

    fn load_settings(&mut self) -> anyhow::Result<Value> {
        match self.get_json("/api/settings") {
            Err(e) if e.is::<Unreachable>() && self.target.is_local() => {
                self.note_offline();
                let cfg = self.store()?.load_existing()?;
                Ok(serde_json::to_value(cfg.settings)?)
            }
            other => other,
        }
    }

    /// Persist the edited settings; returns what is actually in effect
    /// afterwards (the server clamps out-of-range values).
    fn save_settings(&mut self, v: &Value) -> anyhow::Result<Value> {
        let mut parsed: Settings =
            serde_json::from_value(v.clone()).map_err(|e| anyhow!("invalid value: {e}"))?;
        let body = serde_json::to_string(v)?;
        match self.request("PUT", "/api/settings", Some(&body)) {
            Ok(resp) if resp.status / 100 == 2 => self.get_json("/api/settings"),
            Ok(resp) => bail!(
                "the server refused the settings: HTTP {} {}",
                resp.status,
                String::from_utf8_lossy(&resp.body).trim()
            ),
            Err(e) if e.is::<Unreachable>() && self.target.is_local() => {
                self.note_offline();
                let store = self.store()?;
                let mut cfg = store.load_existing()?;
                parsed.sanitize();
                cfg.settings = parsed;
                store.save(&cfg)?;
                Ok(serde_json::to_value(&cfg.settings)?)
            }
            Err(e) => Err(e),
        }
    }
}

/// `path = value` pairs for every leaf, in document order.
fn flatten(v: &Value) -> Vec<(String, String)> {
    fn walk(v: &Value, prefix: &str, out: &mut Vec<(String, String)>) {
        match v {
            Value::Object(m) if !m.is_empty() => {
                for (k, x) in m {
                    let p = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    walk(x, &p, out);
                }
            }
            _ => out.push((prefix.to_string(), v.to_string())),
        }
    }
    let mut out = Vec::new();
    walk(v, "", &mut out);
    out
}

/// Loose key match: exact, else case-insensitive ignoring `_`/`-`, so
/// `camera.gain_max` and `CAMERA.GAINMAX` both find `camera.gainMax`.
fn find_key<'a>(m: &'a serde_json::Map<String, Value>, want: &str) -> Option<&'a String> {
    if let Some((k, _)) = m.get_key_value(want) {
        return Some(k);
    }
    let norm = |s: &str| {
        s.chars()
            .filter(|c| *c != '_' && *c != '-')
            .collect::<String>()
            .to_lowercase()
    };
    let w = norm(want);
    m.keys().find(|k| norm(k) == w)
}

/// Resolve a dotted path to its canonical spelling and value.
fn lookup<'a>(v: &'a Value, path: &str) -> anyhow::Result<(String, &'a Value)> {
    let mut cur = v;
    let mut canon = Vec::new();
    for seg in path.split('.') {
        cur = match cur {
            Value::Object(m) => {
                let k = find_key(m, seg).ok_or_else(|| unknown_path(path, &canon, cur))?;
                canon.push(k.clone());
                &m[k]
            }
            Value::Array(a) => {
                let i: usize = seg.parse().map_err(|_| {
                    anyhow!(
                        "`{seg}` must be an index into the list at {}",
                        canon.join(".")
                    )
                })?;
                canon.push(seg.to_string());
                a.get(i).ok_or_else(|| {
                    anyhow!(
                        "{} has only {} entries",
                        canon[..canon.len() - 1].join("."),
                        a.len()
                    )
                })?
            }
            _ => bail!("{} is a value, not a section", canon.join(".")),
        };
    }
    Ok((canon.join("."), cur))
}

fn unknown_path(path: &str, canon: &[String], at: &Value) -> anyhow::Error {
    let keys: Vec<&str> = at
        .as_object()
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let section = if canon.is_empty() {
        "top level".to_string()
    } else {
        canon.join(".")
    };
    anyhow!(
        "no setting `{path}` (keys under {section}: {})",
        keys.join(", ")
    )
}

/// Set `path` to `raw` in the settings JSON; returns the canonical path.
/// `raw` is JSON when it parses as such, else a plain string — but a string
/// is only accepted where the current value is a string (or an enum).
fn assign(v: &mut Value, path: &str, raw: &str) -> anyhow::Result<String> {
    let (canon, old) = lookup(v, path)?;
    let new = match serde_json::from_str::<Value>(raw) {
        Ok(x) => x,
        Err(_) if old.is_string() => Value::String(raw.to_string()),
        Err(_) => bail!("`{raw}` is not a valid value for {canon} (current value: {old})"),
    };
    let type_ok = matches!(
        (old, &new),
        (Value::Bool(_), Value::Bool(_))
            | (Value::Number(_), Value::Number(_))
            | (Value::String(_), Value::String(_))
            | (Value::Array(_), Value::Array(_))
            | (Value::Object(_), Value::Object(_))
            | (Value::Null, _)
            | (_, Value::Null)
    );
    if !type_ok {
        bail!("{canon} expects a value like {old}, got {new}");
    }
    let mut cur = v;
    for seg in canon.split('.') {
        cur = match cur {
            Value::Object(m) => m.get_mut(seg).expect("path resolved above"),
            Value::Array(a) => &mut a[seg.parse::<usize>().expect("path resolved above")],
            _ => unreachable!("path resolved above"),
        };
    }
    *cur = new;
    Ok(canon)
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

fn exposure(us: f64) -> String {
    if us >= 1_000_000.0 {
        format!("{:.2} s", us / 1_000_000.0)
    } else if us >= 1000.0 {
        format!("{:.1} ms", us / 1000.0)
    } else {
        format!("{us:.0} µs")
    }
}

fn local_time(v: &Value) -> String {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%a %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "—".into())
}

fn uptime(secs: f64) -> String {
    let s = secs as u64;
    let (d, h, m) = (s / 86_400, s % 86_400 / 3600, s % 3600 / 60);
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else {
        format!("{h}h {m}m")
    }
}

/// Human-readable status block from the `/api/status` JSON.
fn format_status(v: &Value) -> String {
    let mut o = String::new();
    let mut line = |label: &str, text: String| o.push_str(&format!("{label:<10}{text}\n"));
    line("version", v["version"].as_str().unwrap_or("?").to_string());

    let cap = &v["capture"];
    let mut c = cap["state"].as_str().unwrap_or("?").to_string();
    if let Some(m) = cap["message"].as_str() {
        c.push_str(&format!(" — {m}"));
    }
    if let Some(cam) = v["camera"].as_object() {
        c.push_str(&format!(
            "  [{} {}x{}]",
            cam["model"].as_str().unwrap_or("?"),
            cam["maxWidth"],
            cam["maxHeight"]
        ));
    }
    line("capture", c);
    let f = &cap["lastFrame"];
    if f.is_object() {
        line(
            "frame",
            format!(
                "{}  exp {}  gain {:.1}  {}  mean {:.0} ({:.0}% metered)",
                local_time(&f["timestamp"]),
                exposure(num(&f["exposureUs"])),
                num(&f["gain"]),
                if f["isNight"].as_bool().unwrap_or(false) {
                    "night"
                } else {
                    "day"
                },
                num(&f["meteredMean"]),
                num(&f["meteredAreaPct"]),
            ),
        );
    } else {
        line("frame", "none yet".into());
    }
    if let Some(p) = v["darksProgress"].as_object() {
        line(
            "darks",
            format!("capturing {}/{}", p["current"], p["total"]),
        );
    }
    if v["focus"]["enabled"].as_bool().unwrap_or(false) {
        line(
            "focus",
            format!(
                "on — exp {}  gain {:.1}",
                exposure(num(&v["focus"]["exposureUs"])),
                num(&v["focus"]["gain"])
            ),
        );
    }

    let a = &v["astro"];
    line(
        "sun",
        format!(
            "alt {:+.1}°  rise {}  set {}  astro night {} → {}",
            num(&a["sunAltDeg"]),
            local_time(&a["sunriseIso"]),
            local_time(&a["sunsetIso"]),
            local_time(&a["astroDuskIso"]),
            local_time(&a["astroDawnIso"]),
        ),
    );
    line(
        "moon",
        format!(
            "alt {:+.1}°  {:.0}% {}  rise {}  set {}",
            num(&a["moonAltDeg"]),
            num(&a["moonPhasePct"]),
            if a["moonWaxing"].as_bool().unwrap_or(false) {
                "waxing"
            } else {
                "waning"
            },
            local_time(&a["moonriseIso"]),
            local_time(&a["moonsetIso"]),
        ),
    );

    let sen = &v["sensor"];
    let r = &sen["reading"];
    line(
        "sensor",
        match sen["state"].as_str() {
            Some("ok") => {
                let mut s = format!(
                    "{:.1} °C  {:.1} hPa",
                    num(&r["temperatureC"]),
                    num(&r["pressureHpa"])
                );
                if r["humidityPct"].is_number() {
                    s.push_str(&format!("  {:.0}% RH", num(&r["humidityPct"])));
                }
                s
            }
            Some(other) => other.replace('_', " "),
            None => "?".into(),
        },
    );

    let s = &v["system"];
    let mut sys = format!(
        "{}  CPU {:.1} °C  load {:.2}/{}  RAM {:.0}/{:.0} MB  disk {:.1}/{:.1} GB  up {}",
        s["model"].as_str().unwrap_or("?"),
        num(&s["cpuTempC"]),
        num(&s["cpuLoadAvg5m"]),
        s["cpuCores"],
        num(&s["ramUsedMb"]),
        num(&s["ramTotalMb"]),
        num(&s["diskUsedGb"]),
        num(&s["diskTotalGb"]),
        uptime(num(&s["uptimeSec"])),
    );
    if s["undervoltageNow"].as_bool().unwrap_or(false) {
        sys.push_str("  UNDERVOLTAGE NOW");
    } else if s["undervoltageSinceBoot"].as_bool().unwrap_or(false) {
        sys.push_str("  (undervoltage since boot)");
    }
    line("system", sys);

    let rt = &v["rtsp"];
    let mut rtsp = if !rt["enabled"].as_bool().unwrap_or(false) {
        "off".to_string()
    } else {
        format!(
            "{} on port {}  user {}  {} client(s){}",
            if rt["listening"].as_bool().unwrap_or(false) {
                "listening"
            } else {
                "not listening"
            },
            rt["port"],
            rt["username"].as_str().unwrap_or("?"),
            rt["clients"],
            if rt["encoding"].as_bool().unwrap_or(false) {
                ", encoding"
            } else {
                ""
            }
        )
    };
    if let Some(e) = rt["lastError"].as_str() {
        rtsp.push_str(&format!("  last error: {e}"));
    }
    line("rtsp", rtsp);
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_words_handles_quotes() {
        assert_eq!(
            split_words("set a.b 'two words' \"x y\"  c\n").unwrap(),
            vec!["set", "a.b", "two words", "x y", "c"]
        );
        assert_eq!(split_words("get ''").unwrap(), vec!["get", ""]);
        assert!(split_words("set a 'oops").is_err());
    }

    #[test]
    fn parse_url_variants() {
        assert_eq!(
            Target::parse_url("http://rpi.local:8080/").unwrap(),
            Target {
                host: "rpi.local".into(),
                port: 8080
            }
        );
        assert_eq!(Target::parse_url("http://pi").unwrap().port, 80);
        assert_eq!(Target::parse_url("http://[::1]:9000").unwrap().host, "::1");
        assert!(Target::parse_url("https://pi").is_err());
    }

    #[test]
    fn parse_response_decodes_chunked_bodies() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5;x=y\r\npedia\r\n0\r\n\r\n";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"Wikipedia");
        let plain = parse_response(b"HTTP/1.1 204 No Content\r\nX-A: 1\r\n\r\n").unwrap();
        assert_eq!(plain.status, 204);
        assert_eq!(plain.header("x-a"), Some("1"));
    }

    fn settings_json() -> Value {
        serde_json::to_value(Settings::default()).unwrap()
    }

    #[test]
    fn lookup_is_forgiving_about_key_spelling() {
        let v = settings_json();
        let (canon, val) = lookup(&v, "camera.gain_max").unwrap();
        assert_eq!(canon, "camera.gainMax");
        assert_eq!(val, &serde_json::json!(16.0));
        assert_eq!(
            lookup(&v, "overlay.textFields.0.id").unwrap().1,
            &serde_json::json!("time")
        );
        let err = lookup(&v, "camera.nope").unwrap_err().to_string();
        assert!(err.contains("gainMax"), "{err}");
        assert!(lookup(&v, "camera.gainMax.x").is_err());
    }

    #[test]
    fn assign_parses_and_type_checks() {
        let mut v = settings_json();
        assert_eq!(
            assign(&mut v, "camera.autoexposure", "false").unwrap(),
            "camera.autoExposure"
        );
        assert_eq!(v["camera"]["autoExposure"], serde_json::json!(false));
        assign(&mut v, "camera.driver", "asi").unwrap(); // bare word → string
        assert_eq!(v["camera"]["driver"], serde_json::json!("asi"));
        assign(
            &mut v,
            "image.crop",
            r#"{"x":0,"y":0,"width":10,"height":10}"#,
        )
        .unwrap();
        assert_eq!(v["image"]["crop"]["width"], serde_json::json!(10));
        assert!(assign(&mut v, "camera.gainMax", "lots").is_err());
        assert!(assign(&mut v, "camera.autoExposure", "1").is_err());
        // The edited document still deserializes into Settings.
        let s: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(s.camera.driver, crate::settings::CameraDriver::Asi);
    }

    #[test]
    fn flatten_lists_every_leaf() {
        let flat = flatten(&settings_json());
        assert!(flat
            .iter()
            .any(|(p, v)| p == "camera.autoExposure" && v == "true"));
        assert!(flat.iter().any(|(p, v)| p == "image.crop" && v == "null"));
    }

    #[test]
    fn format_status_renders_a_harness_status() {
        let v = serde_json::json!({
            "version": "1.2.3",
            "capture": {"state": "capturing", "lastFrame": {
                "timestamp": "2026-10-12T20:00:00Z", "exposureUs": 2500000, "gain": 8.0,
                "isNight": true, "meteredMean": 101.0, "meteredAreaPct": 100.0}},
            "sensor": {"state": "not_detected", "reading": null},
            "system": {"model": "Pi", "cpuTempC": 50.0, "cpuLoadAvg5m": 0.5, "cpuCores": 4,
                "ramUsedMb": 300, "ramTotalMb": 2000, "diskUsedGb": 5.0, "diskTotalGb": 30.0,
                "uptimeSec": 90061, "undervoltageNow": false, "undervoltageSinceBoot": true},
            "astro": {"sunAltDeg": -20.0, "moonAltDeg": 10.0, "moonPhasePct": 40.0,
                "moonWaxing": true, "sunriseIso": null, "sunsetIso": null, "astroDuskIso": null,
                "astroDawnIso": null, "moonriseIso": null, "moonsetIso": null,
                "moonTransitIso": "2026-10-12T20:00:00Z"},
            "camera": null, "darksProgress": null,
            "focus": {"enabled": false, "exposureUs": 0, "gain": 0.0},
            "rtsp": {"enabled": false, "listening": false, "port": 8554, "username": "admin",
                "clients": 0, "encoding": false, "lastError": null}
        });
        let s = format_status(&v);
        assert!(s.contains("capturing"), "{s}");
        assert!(s.contains("2.50 s"), "{s}");
        assert!(s.contains("not detected"), "{s}");
        assert!(s.contains("1d 1h 1m"), "{s}");
        assert!(s.contains("undervoltage since boot"), "{s}");
        assert!(s.contains("rtsp      off"), "{s}");
    }

    #[tokio::test]
    async fn minted_cookie_is_accepted_by_the_server() {
        use axum::body::Body;
        use axum::http::{header, Request, StatusCode};
        use tower::ServiceExt;
        let h = crate::web::testing::harness();
        let secret = [7u8; 64];
        let mut state = h.state.clone();
        state.key = axum_extra::extract::cookie::Key::from(&secret);
        let cookie = mint_cookie(&secret).unwrap();
        let res = crate::web::router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(mint_cookie(b"short").is_err());
    }
}
