//! Local news state for 26.3 RealmsNewsManager and RealmsHeader.openNews.
//! The caller supplies the player's directory; this module never resolves a home.
use serde_json::{Map, Value};
use std::{fs, io, path::Path};

const FILE_NAME: &str = "realms_persistence.json";

#[derive(Clone, Debug, Default)]
pub(super) struct News {
    pub link: Option<String>,
    pub unread: bool,
    // Preserve unrelated settings when rewriting an existing persistence file.
    other: Map<String, Value>,
}

/// RealmsPersistence.readFile: missing, unreadable or malformed data starts empty.
pub(super) fn read(directory: &Path) -> News {
    let path = directory.join(FILE_NAME);
    let result = fs::read(&path).and_then(|bytes| {
        let value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        decode(value).ok_or_else(|| io::Error::other("Invalid Realms persistence data"))
    });
    match result {
        Ok(news) => news,
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                log::warn!("Failed to read Realms storage {}: {error}", path.display());
            }
            News::default()
        }
    }
}

/// Every successful news response rereads storage, including null/unchanged links.
/// A new link is unread even when a best-effort save fails, as in vanilla.
pub(super) fn update(directory: &Path, link: Option<&str>) -> News {
    let mut news = read(directory);
    if let Some(link) = link.filter(|link| !link.is_empty() && uri_text(link))
        && !news
            .link
            .as_deref()
            .is_some_and(|old| uri_key(old) == uri_key(link))
    {
        news.link = Some(link.to_owned());
        news.unread = true;
        save(directory, &news);
    }
    news
}

/// Call on clicking an unread news button, before the link confirmation completes.
/// Reread first so opening the header does not overwrite a newer stored news link.
pub(super) fn mark_read(directory: &Path) {
    let mut news = read(directory);
    news.unread = false;
    save(directory, &news);
}

fn decode(value: Value) -> Option<News> {
    if value.is_null() {
        return Some(News::default());
    }
    let Value::Object(mut other) = value else {
        return None;
    };
    let link = match other.remove("newsLink") {
        None | Some(Value::Null) => None,
        Some(Value::String(link)) if link == "null" => None,
        Some(Value::String(link)) if uri_text(&link) => Some(link),
        Some(Value::Number(link)) => Some(link.to_string()),
        _ => return None,
    };
    let unread = match other.remove("hasUnreadNews") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(unread)) => unread,
        // Gson's boolean adapter also accepts strings, using Boolean.parseBoolean.
        Some(Value::String(unread)) => unread.eq_ignore_ascii_case("true"),
        _ => return None,
    };
    Some(News {
        link,
        unread,
        other,
    })
}

// Reject malformed URI text without normalizing the stored link (paths, explicit
// ports and escaped characters must survive a read/write round trip).
fn uri_text(link: &str) -> bool {
    !link.chars().any(|c| {
        c.is_control()
            || c.is_whitespace()
            || matches!(c, '<' | '>' | '"' | '{' | '}' | '|' | '\\' | '^' | '`')
    }) && link.bytes().enumerate().all(|(i, byte)| {
        byte != b'%'
            || link
                .as_bytes()
                .get(i + 1..i + 3)
                .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
    })
}

// Java URI.equals ignores scheme/host case and the case of percent escapes,
// but does not remove default ports, dot segments or trailing slashes like URL.
fn uri_key(link: &str) -> Vec<u8> {
    let mut key = link.as_bytes().to_vec();
    let mut i = 0;
    while i + 2 < key.len() {
        if key[i] == b'%' {
            key[i + 1..i + 3].make_ascii_uppercase();
            i += 3;
        } else {
            i += 1;
        }
    }
    let Some((scheme, rest)) = link.split_once(':') else {
        return key;
    };
    if scheme.is_empty()
        || !scheme.as_bytes()[0].is_ascii_alphabetic()
        || !scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
    {
        return key;
    }
    key[..scheme.len()].make_ascii_lowercase();
    if let Some(rest) = rest.strip_prefix("//") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let start = authority.rfind('@').map_or(0, |i| i + 1);
        let host_port = &authority[start..];
        let end = if host_port.starts_with('[') {
            host_port.find(']').map(|i| i + 1)
        } else {
            Some(host_port.find(':').unwrap_or(host_port.len()))
        };
        if let Some(end) = end {
            let host = &host_port[..end];
            let ip = host.trim_start_matches('[').trim_end_matches(']');
            let dns = host.trim_end_matches('.');
            let valid_host = ip.parse::<std::net::IpAddr>().is_ok()
                || (dns.rsplit('.').next().is_some_and(|label| {
                    label
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphabetic)
                }) && dns.split('.').all(|label| {
                    label
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphanumeric)
                        && label
                            .as_bytes()
                            .last()
                            .is_some_and(u8::is_ascii_alphanumeric)
                        && label
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                }));
            if valid_host {
                let offset = scheme.len() + 3 + start;
                key[offset..offset + end].make_ascii_lowercase();
            }
        }
    }
    key
}

fn save(directory: &Path, news: &News) {
    let mut value = news.other.clone();
    if let Some(link) = &news.link {
        value.insert("newsLink".into(), Value::String(link.clone()));
    }
    value.insert("hasUnreadNews".into(), Value::Bool(news.unread));
    if let Ok(bytes) = serde_json::to_vec(&value) {
        // Vanilla does not create the game directory or surface save failures.
        let _ = fs::write(directory.join(FILE_NAME), bytes);
    }
}
