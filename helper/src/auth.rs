// SPDX-License-Identifier: GPL-3.0-or-later

use crate::error::{HelperError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const YTM_ORIGIN: &str = "https://music.youtube.com";
const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const BROWSER_SESSION_CAPTURE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub schema: u32,
    pub source: AuthSource,
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub innertube_context: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSource {
    pub kind: String,
    pub browser: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BrowserSessionCapture {
    schema: u32,
    source: BrowserSessionCaptureSource,
    cookies: Vec<BrowserSessionCaptureCookie>,
    #[serde(default)]
    page: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct BrowserSessionCaptureSource {
    browser: String,
    url: String,
    #[serde(default)]
    user_agent: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BrowserSessionCaptureCookie {
    name: String,
    value: String,
    domain: String,
    #[serde(default)]
    expires: f64,
}

#[derive(Debug, Default, Clone, PartialEq)]
struct BrowserSession {
    innertube_context: Option<Value>,
    session_index: Option<String>,
    delegated_session_id: Option<String>,
    data_sync_id: Option<String>,
    user_agent: Option<String>,
}

impl BrowserSession {
    fn has_identity(&self) -> bool {
        self.session_index.is_some()
            || self.delegated_session_id.is_some()
            || self.data_sync_id.is_some()
            || self
                .innertube_context
                .as_ref()
                .and_then(context_page_id)
                .is_some()
    }

    fn require_identity(self) -> Result<Self> {
        if self.has_identity() {
            Ok(self)
        } else {
            Err(HelperError::auth_required(
                "login window session identity is not ready; wait for music.youtube.com to finish loading",
            ))
        }
    }
}

impl AuthConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path).map_err(|error| {
            HelperError::auth_required(format!(
                "cannot read auth file `{}`: {error}",
                path.display()
            ))
        })?;
        let config: Self = serde_json::from_str(&content).map_err(|error| {
            HelperError::auth_required(format!("invalid auth file `{}`: {error}", path.display()))
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != 1 {
            return Err(HelperError::auth_required(format!(
                "unsupported auth schema {}",
                self.schema
            )));
        }
        if self.source.kind != "login-window" {
            return Err(HelperError::auth_required(
                "unsupported auth source; rerun ytm-radio".to_string(),
            ));
        }
        let cookie = self
            .header("cookie")
            .ok_or_else(|| HelperError::auth_required("auth file is missing the cookie header"))?;
        if cookie_value(cookie, "__Secure-3PAPISID")
            .or_else(|| cookie_value(cookie, "SAPISID"))
            .is_none()
        {
            return Err(HelperError::auth_required(
                "auth cookie is missing __Secure-3PAPISID or SAPISID; log in to YouTube Music"
                    .to_string(),
            ));
        }
        Ok(())
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.header("cookie")
            .and_then(|header| cookie_value(header, name))
    }
}

pub fn import_capture(capture_path: &Path, output: &Path) -> Result<AuthConfig> {
    if capture_path == output {
        return Err(HelperError::invalid_request(
            "capture file and auth output must be different paths",
        ));
    }
    let result = (|| {
        let capture = read_browser_session_capture(capture_path)?;
        let config = auth_from_browser_session_capture(&capture)?;
        write_private_json(output, &config)?;
        Ok(config)
    })();
    let removal = fs::remove_file(capture_path);
    match result {
        Ok(config) => removal.map(|()| config).map_err(|error| {
            HelperError::helper_failure(format!(
                "cannot remove imported capture `{}`: {error}",
                capture_path.display()
            ))
        }),
        Err(error) => {
            let _ = removal;
            Err(error)
        }
    }
}

fn read_browser_session_capture(path: &Path) -> Result<BrowserSessionCapture> {
    let content = fs::read_to_string(path).map_err(|error| {
        HelperError::auth_required(format!(
            "cannot read browser-session capture `{}`: {error}",
            path.display()
        ))
    })?;
    let capture: BrowserSessionCapture = serde_json::from_str(&content).map_err(|error| {
        HelperError::auth_required(format!(
            "invalid browser-session capture `{}`: {error}",
            path.display()
        ))
    })?;
    if capture.schema != BROWSER_SESSION_CAPTURE_SCHEMA_VERSION {
        return Err(HelperError::auth_required(format!(
            "unsupported browser-session capture schema {}",
            capture.schema
        )));
    }
    if !is_music_url(&capture.source.url) {
        return Err(HelperError::auth_required(
            "browser-session capture did not originate at music.youtube.com",
        ));
    }
    Ok(capture)
}

fn auth_from_browser_session_capture(capture: &BrowserSessionCapture) -> Result<AuthConfig> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock error: {error}"))?
        .as_secs_f64();
    let mut cookies = BTreeMap::new();
    for cookie in &capture.cookies {
        if is_youtube_domain(&cookie.domain) && !(cookie.expires > 0.0 && cookie.expires <= now) {
            cookies.insert(cookie.name.clone(), cookie.value.clone());
        }
    }
    let session = capture
        .page
        .as_ref()
        .map(browser_session_from_page)
        .unwrap_or_default()
        .require_identity()?;
    let user_agent = session
        .user_agent
        .as_deref()
        .or(capture.source.user_agent.as_deref())
        .unwrap_or(DEFAULT_USER_AGENT)
        .to_string();
    let browser =
        (!capture.source.browser.trim().is_empty()).then_some(capture.source.browser.as_str());
    let mut config = auth_from_cookie_map(
        "login-window",
        browser,
        cookies,
        &user_agent,
        "login window is not authenticated yet; finish signing in to music.youtube.com",
    )?;
    apply_browser_session(&mut config, session);
    Ok(config)
}

fn is_music_url(url: &str) -> bool {
    url == YTM_ORIGIN
        || url.starts_with(&format!("{YTM_ORIGIN}/"))
        || url.starts_with(&format!("{YTM_ORIGIN}?"))
        || url.starts_with(&format!("{YTM_ORIGIN}#"))
}

fn auth_from_cookie_map(
    source_kind: &str,
    browser: Option<&str>,
    cookies: BTreeMap<String, String>,
    user_agent: &str,
    missing_error: &str,
) -> Result<AuthConfig> {
    if !cookies.contains_key("__Secure-3PAPISID") && !cookies.contains_key("SAPISID") {
        return Err(HelperError::auth_required(missing_error));
    }

    let cookie_header = cookies
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ");
    Ok(AuthConfig {
        schema: 1,
        source: AuthSource {
            kind: source_kind.to_string(),
            browser: browser.map(str::to_string),
        },
        headers: BTreeMap::from([
            ("cookie".to_string(), cookie_header),
            ("origin".to_string(), YTM_ORIGIN.to_string()),
            ("user-agent".to_string(), user_agent.to_string()),
            ("x-goog-authuser".to_string(), "0".to_string()),
        ]),
        innertube_context: None,
    })
}

fn browser_session_from_page(value: &Value) -> BrowserSession {
    BrowserSession {
        innertube_context: value
            .get("innertubeContext")
            .filter(|context| context.is_object())
            .cloned(),
        session_index: json_string_field(value, "sessionIndex"),
        delegated_session_id: json_string_field(value, "delegatedSessionId"),
        data_sync_id: json_string_field(value, "dataSyncId"),
        user_agent: json_string_field(value, "userAgent"),
    }
}

fn json_string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_string)
}

fn apply_browser_session(config: &mut AuthConfig, mut session: BrowserSession) {
    if let Some(session_index) = session.session_index.take() {
        config
            .headers
            .insert("x-goog-authuser".to_string(), session_index);
    }

    let page_id = session
        .delegated_session_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| session.innertube_context.as_ref().and_then(context_page_id))
        .or_else(|| session.data_sync_id.as_deref().and_then(data_sync_page_id));
    if let Some(page_id) = page_id {
        config
            .headers
            .insert("x-goog-pageid".to_string(), page_id.clone());
        if let Some(context) = session.innertube_context.as_mut() {
            insert_on_behalf_of_user(context, &page_id);
        }
    }

    if let Some(context) = session.innertube_context.take() {
        config.innertube_context = Some(context);
    }
}

fn context_page_id(context: &Value) -> Option<String> {
    context
        .pointer("/user/onBehalfOfUser")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn data_sync_page_id(data_sync_id: &str) -> Option<String> {
    let first = data_sync_id
        .split("||")
        .next()
        .unwrap_or(data_sync_id)
        .trim();
    (!first.is_empty()
        && first.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        }))
    .then(|| first.to_string())
}

fn insert_on_behalf_of_user(context: &mut Value, page_id: &str) {
    let Some(context_object) = context.as_object_mut() else {
        return;
    };
    if !context_object
        .get("user")
        .map(Value::is_object)
        .unwrap_or(false)
    {
        context_object.insert("user".to_string(), Value::Object(serde_json::Map::new()));
    }
    if let Some(user) = context_object
        .get_mut("user")
        .and_then(Value::as_object_mut)
    {
        user.entry("onBehalfOfUser".to_string())
            .or_insert_with(|| Value::String(page_id.to_string()));
    }
}

fn is_youtube_domain(domain: &str) -> bool {
    let domain = domain.trim_start_matches('.');
    domain == "youtube.com" || domain.ends_with(".youtube.com")
}

fn write_private_json(path: &Path, config: &AuthConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            HelperError::helper_failure(format!("cannot create `{}`: {error}", parent.display()))
        })?;
    }
    let content = serde_json::to_vec_pretty(config).map_err(|error| {
        HelperError::helper_failure(format!("cannot encode auth file: {error}"))
    })?;
    let temporary = temporary_path(path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| {
                HelperError::helper_failure(format!(
                    "cannot create `{}`: {error}",
                    temporary.display()
                ))
            })?;
        file.write_all(&content).map_err(|error| {
            HelperError::helper_failure(format!("cannot write `{}`: {error}", temporary.display()))
        })?;
    }
    #[cfg(not(unix))]
    fs::write(&temporary, content).map_err(|error| {
        HelperError::helper_failure(format!("cannot write `{}`: {error}", temporary.display()))
    })?;
    set_private_permissions(&temporary)?;
    fs::rename(&temporary, path).map_err(|error| {
        HelperError::helper_failure(format!("cannot install `{}`: {error}", path.display()))
    })
}

fn temporary_path(path: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    path.with_file_name(format!(
        ".{}.{}.{stamp}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("auth"),
        std::process::id()
    ))
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        HelperError::helper_failure(format!("cannot protect `{}`: {error}", path.display()))
    })
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then_some(value)
    })
}

#[cfg(test)]
#[path = "auth/tests.rs"]
mod tests;
