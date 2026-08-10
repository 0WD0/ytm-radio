// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn imports_generic_browser_session_capture() {
    let capture = BrowserSessionCapture {
        schema: BROWSER_SESSION_CAPTURE_SCHEMA_VERSION,
        source: BrowserSessionCaptureSource {
            browser: "chrome".to_string(),
            url: YTM_ORIGIN.to_string(),
            user_agent: Some("Captured source UA".to_string()),
        },
        cookies: vec![capture_cookie("__Secure-3PAPISID", ".youtube.com", 0.0)],
        page: Some(json!({
            "innertubeContext": {"client": {"clientName": "WEB_REMIX"}, "user": {}},
            "sessionIndex": "2",
            "delegatedSessionId": "brand-page-id",
            "dataSyncId": null,
            "userAgent": "Captured page UA"
        })),
    };

    let config = auth_from_browser_session_capture(&capture).unwrap();

    assert_eq!(config.source.kind, "login-window");
    assert_eq!(config.source.browser, Some("chrome".to_string()));
    assert!(config.cookie("__Secure-3PAPISID").is_some());
    assert_eq!(config.header("user-agent"), Some("Captured page UA"));
    assert_eq!(config.header("x-goog-authuser"), Some("2"));
    assert_eq!(config.header("x-goog-pageid"), Some("brand-page-id"));
}

#[test]
fn capture_import_uses_source_user_agent_when_page_omits_it() {
    let capture = BrowserSessionCapture {
        schema: BROWSER_SESSION_CAPTURE_SCHEMA_VERSION,
        source: BrowserSessionCaptureSource {
            browser: "firefox".to_string(),
            url: YTM_ORIGIN.to_string(),
            user_agent: Some("Source UA".to_string()),
        },
        cookies: vec![capture_cookie("SAPISID", ".youtube.com", 0.0)],
        page: Some(json!({"sessionIndex": "1"})),
    };

    let config = auth_from_browser_session_capture(&capture).unwrap();

    assert_eq!(config.header("user-agent"), Some("Source UA"));
}

#[test]
fn capture_import_rejects_unrelated_or_expired_cookies() {
    let capture = BrowserSessionCapture {
        schema: BROWSER_SESSION_CAPTURE_SCHEMA_VERSION,
        source: BrowserSessionCaptureSource {
            browser: "chrome".to_string(),
            url: YTM_ORIGIN.to_string(),
            user_agent: None,
        },
        cookies: vec![
            capture_cookie("SAPISID", ".example.com", 0.0),
            capture_cookie("__Secure-3PAPISID", ".youtube.com", 1.0),
        ],
        page: Some(json!({"sessionIndex": "1"})),
    };

    let error = match auth_from_browser_session_capture(&capture) {
        Ok(_) => panic!("unrelated and expired capture cookies must be rejected"),
        Err(error) => error,
    };

    assert_eq!(error.code, "auth-required");
}

#[test]
fn recognizes_music_origin_urls_without_matching_siblings() {
    assert!(is_music_url(YTM_ORIGIN));
    assert!(is_music_url("https://music.youtube.com/?feature=test"));
    assert!(is_music_url("https://music.youtube.com?feature=test"));
    assert!(!is_music_url("https://music.youtube.com.example.com"));
}

#[test]
fn rejects_capture_from_another_origin() {
    let directory = temporary_test_directory();
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("capture.json");
    write_capture(&path, "https://example.com", Vec::new(), Value::Null, None);

    let error = read_browser_session_capture(&path).unwrap_err();

    assert_eq!(error.code, "auth-required");
    assert!(error.message.contains("music.youtube.com"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn import_capture_consumes_private_capture_file() {
    let directory = temporary_test_directory();
    fs::create_dir_all(&directory).unwrap();
    let capture_path = directory.join("capture.json");
    let auth_path = directory.join("auth.json");
    write_capture(
        &capture_path,
        YTM_ORIGIN,
        vec![json!({
            "name": "SAPISID",
            "value": test_value(),
            "domain": ".youtube.com",
            "expires": 0.0
        })],
        json!({"sessionIndex": "1", "userAgent": "Page UA"}),
        Some("Source UA"),
    );

    let config = import_capture(&capture_path, &auth_path).unwrap();

    assert!(!capture_path.exists());
    assert!(AuthConfig::load(&auth_path).is_ok());
    assert_eq!(config.source.browser, Some("browser".to_string()));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn failed_import_consumes_capture_file() {
    let directory = temporary_test_directory();
    fs::create_dir_all(&directory).unwrap();
    let capture_path = directory.join("capture.json");
    let auth_path = directory.join("auth.json");
    fs::write(&capture_path, b"not JSON").unwrap();

    let error = match import_capture(&capture_path, &auth_path) {
        Ok(_) => panic!("invalid capture must be rejected"),
        Err(error) => error,
    };

    assert_eq!(error.code, "auth-required");
    assert!(!capture_path.exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn rejects_old_auth_source_kinds() {
    let config: AuthConfig = serde_json::from_value(json!({
        "schema": 1,
        "source": {"kind": "browser", "browser": "chrome"},
        "headers": {
            "cookie": format!("SAPISID={}", test_value()),
            "origin": YTM_ORIGIN
        }
    }))
    .unwrap();

    let error = config.validate().unwrap_err();

    assert!(error.message.contains("rerun ytm-radio"));
    assert!(error.auth_required);
}

#[test]
fn waits_for_browser_session_identity_before_completing_login() {
    let error = BrowserSession::default().require_identity().unwrap_err();
    assert!(error.message.contains("session identity is not ready"));

    let context_only = BrowserSession {
        innertube_context: Some(json!({
            "client": {"clientName": "WEB_REMIX"},
            "user": {}
        })),
        ..BrowserSession::default()
    };
    assert!(context_only.require_identity().is_err());

    let session = BrowserSession {
        session_index: Some("1".to_string()),
        ..BrowserSession::default()
    };
    assert_eq!(
        session.require_identity().unwrap().session_index.as_deref(),
        Some("1")
    );
}

#[test]
fn applies_browser_session_identity_to_auth_config() {
    let mut config = auth_from_cookie_map(
        "login-window",
        Some("chrome"),
        BTreeMap::from([("SAPISID".to_string(), test_value())]),
        "Browser UA",
        "missing login",
    )
    .unwrap();
    apply_browser_session(
        &mut config,
        BrowserSession {
            innertube_context: Some(json!({
                "client": {"clientName": "WEB_REMIX"},
                "user": {}
            })),
            session_index: Some("2".to_string()),
            delegated_session_id: Some("brand-page-id".to_string()),
            data_sync_id: None,
            user_agent: None,
        },
    );

    assert_eq!(config.header("x-goog-authuser"), Some("2"));
    assert_eq!(config.header("x-goog-pageid"), Some("brand-page-id"));
    assert_eq!(
        config
            .innertube_context
            .as_ref()
            .and_then(|context| context.pointer("/user/onBehalfOfUser"))
            .and_then(Value::as_str),
        Some("brand-page-id")
    );
}

#[cfg(unix)]
#[test]
fn writes_private_login_auth_file() {
    use std::os::unix::fs::PermissionsExt;

    let directory = temporary_test_directory();
    fs::create_dir_all(&directory).unwrap();
    let auth_file = directory.join("auth.json");
    let config = AuthConfig {
        schema: 1,
        source: AuthSource {
            kind: "login-window".to_string(),
            browser: Some("chrome".to_string()),
        },
        headers: BTreeMap::from([
            ("cookie".to_string(), format!("SAPISID={}", test_value())),
            ("origin".to_string(), YTM_ORIGIN.to_string()),
        ]),
        innertube_context: None,
    };

    write_private_json(&auth_file, &config).unwrap();

    assert_eq!(
        fs::metadata(&auth_file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(AuthConfig::load(&auth_file).is_ok());
    fs::remove_dir_all(directory).unwrap();
}

fn capture_cookie(name: &str, domain: &str, expires: f64) -> BrowserSessionCaptureCookie {
    BrowserSessionCaptureCookie {
        name: name.to_string(),
        value: test_value(),
        domain: domain.to_string(),
        expires,
    }
}

fn write_capture(
    path: &Path,
    url: &str,
    cookies: Vec<Value>,
    page: Value,
    user_agent: Option<&str>,
) {
    fs::write(
        path,
        serde_json::to_vec(&json!({
            "schema": BROWSER_SESSION_CAPTURE_SCHEMA_VERSION,
            "source": {
                "browser": "browser",
                "url": url,
                "user_agent": user_agent
            },
            "cookies": cookies,
            "page": page
        }))
        .unwrap(),
    )
    .unwrap();
}

fn test_value() -> String {
    format!("test-{}", std::process::id())
}

fn temporary_test_directory() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "ytm-radio-auth-test-{}-{stamp}-{counter}",
        std::process::id()
    ))
}
