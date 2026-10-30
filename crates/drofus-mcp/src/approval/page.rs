//! The approval page: a tiny HTTP server on `127.0.0.1` that shows what is waiting and takes a click.
//!
//! Hand-rolled over `tokio::net` rather than a web framework: it serves one page and one form, so a
//! framework would be most of the crate's dependency weight for two routes. The price is that the
//! parsing is deliberately strict and small (head 8 KiB, body 2 KiB, 5 s per connection, one request
//! per connection) and refuses anything it does not recognise.
//!
//! **What stands between a request and a decision**
//! - **A key** in the address (`?k=`, on the form as a hidden field), unguessable and never sent to the
//!   model. Without it: 403.
//! - **The `Host` header must be this server's own address.** A DNS-rebinding page reaches 127.0.0.1 but
//!   cannot make the browser send `Host: 127.0.0.1:<port>`.
//! - **A cross-origin POST is refused** (`Origin`, when the browser sends one, must be this page).
//! - **Everything the model wrote is HTML-escaped**, and the response carries a CSP that forbids
//!   scripts, `Referrer-Policy: no-referrer` (the key is in the address) and `X-Frame-Options: DENY`.
//!
//! The page has no JavaScript: it reloads itself every few seconds with a meta refresh so a new
//! proposal appears, and a click is an ordinary form post.

use std::{fmt::Write as _, io, time::Duration};

use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

use super::{random_hex, Gate, Ui};

const MAX_HEAD: usize = 8 * 1024;
const MAX_BODY: usize = 2 * 1024;
const PER_CONNECTION: Duration = Duration::from_secs(5);
const REFRESH_SECS: u32 = 4;

/// Bind a free port on loopback and serve until the process ends.
pub(super) async fn serve(gate: Gate) -> io::Result<Ui> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let key = random_hex(16);
    let state = (gate, port, key.clone());
    tokio::spawn(async move {
        loop {
            let Ok((conn, _)) = listener.accept().await else {
                continue;
            };
            let state = state.clone();
            tokio::spawn(async move {
                let _ = timeout(PER_CONNECTION, handle(conn, state)).await;
            });
        }
    });
    Ok(Ui { port, key })
}

struct Request {
    method: String,
    path: String,
    query: String,
    host: String,
    origin: Option<String>,
    body: String,
}

async fn read_request(conn: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
        let n = conn.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target) = (first.next()?.to_string(), first.next()?.to_string());
    let (path, query) = target
        .split_once('?')
        .map_or((target.clone(), String::new()), |(p, q)| (p.to_string(), q.to_string()));
    let (mut host, mut origin, mut length) = (String::new(), None, 0usize);
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "host" => host = value.to_string(),
            "origin" => origin = Some(value.to_string()),
            "content-length" => length = value.parse().ok()?,
            _ => {}
        }
    }
    if length > MAX_BODY {
        return None;
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = conn.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Some(Request {
        method,
        path,
        query,
        host,
        origin,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if bytes.get(i + 1..i + 3).is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit)) => {
                out.push(u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16).unwrap_or(b'?'));
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn form_field(form: &str, name: &str) -> Option<String> {
    form.split('&')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| percent_decode(v))
}

/// Compare without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(super) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

async fn respond(conn: &mut TcpStream, status: &str, extra: &str, body: &str) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; form-action 'self'\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    );
    let _ = conn.write_all(head.as_bytes()).await;
    let _ = conn.write_all(body.as_bytes()).await;
    let _ = conn.shutdown().await;
}

async fn handle(mut conn: TcpStream, (gate, port, key): (Gate, u16, String)) {
    let Some(req) = read_request(&mut conn).await else {
        return respond(&mut conn, "400 Bad Request", "", "bad request").await;
    };
    let own = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if !own.contains(&req.host) {
        return respond(&mut conn, "403 Forbidden", "", "forbidden").await;
    }
    if let Some(origin) = &req.origin
        && !own.iter().any(|h| *origin == format!("http://{h}"))
    {
        return respond(&mut conn, "403 Forbidden", "", "forbidden").await;
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => {
            if !form_field(&req.query, "k").is_some_and(|k| same(&k, &key)) {
                return respond(&mut conn, "403 Forbidden", "", "forbidden").await;
            }
            gate.mark_page_seen();
            respond(&mut conn, "200 OK", "", &render(&gate, &key, "")).await;
        }
        ("POST", "/decide") => {
            if !form_field(&req.body, "k").is_some_and(|k| same(&k, &key)) {
                return respond(&mut conn, "403 Forbidden", "", "forbidden").await;
            }
            let (id, hash) = (
                form_field(&req.body, "id").unwrap_or_default(),
                form_field(&req.body, "hash").unwrap_or_default(),
            );
            let approve = match form_field(&req.body, "decision").as_deref() {
                Some("approve") => true,
                Some("reject") => false,
                _ => return respond(&mut conn, "400 Bad Request", "", "bad request").await,
            };
            match gate.decide(&id, &hash, approve) {
                Ok(()) => respond(&mut conn, "303 See Other", &format!("Location: /?k={key}\r\n"), "").await,
                Err(e) => respond(&mut conn, "409 Conflict", "", &render(&gate, &key, &e.0)).await,
            }
        }
        _ => respond(&mut conn, "404 Not Found", "", "not found").await,
    }
}

fn render(gate: &Gate, key: &str, message: &str) -> String {
    let mut html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"refresh\" content=\"{REFRESH_SECS}\">\
         <title>Approve dRofus changes</title><style>body{{font:15px system-ui,sans-serif;max-width:60rem;margin:2rem auto;padding:0 1rem}}\
         pre{{background:#f4f4f4;padding:.75rem;overflow:auto;white-space:pre-wrap}}.card{{border:1px solid #bbb;border-radius:6px;padding:1rem;margin:1rem 0}}\
         button{{font:inherit;padding:.4rem 1.2rem;margin-right:.5rem}}.msg{{color:#a00}}small{{color:#555}}</style></head><body>\
         <h1>Changes waiting for your decision</h1>\
         <p><small>Nothing has been changed yet. Approving lets the assistant apply exactly the change shown, once. \
         This page reloads every {REFRESH_SECS} seconds.</small></p>"
    );
    if !message.is_empty() {
        let _ = write!(html, "<p class=\"msg\">{}</p>", escape(message));
    }
    let waiting = gate.waiting();
    if waiting.is_empty() {
        html.push_str("<p>Nothing is waiting.</p>");
    }
    for (id, summary, payload, hash, left) in waiting {
        let pretty = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| Value::Null.to_string());
        let _ = write!(
            html,
            "<div class=\"card\"><h2>{}</h2><pre>{}</pre><p><small>id {} &middot; hash {} &middot; expires in {} s</small></p>\
             <form method=\"post\" action=\"/decide\"><input type=\"hidden\" name=\"k\" value=\"{}\">\
             <input type=\"hidden\" name=\"id\" value=\"{}\"><input type=\"hidden\" name=\"hash\" value=\"{}\">\
             <button name=\"decision\" value=\"approve\">Approve</button><button name=\"decision\" value=\"reject\">Reject</button></form></div>",
            escape(&summary),
            escape(&pretty),
            escape(&id),
            escape(&hash),
            left,
            escape(key),
            escape(&id),
            escape(&hash)
        );
    }
    html.push_str("</body></html>");
    html
}

/// Open the address with the system's default browser. `false` when it could not be started.
pub(super) fn open_in_browser(url: &str) -> bool {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else {
        Command::new("xdg-open")
    };
    cmd.arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_covers_the_characters_that_break_out_of_html() {
        assert_eq!(
            escape("<a href=\"x\" onclick='y'>&"),
            "&lt;a href=&quot;x&quot; onclick=&#39;y&#39;&gt;&amp;"
        );
    }

    #[test]
    fn form_fields_decode_and_unknown_ones_are_absent() {
        assert_eq!(form_field("a=1&b=x%20y+z", "b").as_deref(), Some("x y z"));
        assert_eq!(form_field("a=1", "c"), None);
    }

    #[test]
    fn key_comparison_is_exact() {
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "ab"));
    }
}
