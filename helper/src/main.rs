// SPDX-License-Identifier: GPL-3.0-or-later

mod auth;
mod error;
mod playback;
mod ytmusic;

use auth::{import_capture, AuthConfig};
use error::HelperError;
use playback::resolve_stream;
use serde::Serialize;
use serde_json::json;
#[cfg(any(test, debug_assertions))]
use serde_json::Value;
use std::env;
use std::path::PathBuf;
use std::process;
use ytmusic::{
    add_to_playlist, bootstrap_cache_path, browse, browse_id as browse_detail, clear_account_cache,
    clear_response_cache, continuation, item_library, library, playlist_options, radio, rate,
    search, subscription, track_status, BrowseTarget,
};

const SCHEMA_VERSION: u32 = 1;
const HELPER_PROTOCOL_VERSION: u32 = 1;
const HELPER_VERSION: &str = env!("CARGO_PKG_VERSION");
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Help,
    Version,
    AuthCheck,
    AuthImportCapture {
        capture: PathBuf,
        output: PathBuf,
    },
    Browse(BrowseTarget),
    BrowseId {
        browse_id: String,
        params: Option<String>,
    },
    Continuation {
        token: String,
    },
    Search {
        query: String,
    },
    Rate {
        video_id: String,
        rating: String,
    },
    Radio {
        video_id: String,
    },
    PlaylistOptions {
        video_id: String,
    },
    AddToPlaylist {
        video_id: String,
        playlist_id: String,
    },
    Library {
        video_id: String,
        action: String,
    },
    ItemLibrary {
        browse_id: String,
        params: Option<String>,
        action: String,
    },
    Subscription {
        browse_id: String,
        params: Option<String>,
        action: String,
    },
    TrackStatus {
        video_id: String,
    },
    Stream {
        video_id: String,
        yt_dlp_program: String,
        format: String,
    },
}

impl Command {
    fn mutation_p(&self) -> bool {
        matches!(
            self,
            Self::Rate { .. }
                | Self::AddToPlaylist { .. }
                | Self::Library { .. }
                | Self::ItemLibrary { .. }
                | Self::Subscription { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    command: Command,
    auth_file: Option<PathBuf>,
    limit: usize,
    mock_data: bool,
    initial_only: bool,
    fresh: bool,
    proxy: Option<String>,
}

#[derive(Serialize)]
struct Envelope<T> {
    ok: bool,
    schema: u32,
    protocol: u32,
    #[serde(rename = "helper-version")]
    helper_version: &'static str,
    data: T,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ErrorPayload {
    code: &'static str,
    message: String,
    retryable: bool,
    #[serde(rename = "auth-required")]
    auth_required: bool,
}

#[derive(Serialize)]
struct ErrorEnvelope {
    ok: bool,
    schema: u32,
    protocol: u32,
    #[serde(rename = "helper-version")]
    helper_version: &'static str,
    error: ErrorPayload,
    warnings: Vec<String>,
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match run(args) {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("{error}");
            println!("{}", encode_error(&error));
            process::exit(1);
        }
    }
}

fn encode_error(error: &HelperError) -> String {
    serde_json::to_string(&ErrorEnvelope {
        ok: false,
        schema: SCHEMA_VERSION,
        protocol: HELPER_PROTOCOL_VERSION,
        helper_version: HELPER_VERSION,
        error: ErrorPayload {
            code: error.code,
            message: error.message.clone(),
            retryable: error.retryable,
            auth_required: error.auth_required,
        },
        warnings: Vec::new(),
    })
    .expect("error envelope is serializable")
}

fn run<I>(args: I) -> error::Result<String>
where
    I: IntoIterator,
    I::Item: Into<String>,
{
    let options = parse_args(args).map_err(HelperError::invalid_request)?;
    let proxy = options.proxy.as_deref();
    if options.fresh && !options.mock_data {
        let auth_path = required_auth_path(&options)?;
        clear_response_cache(&bootstrap_cache_path(auth_path))?;
    }
    let data = match &options.command {
        Command::Help => json!({ "usage": usage() }),
        Command::Version => {
            json!({
                "schema": SCHEMA_VERSION,
                "protocol": HELPER_PROTOCOL_VERSION,
                "helper-version": HELPER_VERSION
            })
        }
        Command::AuthCheck => {
            let path = required_auth_path(&options)?;
            let config = AuthConfig::load(path)?;
            json!({
                "auth": {
                    "configured": true,
                    "source": config.source,
                    "path": path
                }
            })
        }
        Command::AuthImportCapture { capture, output } => {
            let config = import_capture(capture, output)?;
            if let Err(error) = clear_account_cache(&bootstrap_cache_path(output)) {
                eprintln!("ytm-radio-helper cache-invalidation-error={error}");
            }
            json!({
                "auth": {
                    "configured": true,
                    "source": config.source,
                    "path": output
                }
            })
        }
        #[cfg(any(test, debug_assertions))]
        Command::Browse(target) if options.mock_data => {
            mock_browse(target, options.limit, options.initial_only)
        }
        Command::Browse(target) => {
            let (auth, cache_path) = load_auth_with_cache_path(&options)?;
            browse(
                target,
                options.limit,
                &auth,
                options.initial_only,
                Some(&cache_path),
                proxy,
            )?
        }
        #[cfg(any(test, debug_assertions))]
        Command::BrowseId { browse_id, .. } if options.mock_data => {
            mock_browse_id(browse_id, options.limit)
        }
        Command::BrowseId { browse_id, params } => {
            let (auth, cache_path) = load_auth_with_cache_path(&options)?;
            browse_detail(
                browse_id,
                params.as_deref(),
                options.limit,
                &auth,
                Some(&cache_path),
                proxy,
            )?
        }
        #[cfg(any(test, debug_assertions))]
        Command::Continuation { token } if options.mock_data => {
            mock_continuation(token, options.limit)
        }
        Command::Continuation { token } => {
            let (auth, cache_path) = load_auth_with_cache_path(&options)?;
            continuation(token, options.limit, &auth, Some(&cache_path), proxy)?
        }
        #[cfg(any(test, debug_assertions))]
        Command::Search { query } if options.mock_data => mock_search(query, options.limit),
        Command::Search { query } => {
            let (auth, cache_path) = load_auth_with_cache_path(&options)?;
            search(query, options.limit, &auth, Some(&cache_path), proxy)?
        }
        #[cfg(any(test, debug_assertions))]
        Command::Rate { video_id, rating } if options.mock_data => {
            json!({ "video-id": video_id, "rating": rating })
        }
        Command::Rate { video_id, rating } => {
            let (auth, cache_path) = load_auth_with_cache_path(&options)?;
            rate(video_id, rating, &auth, Some(&cache_path), proxy)?
        }
        #[cfg(any(test, debug_assertions))]
        Command::Radio { video_id } if options.mock_data => mock_radio(video_id, options.limit),
        Command::Radio { video_id } => {
            let (auth, cache_path) = load_auth_with_cache_path(&options)?;
            radio(video_id, options.limit, &auth, Some(&cache_path), proxy)?
        }
        #[cfg(any(test, debug_assertions))]
        Command::PlaylistOptions { video_id } if options.mock_data => {
            mock_playlist_options(video_id)
        }
        Command::PlaylistOptions { video_id } => {
            let (auth, cache_path) = load_auth_with_cache_path(&options)?;
            playlist_options(video_id, &auth, Some(&cache_path), proxy)?
        }
        #[cfg(any(test, debug_assertions))]
        Command::AddToPlaylist {
            video_id,
            playlist_id,
        } if options.mock_data => {
            json!({ "video-id": video_id, "playlist-id": playlist_id })
        }
        Command::AddToPlaylist {
            video_id,
            playlist_id,
        } => {
            let (auth, cache_path) = load_auth_with_cache_path(&options)?;
            add_to_playlist(video_id, playlist_id, &auth, Some(&cache_path), proxy)?
        }
        #[cfg(any(test, debug_assertions))]
        Command::Library { video_id, action } if options.mock_data => {
            json!({
                "video-id": video_id,
                "in-library": action != "remove",
                "like-status": "like",
                "action": action,
                "changed": true
            })
        }
        Command::Library { video_id, action } => {
            let (auth, cache_path) = load_auth_with_cache_path(&options)?;
            library(video_id, action, &auth, Some(&cache_path), proxy)?
        }
        #[cfg(any(test, debug_assertions))]
        Command::ItemLibrary {
            browse_id, action, ..
        } if options.mock_data => {
            json!({
                "browse-id": browse_id,
                "in-library": action != "remove",
                "action": action,
                "changed": true
            })
        }
        Command::ItemLibrary {
            browse_id,
            params,
            action,
        } => {
            let (auth, cache_path) = load_auth_with_cache_path(&options)?;
            item_library(
                browse_id,
                params.as_deref(),
                action,
                &auth,
                Some(&cache_path),
                proxy,
            )?
        }
        #[cfg(any(test, debug_assertions))]
        Command::Subscription {
            browse_id, action, ..
        } if options.mock_data => {
            json!({
                "browse-id": browse_id,
                "subscribed": action != "unsubscribe",
                "action": action,
                "changed": true
            })
        }
        Command::Subscription {
            browse_id,
            params,
            action,
        } => {
            let (auth, cache_path) = load_auth_with_cache_path(&options)?;
            subscription(
                browse_id,
                params.as_deref(),
                action,
                &auth,
                Some(&cache_path),
                proxy,
            )?
        }
        #[cfg(any(test, debug_assertions))]
        Command::TrackStatus { video_id } if options.mock_data => {
            json!({ "video-id": video_id, "in-library": true, "like-status": "like" })
        }
        Command::TrackStatus { video_id } => {
            let (auth, cache_path) = load_auth_with_cache_path(&options)?;
            track_status(video_id, &auth, Some(&cache_path), proxy)?
        }
        #[cfg(any(test, debug_assertions))]
        Command::Stream { video_id, .. } if options.mock_data => {
            json!({ "url": format!("https://media.example/{video_id}") })
        }
        Command::Stream {
            video_id,
            yt_dlp_program,
            format,
        } => {
            let auth = AuthConfig::load(required_auth_path(&options)?)?;
            json!({
                "url": resolve_stream(video_id, &auth, yt_dlp_program, format, proxy)?
            })
        }
    };
    if options.command.mutation_p() && !options.mock_data {
        let auth_path = required_auth_path(&options)?;
        if let Err(error) = clear_response_cache(&bootstrap_cache_path(auth_path)) {
            eprintln!("ytm-radio-helper cache-invalidation-error={error}");
        }
    }
    serde_json::to_string(&Envelope {
        ok: true,
        schema: SCHEMA_VERSION,
        protocol: HELPER_PROTOCOL_VERSION,
        helper_version: HELPER_VERSION,
        data,
        warnings: Vec::new(),
    })
    .map_err(|error| HelperError::helper_failure(format!("cannot encode response: {error}")))
}

fn required_auth_path(options: &Options) -> error::Result<&std::path::Path> {
    options
        .auth_file
        .as_deref()
        .ok_or_else(|| HelperError::invalid_request("missing --auth FILE"))
}

fn load_auth_with_cache_path(options: &Options) -> error::Result<(AuthConfig, PathBuf)> {
    let auth_path = required_auth_path(options)?;
    Ok((
        AuthConfig::load(auth_path)?,
        bootstrap_cache_path(auth_path),
    ))
}

fn parse_args<I>(args: I) -> Result<Options, String>
where
    I: IntoIterator,
    I::Item: Into<String>,
{
    let mut args: Vec<String> = args.into_iter().map(Into::into).collect();
    if args.is_empty() {
        return Err(usage());
    }

    let command = match args.remove(0).as_str() {
        "version" => Command::Version,
        "auth" => parse_auth_command(&mut args)?,
        "browse" => parse_browse_command(&mut args)?,
        "browse-id" => parse_browse_id_command(&mut args)?,
        "continuation" => parse_continuation_command(&mut args)?,
        "search" => parse_search_command(&mut args)?,
        "rate" => parse_rate_command(&mut args)?,
        "radio" => parse_video_id_command(&mut args, "radio")?,
        "playlist-options" => parse_video_id_command(&mut args, "playlist-options")?,
        "add-to-playlist" => parse_add_to_playlist_command(&mut args)?,
        "library" => parse_library_command(&mut args)?,
        "item-library" => parse_item_library_command(&mut args)?,
        "subscription" => parse_subscription_command(&mut args)?,
        "track-status" => parse_track_status_command(&mut args)?,
        "stream" => Command::Stream {
            video_id: parse_required_video_id(&mut args)?,
            yt_dlp_program: "yt-dlp".to_string(),
            format: "bestaudio/best".to_string(),
        },
        "help" | "--help" | "-h" => Command::Help,
        other => return Err(format!("unknown command `{other}`")),
    };

    let mut auth_file = None;
    let mut limit = 100;
    let mut limit_specified = false;
    #[cfg(any(test, debug_assertions))]
    let mut mock_data = false;
    #[cfg(not(any(test, debug_assertions)))]
    let mock_data = false;
    let mut output = None;
    let mut capture = None;
    let mut browse_params = None;
    let mut initial_only = false;
    let mut fresh = false;
    let mut proxy = None;
    let mut yt_dlp_program = None;
    let mut stream_format = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--auth" => auth_file = Some(PathBuf::from(option_value(&args, &mut index)?)),
            "--limit" => {
                limit_specified = true;
                let value = option_value(&args, &mut index)?;
                limit = value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid limit `{value}`"))?;
                if limit == 0 {
                    return Err("limit must be greater than zero".to_string());
                }
            }
            #[cfg(any(test, debug_assertions))]
            "--mock" => mock_data = true,
            "--output" => output = Some(PathBuf::from(option_value(&args, &mut index)?)),
            "--capture" => capture = Some(PathBuf::from(option_value(&args, &mut index)?)),
            "--params" => {
                let value = option_value(&args, &mut index)?;
                if value.trim().is_empty() {
                    return Err("browse params must not be empty".to_string());
                }
                browse_params = Some(value.to_string());
            }
            "--initial-only" => initial_only = true,
            "--fresh" => fresh = true,
            "--proxy" => {
                let value = option_value(&args, &mut index)?.trim();
                if value.is_empty() {
                    return Err("proxy URL must not be empty".to_string());
                }
                proxy = Some(value.to_string());
            }
            "--yt-dlp-program" => {
                let value = option_value(&args, &mut index)?.trim();
                if value.is_empty() {
                    return Err("yt-dlp program must not be empty".to_string());
                }
                yt_dlp_program = Some(value.to_string());
            }
            "--format" => {
                let value = option_value(&args, &mut index)?.trim();
                if value.is_empty() {
                    return Err("stream format must not be empty".to_string());
                }
                stream_format = Some(value.to_string());
            }
            "--browser" | "--port" | "--profile-dir" | "--timeout-secs" | "--restart-running" => {
                return Err("browser options are owned by browser-session".to_string());
            }
            other => return Err(format!("unknown option `{other}`")),
        }
        index += 1;
    }

    if !matches!(&command, Command::Stream { .. })
        && (yt_dlp_program.is_some() || stream_format.is_some())
    {
        return Err("stream options require a stream action".to_string());
    }
    if capture.is_some() && !matches!(&command, Command::AuthImportCapture { .. }) {
        return Err("capture options require auth import-capture".to_string());
    }

    let command = match command {
        Command::AuthImportCapture { .. } => {
            if limit_specified
                || proxy.is_some()
                || browse_params.is_some()
                || initial_only
                || fresh
                || mock_data
                || auth_file.is_some()
            {
                return Err("capture import accepts only capture and output options".to_string());
            }
            let capture = capture.ok_or_else(|| "missing --capture FILE".to_string())?;
            let output = output.ok_or_else(|| "missing --output FILE".to_string())?;
            Command::AuthImportCapture { capture, output }
        }
        Command::Version => {
            if limit_specified
                || output.is_some()
                || browse_params.is_some()
                || initial_only
                || fresh
                || mock_data
                || auth_file.is_some()
                || proxy.is_some()
            {
                return Err("version does not accept request options".to_string());
            }
            Command::Version
        }
        Command::BrowseId { browse_id, .. } => {
            if output.is_some() || initial_only {
                return Err("auth options require an auth action".to_string());
            }
            Command::BrowseId {
                browse_id,
                params: browse_params,
            }
        }
        Command::ItemLibrary {
            browse_id, action, ..
        } => {
            if output.is_some() || initial_only {
                return Err("auth options require an auth action".to_string());
            }
            Command::ItemLibrary {
                browse_id,
                params: browse_params,
                action,
            }
        }
        Command::Subscription {
            browse_id, action, ..
        } => {
            if output.is_some() || initial_only {
                return Err("auth options require an auth action".to_string());
            }
            Command::Subscription {
                browse_id,
                params: browse_params,
                action,
            }
        }
        Command::Browse(target) => {
            if output.is_some() || browse_params.is_some() {
                return Err("auth options require an auth action".to_string());
            }
            if initial_only && !matches!(target, BrowseTarget::Home) {
                return Err("--initial-only is only supported for browse home".to_string());
            }
            Command::Browse(target)
        }
        Command::AuthCheck => {
            if proxy.is_some() {
                return Err("proxy options require a YouTube Music request action".to_string());
            }
            if output.is_some() {
                return Err("auth options require an auth action".to_string());
            }
            if browse_params.is_some() || initial_only || fresh || mock_data {
                return Err("browse options require a browse action".to_string());
            }
            Command::AuthCheck
        }
        Command::Stream { video_id, .. } => {
            if output.is_some() || browse_params.is_some() || initial_only || fresh {
                return Err("stream accepts only auth, proxy, and yt-dlp options".to_string());
            }
            Command::Stream {
                video_id,
                yt_dlp_program: yt_dlp_program.unwrap_or_else(|| "yt-dlp".to_string()),
                format: stream_format.unwrap_or_else(|| "bestaudio/best".to_string()),
            }
        }
        other => {
            if output.is_some() || browse_params.is_some() || initial_only {
                return Err("auth options require an auth action".to_string());
            }
            other
        }
    };

    Ok(Options {
        command,
        auth_file,
        limit,
        mock_data,
        initial_only,
        fresh,
        proxy,
    })
}

fn parse_auth_command(args: &mut Vec<String>) -> Result<Command, String> {
    let Some(action) = args.first().cloned() else {
        return Err("expected an auth action".to_string());
    };
    args.remove(0);
    match action.as_str() {
        "check" => Ok(Command::AuthCheck),
        "import-capture" => Ok(Command::AuthImportCapture {
            capture: PathBuf::new(),
            output: PathBuf::new(),
        }),
        other => Err(format!("unknown auth action `{other}`")),
    }
}

fn parse_browse_command(args: &mut Vec<String>) -> Result<Command, String> {
    let Some(target) = args.first().cloned() else {
        return Err("expected browse target".to_string());
    };
    args.remove(0);
    Ok(Command::Browse(match target.as_str() {
        "home" => BrowseTarget::Home,
        "explore" => BrowseTarget::Explore,
        "library" => BrowseTarget::Library,
        "library-songs" => BrowseTarget::LibrarySongs,
        "library-albums" => BrowseTarget::LibraryAlbums,
        "library-artists" => BrowseTarget::LibraryArtists,
        "library-playlists" => BrowseTarget::LibraryPlaylists,
        "liked" => BrowseTarget::Liked,
        other => return Err(format!("unknown browse target `{other}`")),
    }))
}

fn parse_browse_id_command(args: &mut Vec<String>) -> Result<Command, String> {
    let Some(browse_id) = args.first().cloned() else {
        return Err("expected browse id".to_string());
    };
    args.remove(0);
    if browse_id.trim().is_empty() {
        return Err("browse id must not be empty".to_string());
    }
    Ok(Command::BrowseId {
        browse_id,
        params: None,
    })
}

fn parse_continuation_command(args: &mut Vec<String>) -> Result<Command, String> {
    let Some(token) = args.first().cloned() else {
        return Err("expected continuation token".to_string());
    };
    args.remove(0);
    if token.trim().is_empty() {
        return Err("continuation token must not be empty".to_string());
    }
    Ok(Command::Continuation { token })
}

fn parse_search_command(args: &mut Vec<String>) -> Result<Command, String> {
    let Some(query) = args.first().cloned() else {
        return Err("expected search query".to_string());
    };
    args.remove(0);
    if query.trim().is_empty() {
        return Err("search query must not be empty".to_string());
    }
    Ok(Command::Search { query })
}

fn parse_rate_command(args: &mut Vec<String>) -> Result<Command, String> {
    let video_id = parse_required_video_id(args)?;
    let Some(rating) = args.first().cloned() else {
        return Err("expected rating".to_string());
    };
    args.remove(0);
    match rating.as_str() {
        "like" | "dislike" | "indifferent" => Ok(Command::Rate { video_id, rating }),
        other => Err(format!("unknown rating `{other}`")),
    }
}

fn parse_video_id_command(args: &mut Vec<String>, command: &str) -> Result<Command, String> {
    let video_id = parse_required_video_id(args)?;
    match command {
        "radio" => Ok(Command::Radio { video_id }),
        "playlist-options" => Ok(Command::PlaylistOptions { video_id }),
        other => Err(format!("unknown video id command `{other}`")),
    }
}

fn parse_add_to_playlist_command(args: &mut Vec<String>) -> Result<Command, String> {
    let video_id = parse_required_video_id(args)?;
    let Some(playlist_id) = args.first().cloned() else {
        return Err("expected playlist id".to_string());
    };
    args.remove(0);
    if playlist_id.trim().is_empty() {
        return Err("playlist id must not be empty".to_string());
    }
    Ok(Command::AddToPlaylist {
        video_id,
        playlist_id,
    })
}

fn parse_library_command(args: &mut Vec<String>) -> Result<Command, String> {
    let video_id = parse_required_video_id(args)?;
    let Some(action) = args.first().cloned() else {
        return Err("expected library action".to_string());
    };
    args.remove(0);
    match action.as_str() {
        "toggle" | "save" | "remove" => Ok(Command::Library { video_id, action }),
        other => Err(format!("unknown library action `{other}`")),
    }
}

fn parse_item_library_command(args: &mut Vec<String>) -> Result<Command, String> {
    let browse_id = parse_required_browse_id(args)?;
    let action = parse_required_action(args, "library")?;
    match action.as_str() {
        "toggle" | "save" | "remove" => Ok(Command::ItemLibrary {
            browse_id,
            params: None,
            action,
        }),
        other => Err(format!("unknown library action `{other}`")),
    }
}

fn parse_subscription_command(args: &mut Vec<String>) -> Result<Command, String> {
    let browse_id = parse_required_browse_id(args)?;
    let action = parse_required_action(args, "subscription")?;
    match action.as_str() {
        "toggle" | "subscribe" | "unsubscribe" => Ok(Command::Subscription {
            browse_id,
            params: None,
            action,
        }),
        other => Err(format!("unknown subscription action `{other}`")),
    }
}

fn parse_track_status_command(args: &mut Vec<String>) -> Result<Command, String> {
    Ok(Command::TrackStatus {
        video_id: parse_required_video_id(args)?,
    })
}

fn parse_required_browse_id(args: &mut Vec<String>) -> Result<String, String> {
    let Some(browse_id) = args.first().cloned() else {
        return Err("expected browse id".to_string());
    };
    args.remove(0);
    if browse_id.trim().is_empty() {
        return Err("browse id must not be empty".to_string());
    }
    Ok(browse_id)
}

fn parse_required_action(args: &mut Vec<String>, command: &str) -> Result<String, String> {
    let Some(action) = args.first().cloned() else {
        return Err(format!("expected {command} action"));
    };
    args.remove(0);
    Ok(action)
}

fn parse_required_video_id(args: &mut Vec<String>) -> Result<String, String> {
    let Some(video_id) = args.first().cloned() else {
        return Err("expected video id".to_string());
    };
    args.remove(0);
    if !is_youtube_video_id(&video_id) {
        return Err("video id must be an 11-character YouTube video id".to_string());
    }
    Ok(video_id)
}

fn is_youtube_video_id(video_id: &str) -> bool {
    video_id.len() == 11
        && video_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn option_value<'a>(args: &'a [String], index: &mut usize) -> Result<&'a str, String> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| "missing option value".to_string())
}

fn usage() -> String {
    [
        "usage:",
        "  ytm-radio-helper version",
        "  ytm-radio-helper auth check --auth FILE",
        "  ytm-radio-helper auth import-capture --capture FILE --output FILE",
        "  ytm-radio-helper browse home --auth FILE [--limit N] [--initial-only] [--fresh]",
        "  ytm-radio-helper browse explore|library|library-songs|library-albums|library-artists|library-playlists|liked --auth FILE [--limit N] [--fresh]",
        "  ytm-radio-helper browse-id BROWSE_ID --auth FILE [--params PARAMS] [--limit N] [--fresh]",
        "  ytm-radio-helper continuation TOKEN --auth FILE [--limit N] [--fresh]",
        "  ytm-radio-helper search QUERY --auth FILE [--limit N] [--fresh]",
        "  ytm-radio-helper rate VIDEO_ID like|dislike|indifferent --auth FILE",
        "  ytm-radio-helper radio VIDEO_ID --auth FILE [--limit N]",
        "  ytm-radio-helper playlist-options VIDEO_ID --auth FILE",
        "  ytm-radio-helper add-to-playlist VIDEO_ID PLAYLIST_ID --auth FILE",
        "  ytm-radio-helper library VIDEO_ID toggle|save|remove --auth FILE",
        "  ytm-radio-helper item-library BROWSE_ID toggle|save|remove --auth FILE [--params PARAMS]",
        "  ytm-radio-helper subscription BROWSE_ID toggle|subscribe|unsubscribe --auth FILE [--params PARAMS]",
        "  ytm-radio-helper track-status VIDEO_ID --auth FILE",
        "  ytm-radio-helper stream VIDEO_ID --auth FILE [--yt-dlp-program PROGRAM] [--format FORMAT] [--proxy URL]",
        "",
        "options:",
        "  --proxy URL  proxy YouTube Music requests",
    ]
    .join("\n")
}

#[cfg(any(test, debug_assertions))]
fn mock_browse(target: &BrowseTarget, limit: usize, initial_only: bool) -> Value {
    if matches!(target, BrowseTarget::Home) {
        return mock_home_browse(limit, initial_only);
    }
    if matches!(target, BrowseTarget::Explore) {
        return mock_explore_browse(limit);
    }
    if matches!(target, BrowseTarget::Library) {
        return mock_library_browse(limit);
    }
    let (id, kind, title, url, track_id, track_title) = match target {
        BrowseTarget::Home => (
            "ytm:home:mock",
            "youtube-music-home",
            "Mock Home",
            "ytm://home/mock",
            "mock-home-track",
            "Mock Home Track",
        ),
        BrowseTarget::Explore => unreachable!(),
        BrowseTarget::Library => unreachable!(),
        BrowseTarget::LibrarySongs => (
            "ytm:library:songs",
            "youtube-music-library",
            "Library Songs",
            "ytm://library/songs",
            "mock-library-track",
            "Mock Library Track",
        ),
        BrowseTarget::LibraryAlbums => (
            "ytm:library:albums",
            "youtube-music-library",
            "Library Albums",
            "ytm://library/albums",
            "mock-album-track",
            "Mock Album Track",
        ),
        BrowseTarget::LibraryArtists => (
            "ytm:library:artists",
            "youtube-music-library",
            "Library Artists",
            "ytm://library/artists",
            "mock-artist-track",
            "Mock Artist Track",
        ),
        BrowseTarget::LibraryPlaylists => (
            "ytm:library:playlists",
            "youtube-music-library",
            "Library Playlists",
            "ytm://library/playlists",
            "mock-playlist-track",
            "Mock Playlist Track",
        ),
        BrowseTarget::Liked => (
            "ytm:library:liked",
            "youtube-music-liked",
            "Liked Music",
            "ytm://library/liked",
            "mock-liked-track",
            "Mock Liked Track",
        ),
    };
    let mut items = vec![json!({
        "type": "track",
        "id": track_id,
        "title": track_title,
        "url": format!("https://music.youtube.com/watch?v={track_id}"),
        "artist": "Mock Artist",
        "album": "Mock Album",
        "duration": 180,
        "thumbnail-url": null
    })];
    if limit.clamp(1, 2) > 1 {
        items.push(json!({
            "type": "playlist",
            "id": "mock-playlist",
            "title": "Mock Playlist",
            "url": "https://music.youtube.com/playlist?list=mock-playlist",
            "browse-id": "VLmock-playlist",
            "playlist-id": "mock-playlist",
            "subtitle": "Playlist",
            "thumbnail-url": null
        }));
    }
    json!({
        "sources": [{
            "id": id,
            "kind": kind,
            "title": title,
            "url": url,
            "items": items,
            "continuation": null
        }]
    })
}

#[cfg(any(test, debug_assertions))]
fn mock_search(query: &str, limit: usize) -> Value {
    let items = vec![
        json!({
            "type": "track",
            "id": "mock-search-track",
            "title": format!("{query} result"),
            "url": "https://music.youtube.com/watch?v=mock-search-track",
            "artist": "Mock Artist",
            "album": "Mock Album",
            "duration": 180,
            "thumbnail-url": null
        }),
        json!({
            "type": "album",
            "id": "mock-search-album",
            "title": format!("{query} album"),
            "url": "https://music.youtube.com/browse/mock-search-album",
            "browse-id": "mock-search-album",
            "playlist-id": null,
            "subtitle": "Album - Mock Artist",
            "thumbnail-url": null
        }),
        json!({
            "type": "artist",
            "id": "UCmock-search-artist",
            "title": format!("{query} artist"),
            "url": "https://music.youtube.com/browse/UCmock-search-artist",
            "browse-id": "UCmock-search-artist",
            "playlist-id": null,
            "subtitle": "Artist",
            "thumbnail-url": null
        }),
        json!({
            "type": "podcast",
            "id": "MPSPmock-search-podcast",
            "title": format!("{query} podcast"),
            "url": "https://music.youtube.com/browse/MPSPmock-search-podcast",
            "browse-id": "MPSPmock-search-podcast",
            "playlist-id": null,
            "subtitle": "Podcast",
            "thumbnail-url": null
        }),
        json!({
            "type": "playlist",
            "id": "mock-search-playlist",
            "title": format!("{query} playlist"),
            "url": "https://music.youtube.com/playlist?list=mock-search-playlist",
            "browse-id": "VLmock-search-playlist",
            "playlist-id": "mock-search-playlist",
            "subtitle": "Playlist",
            "thumbnail-url": null
        }),
    ];
    json!({
        "sources": [{
            "id": "ytm:search:mock",
            "kind": "youtube-music-search",
            "title": format!("Search: {query}"),
            "url": "ytm://search/mock",
            "items": items.into_iter().take(limit).collect::<Vec<_>>(),
            "continuation": null
        }]
    })
}

#[cfg(any(test, debug_assertions))]
fn mock_radio(video_id: &str, limit: usize) -> Value {
    let items = (0..limit.min(3))
        .map(|index| {
            let id = if index == 0 {
                video_id.to_string()
            } else {
                format!("mockRADIO{index:02}")
            };
            json!({
                "type": "track",
                "id": id,
                "title": format!("Mock Radio Track {}", index + 1),
                "url": format!("https://music.youtube.com/watch?v={id}"),
                "artist": "Mock Artist",
                "album": null,
                "duration": 180,
                "thumbnail-url": null,
                "like-status": null,
                "in-library": false
            })
        })
        .collect::<Vec<_>>();
    json!({
        "sources": [{
            "id": format!("ytm:radio:{video_id}"),
            "kind": "youtube-music-radio",
            "title": format!("Radio: {video_id}"),
            "url": format!("https://music.youtube.com/watch?v={video_id}&list=RDAMVM{video_id}"),
            "items": items,
            "continuation": "mock-radio-continuation"
        }]
    })
}

#[cfg(any(test, debug_assertions))]
fn mock_playlist_options(_video_id: &str) -> Value {
    json!({
        "title": "Add to playlist",
        "options": [
            {
                "playlist-id": "mock-playlist-1",
                "title": "Mock Playlist 1",
                "subtitle": "Private",
                "thumbnail-url": null,
                "selected": false,
                "privacy-status": "PRIVATE"
            },
            {
                "playlist-id": "mock-playlist-2",
                "title": "Mock Playlist 2",
                "subtitle": "Public",
                "thumbnail-url": null,
                "selected": false,
                "privacy-status": "PUBLIC"
            }
        ],
        "can-create-playlist": false
    })
}

#[cfg(any(test, debug_assertions))]
fn mock_library_browse(limit: usize) -> Value {
    let songs = mock_browse(&BrowseTarget::LibrarySongs, limit, false)
        .pointer("/sources/0")
        .cloned()
        .unwrap_or(Value::Null);
    let albums = json!({
        "id": "ytm:library:albums",
        "kind": "youtube-music-library-section",
        "title": "Albums",
        "url": "ytm://library/albums",
        "items": [{
            "type": "album",
            "id": "mock-library-album",
            "title": "Mock Album",
            "url": "https://music.youtube.com/browse/mock-library-album",
            "browse-id": "mock-library-album",
            "playlist-id": null,
            "subtitle": "Mock Artist",
            "thumbnail-url": null
        }],
        "continuation": null
    });
    let playlists = json!({
        "id": "ytm:library:playlists",
        "kind": "youtube-music-library-section",
        "title": "Playlists",
        "url": "ytm://library/playlists",
        "items": [{
            "type": "playlist",
            "id": "mock-library-playlist",
            "title": "Mock Playlist",
            "url": "https://music.youtube.com/playlist?list=mock-library-playlist",
            "browse-id": "VLmock-library-playlist",
            "playlist-id": "mock-library-playlist",
            "subtitle": "Playlist",
            "thumbnail-url": null
        }],
        "continuation": null
    });
    json!({ "sources": [songs, albums, playlists] })
}

#[cfg(any(test, debug_assertions))]
fn mock_home_browse(limit: usize, initial_only: bool) -> Value {
    let item_limit = limit.clamp(1, 2);
    let mut listen_again = vec![json!({
        "type": "track",
        "id": "mock-home-track",
        "title": "Mock Home Track",
        "url": "https://music.youtube.com/watch?v=mock-home-track",
        "artist": "Mock Artist",
        "album": "Mock Album",
        "duration": 180,
        "thumbnail-url": null
    })];
    if item_limit > 1 {
        listen_again.push(json!({
            "type": "album",
            "id": "mock-album",
            "title": "Mock Album",
            "url": "https://music.youtube.com/browse/mock-album",
            "browse-id": "mock-album",
            "playlist-id": null,
            "subtitle": "Mock Artist",
            "thumbnail-url": null
        }));
    }
    let mut sources = vec![json!({
            "id": "ytm:home:mock:listen-again",
            "kind": "youtube-music-home-section",
            "title": "Listen again",
            "url": "ytm://home/mock",
            "items": listen_again,
            "continuation": null
    })];
    if !initial_only {
        sources.push(json!({
            "id": "ytm:home:mock:mixed-for-you",
            "kind": "youtube-music-home-section",
            "title": "Mixed for you",
            "url": "ytm://home/mock",
            "items": [{
                "type": "playlist",
                "id": "mock-mix",
                "title": "Mock Mix",
                "url": "https://music.youtube.com/playlist?list=mock-mix",
                "browse-id": "VLmock-mix",
                "playlist-id": "mock-mix",
                "subtitle": "Playlist",
                "thumbnail-url": null
            }],
            "continuation": null
        }));
    }
    json!({
        "sources": sources,
        "continuation": if initial_only { Some("mock-home-more") } else { None::<&str> }
    })
}

#[cfg(any(test, debug_assertions))]
fn mock_continuation(token: &str, _limit: usize) -> Value {
    if token == "mock-home-more" {
        json!({
            "sources": [{
                "id": "ytm:home:mock:mixed-for-you",
                "kind": "youtube-music-home-section",
                "title": "Mixed for you",
                "url": "ytm://home/mock",
                "items": [{
                    "type": "playlist",
                    "id": "mock-mix",
                    "title": "Mock Mix",
                    "url": "https://music.youtube.com/playlist?list=mock-mix",
                    "browse-id": "VLmock-mix",
                    "playlist-id": "mock-mix",
                    "subtitle": "Playlist",
                    "thumbnail-url": null
                }],
                "continuation": null
            }],
            "continuation": null
        })
    } else {
        json!({
            "sources": [],
            "continuation": null
        })
    }
}

#[cfg(any(test, debug_assertions))]
fn mock_explore_browse(limit: usize) -> Value {
    let item_limit = limit.clamp(1, 2);
    let mut new_releases = vec![json!({
        "type": "album",
        "id": "mock-explore-album",
        "title": "Mock New Release",
        "url": "https://music.youtube.com/browse/mock-explore-album",
        "browse-id": "mock-explore-album",
        "playlist-id": null,
        "subtitle": "Mock Artist",
        "thumbnail-url": null
    })];
    if item_limit > 1 {
        new_releases.push(json!({
            "type": "playlist",
            "id": "mock-explore-playlist",
            "title": "Mock Trending Playlist",
            "url": "https://music.youtube.com/playlist?list=mock-explore-playlist",
            "browse-id": "VLmock-explore-playlist",
            "playlist-id": "mock-explore-playlist",
            "subtitle": "Playlist",
            "thumbnail-url": null
        }));
    }
    json!({
        "sources": [{
            "id": "ytm:explore:mock:new-releases",
            "kind": "youtube-music-explore-section",
            "title": "New releases",
            "url": "ytm://explore/mock",
            "items": new_releases,
            "continuation": null
        }, {
            "id": "ytm:explore:mock:charts",
            "kind": "youtube-music-explore-section",
            "title": "Charts",
            "url": "ytm://explore/mock",
            "items": [{
                "type": "playlist",
                "id": "mock-chart",
                "title": "Mock Chart",
                "url": "https://music.youtube.com/playlist?list=mock-chart",
                "browse-id": "VLmock-chart",
                "playlist-id": "mock-chart",
                "subtitle": "Playlist",
                "thumbnail-url": null
            }],
            "continuation": null
        }]
    })
}

#[cfg(any(test, debug_assertions))]
fn mock_browse_id(browse_id: &str, limit: usize) -> Value {
    let item_limit = limit.clamp(1, 2);
    let kind = if browse_id.starts_with("MPRE") {
        "youtube-music-album"
    } else if browse_id.starts_with("UC") {
        "youtube-music-artist"
    } else if browse_id.starts_with("VL")
        || browse_id.starts_with("PL")
        || browse_id.starts_with("RD")
    {
        "youtube-music-playlist"
    } else {
        "youtube-music-detail"
    };
    let mut items = vec![json!({
        "type": "track",
        "id": "mock-detail-track",
        "title": "Mock Detail Track",
        "url": "https://music.youtube.com/watch?v=mock-detail-track",
        "artist": "Mock Artist",
        "album": "Mock Detail",
        "duration": 181,
        "thumbnail-url": null
    })];
    if item_limit > 1 {
        items.push(json!({
            "type": "track",
            "id": "mock-detail-track-2",
            "title": "Mock Detail Track 2",
            "url": "https://music.youtube.com/watch?v=mock-detail-track-2",
            "artist": "Mock Artist",
            "album": "Mock Detail",
            "duration": 182,
            "thumbnail-url": null
        }));
    }
    json!({
        "sources": [{
            "id": format!("ytm:browse:{browse_id}"),
            "kind": kind,
            "title": format!("Mock {browse_id}"),
            "url": format!("https://music.youtube.com/browse/{browse_id}"),
            "items": items,
            "continuation": null
        }]
    })
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
