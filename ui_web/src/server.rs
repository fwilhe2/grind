// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The server behind the page, when there is one (`doc/server.md`): `grind serve`, found by
//! asking `api/v1/hello` beside the page and never assumed. Every request is to a path relative
//! to the page's own address — **the one module that sends anything but a plain GET**, held to
//! that by `cli/tests/local_only.rs`.

use js_sys::{JSON, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

/// Where every request goes, relative to the page.
const API: &str = "api/v1/";
/// What this page speaks; a server speaking another is no server.
const PROTOCOL: f64 = 1.0;

/// What a server said it can do.
#[derive(Clone, Debug)]
pub struct Offers {
    pub history: bool,
}

/// A document in the server's library.
#[derive(Clone, Debug)]
pub struct Doc {
    pub id: String,
    pub name: String,
    pub etag: String,
}

/// One earlier version of a document.
#[derive(Clone, Debug)]
pub struct Version {
    pub rev: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}

/// Where the open document came from, and so where Save puts it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    /// Picked, dropped, made here or fetched: saving downloads it.
    #[default]
    Local,
    /// A document in the server's library, at the version it was opened or last saved at.
    Remote { id: String, etag: String },
}

/// Why a write did not happen.
pub enum Refused {
    /// Somebody saved first.
    Stale,
    Said(String),
}

impl From<String> for Refused {
    fn from(message: String) -> Self {
        Refused::Said(message)
    }
}

struct Reply {
    status: u16,
    etag: Option<String>,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Option<JsValue> {
        JSON::parse(std::str::from_utf8(&self.body).ok()?).ok()
    }

    /// The server's `{"error": …}`, or the status when it said nothing.
    fn error(&self) -> String {
        self.json()
            .and_then(|v| string(&v, "error"))
            .unwrap_or_else(|| format!("the server answered {}", self.status))
    }
}

async fn request(
    method: &str,
    path: &str,
    if_match: Option<&str>,
    body: Option<&[u8]>,
) -> Result<Reply, String> {
    let window = web_sys::window().ok_or("no window")?;
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    let headers = js_sys::Object::new();
    // What tells the server a write is this page's and not a form on another site.
    let _ = Reflect::set(&headers, &"X-Grind-Request".into(), &"1".into());
    if let Some(tag) = if_match {
        let _ = Reflect::set(&headers, &"If-Match".into(), &tag.into());
    }
    init.set_headers(&headers);
    if let Some(body) = body {
        init.set_body(&Uint8Array::from(body));
    }
    let unreachable = |_| "the server did not answer".to_owned();
    let response = JsFuture::from(window.fetch_with_str_and_init(&format!("{API}{path}"), &init))
        .await
        .map_err(unreachable)?;
    let response: web_sys::Response = response.dyn_into().map_err(|_| "not a response")?;
    let buffer = JsFuture::from(response.array_buffer().map_err(unreachable)?)
        .await
        .map_err(unreachable)?;
    Ok(Reply {
        status: response.status(),
        etag: response.headers().get("ETag").ok().flatten(),
        body: Uint8Array::new(&buffer).to_vec(),
    })
}

fn string(value: &JsValue, key: &str) -> Option<String> {
    Reflect::get(value, &key.into()).ok()?.as_string()
}

fn list_of(value: &JsValue, key: &str) -> Vec<JsValue> {
    Reflect::get(value, &key.into())
        .ok()
        .and_then(|v| v.dyn_into::<js_sys::Array>().ok())
        .map_or_default(|a| a.iter().collect())
}

fn doc(value: &JsValue) -> Option<Doc> {
    Some(Doc {
        id: string(value, "id")?,
        name: string(value, "name")?,
        etag: string(value, "etag")?,
    })
}

fn path_segment(text: &str) -> String {
    String::from(js_sys::encode_uri_component(text))
}

/// Ask whether a server is there. `Err` is a server of another build, which the page then does
/// not talk to; `Ok(None)` is no server at all, the page as it always was.
pub async fn probe() -> Result<Option<Offers>, String> {
    let Ok(reply) = request("GET", "hello", None, None).await else {
        return Ok(None);
    };
    let Some(hello) = reply.json().filter(|_| reply.status == 200) else {
        return Ok(None);
    };
    let protocol = Reflect::get(&hello, &"protocol".into())
        .ok()
        .and_then(|v| v.as_f64());
    if protocol != Some(PROTOCOL) {
        return Ok(None);
    }
    if string(&hello, "commit").as_deref() != Some(grind_core::build_info::COMMIT) {
        return Err("This page and the server are different builds of grind — reload the page to talk to the server".to_owned());
    }
    let offers = list_of(&hello, "offers");
    Ok(Some(Offers {
        history: offers
            .iter()
            .any(|o| o.as_string().as_deref() == Some("history")),
    }))
}

pub async fn list() -> Result<Vec<Doc>, String> {
    let reply = request("GET", "docs", None, None).await?;
    match (reply.status, reply.json()) {
        (200, Some(json)) => Ok(list_of(&json, "docs").iter().filter_map(doc).collect()),
        _ => Err(reply.error()),
    }
}

/// The bytes and the version they are.
pub async fn get(id: &str) -> Result<(Vec<u8>, String), String> {
    let reply = request("GET", &format!("docs/{}", path_segment(id)), None, None).await?;
    match (reply.status, reply.etag.clone()) {
        (200, Some(tag)) => Ok((reply.body, tag)),
        _ => Err(reply.error()),
    }
}

/// Save over the version `etag`; `"*"` over whatever is there. The new version on success.
pub async fn put(id: &str, etag: &str, bytes: &[u8]) -> Result<String, Refused> {
    let path = format!("docs/{}", path_segment(id));
    let reply = request("PUT", &path, Some(etag), Some(bytes)).await?;
    written(reply).map(|doc| doc.etag)
}

fn written(reply: Reply) -> Result<Doc, Refused> {
    match reply.status {
        200 | 201 => reply
            .json()
            .as_ref()
            .and_then(doc)
            .ok_or_else(|| Refused::Said(reply.error())),
        412 => Err(Refused::Stale),
        _ => Err(Refused::Said(reply.error())),
    }
}

pub async fn create(name: &str, bytes: &[u8]) -> Result<Doc, String> {
    let path = format!("docs?name={}", path_segment(name));
    let reply = request("POST", &path, None, Some(bytes)).await?;
    written(reply).map_err(|refused| match refused {
        Refused::Stale => "somebody saved first".to_owned(),
        Refused::Said(why) => why,
    })
}

pub async fn rename(id: &str, etag: &str, name: &str) -> Result<Doc, Refused> {
    let body = JSON::stringify(&js_object(&[("name", name)])).map_or_default(String::from);
    let path = format!("docs/{}", path_segment(id));
    written(request("PATCH", &path, Some(etag), Some(body.as_bytes())).await?)
}

pub async fn delete(id: &str, etag: &str) -> Result<(), Refused> {
    let path = format!("docs/{}", path_segment(id));
    let reply = request("DELETE", &path, Some(etag), None).await?;
    match reply.status {
        204 => Ok(()),
        412 => Err(Refused::Stale),
        _ => Err(Refused::Said(reply.error())),
    }
}

pub async fn history(id: &str) -> Result<Vec<Version>, String> {
    let path = format!("docs/{}/history", path_segment(id));
    let reply = request("GET", &path, None, None).await?;
    let Some(json) = reply.json().filter(|_| reply.status == 200) else {
        return Err(reply.error());
    };
    Ok(list_of(&json, "versions")
        .iter()
        .filter_map(|v| {
            Some(Version {
                rev: string(v, "rev")?,
                author: string(v, "author")?,
                date: string(v, "date")?,
                subject: string(v, "subject")?,
            })
        })
        .collect())
}

pub async fn at(id: &str, rev: &str) -> Result<Vec<u8>, String> {
    let path = format!("docs/{}/at/{}", path_segment(id), path_segment(rev));
    let reply = request("GET", &path, None, None).await?;
    match reply.status {
        200 => Ok(reply.body),
        _ => Err(reply.error()),
    }
}

/// Put `rev` back as the current version, over the version `etag`.
pub async fn restore(id: &str, etag: &str, rev: &str) -> Result<Doc, Refused> {
    let body = JSON::stringify(&js_object(&[("rev", rev)])).map_or_default(String::from);
    let path = format!("docs/{}/restore", path_segment(id));
    written(request("POST", &path, Some(etag), Some(body.as_bytes())).await?)
}

fn js_object(pairs: &[(&str, &str)]) -> JsValue {
    let object = js_sys::Object::new();
    for (key, value) in pairs {
        let _ = Reflect::set(&object, &(*key).into(), &(*value).into());
    }
    object.into()
}

/// A version's name in the palette: when, what, who.
pub fn describe(version: &Version) -> String {
    let when = version
        .date
        .get(..16)
        .unwrap_or(&version.date)
        .replace('T', " ");
    format!("{when} · {} · {}", version.subject, version.author)
}

/// The name a version opens under: the document's own, with the commit before the extension, so
/// a download keeps the form.
pub fn version_name(name: &str, rev: &str) -> String {
    let short = rev.get(..7).unwrap_or(rev);
    let file = name.rsplit('/').next().unwrap_or(name);
    match file.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem} @ {short}.{ext}"),
        None => format!("{file} @ {short}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_opens_under_its_documents_name() {
        let rev = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            version_name("letters/plan.fods", rev),
            "plan @ 0123456.fods"
        );
        assert_eq!(
            describe(&Version {
                rev: rev.to_owned(),
                author: "Test".to_owned(),
                date: "2026-11-08T09:30:12+01:00".to_owned(),
                subject: "Save plan.fods".to_owned(),
            }),
            "2026-11-08 09:30 · Save plan.fods · Test"
        );
    }
}
