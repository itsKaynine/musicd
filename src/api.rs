use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::extract::FromRequest;
use axum::{
    Json, Router,
    extract::connect_info::ConnectInfo,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Path as AxPath, State as AxState},
    http::{StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
    routing::{any, get, post},
};
use axum_extra::TypedHeader;
use futures_util::{sink::SinkExt, stream::StreamExt};
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};

use crate::downloader::DownloaderKind;
use crate::job::{Job, JobManager};
use crate::notifier::Notifier;
use crate::player::{PlayerHandle, SetPlaylistMode};
use crate::playlist::{PlaylistMeta, get_playlists};
use crate::publisher::Publisher;
use crate::settings::Paths;
use crate::state::State as Kv;
use crate::utils::hhmmss::Hhmmss;

static INDEX_HTML: &str = "index.html";

#[derive(Embed)]
#[folder = "static"]
struct StaticAssets;

#[derive(Clone)]
pub struct AppCtx {
    pub paths: Paths,
    pub kv: Arc<Kv>,
    pub notifier: Notifier,
    pub publisher: Publisher,
    pub player: PlayerHandle,
    pub job_manager: JobManager,
}

enum AppError {
    AnyhowError(anyhow::Error),
}

#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(AppError))]
struct AppJson<T>(T);

impl<T> IntoResponse for AppJson<T>
where
    axum::Json<T>: IntoResponse,
{
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct ErrorResponse {
            success: bool,
            message: String,
        }

        let (status, message) = match self {
            AppError::AnyhowError(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        };

        (
            status,
            AppJson(ErrorResponse {
                success: false,
                message,
            }),
        )
            .into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(error: anyhow::Error) -> Self {
        Self::AnyhowError(error)
    }
}

#[derive(Serialize)]
struct ListPlaylistItem {
    folder: String,
    meta: PlaylistMeta,
}

#[derive(Deserialize)]
pub struct PublishParams {
    name: String,
    source_urls: Vec<String>,
    #[serde(default)]
    downloader: Option<DownloaderKind>,
}

#[derive(Deserialize)]
pub struct SeekParams {
    secs: u64,
}

#[derive(Deserialize)]
pub struct SetVolumeParams {
    value: f32,
}

#[derive(Deserialize)]
pub struct SetPlaylistParams {
    mode: SetPlaylistMode,
}

#[derive(Serialize)]
pub struct StatusResp {
    playlist_id: Option<String>,
    playlist_name: Option<String>,
    current_index: usize,
    current_track: Option<String>,
    current_pos: Option<Duration>,
    total_duration: Option<Duration>,
    is_paused: Option<bool>,
    volume: Option<f32>,
    position: Option<String>,
}

pub fn router(ctx: AppCtx) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/playlists", get(list_playlists))
        .route("/jobs", get(list_jobs))
        .route("/publish", post(publish))
        .route("/clean", post(clean))
        .route("/control/play", post(play))
        .route("/control/pause", post(pause))
        .route("/control/prev", post(prev))
        .route("/control/next", post(next))
        .route("/control/seek", post(seek))
        .route("/control/volume", post(set_volume))
        .route("/control/playlist/{id}", post(set_playlist))
        .route("/control/track/{idx}", post(set_track))
        .route("/ws", any(ws_handler))
        .fallback(static_handler)
        .with_state(ctx)
        .layer(TraceLayer::new_for_http().make_span_with(DefaultMakeSpan::default().include_headers(true)))
}

async fn ws_handler(
    AxState(ctx): AxState<AppCtx>,
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        String::from("Unknown browser")
    };
    tracing::info!("`{user_agent}` at {addr} connected to websocket");

    ws.on_upgrade(move |socket| handle_socket(socket, addr, ctx.notifier.clone()))
}

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    if path.is_empty() || path == INDEX_HTML {
        return index_html().await;
    }

    match StaticAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();

            ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response()
        }
        None => {
            if path.contains('.') {
                return not_found().await;
            }

            index_html().await
        }
    }
}

async fn index_html() -> Response {
    match StaticAssets::get(INDEX_HTML) {
        Some(content) => Html(content.data).into_response(),
        None => not_found().await,
    }
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "404").into_response()
}

async fn status(AxState(ctx): AxState<AppCtx>) -> Result<Json<StatusResp>, AppError> {
    let s = ctx.player.status()?;

    let current_pos_display = s.current_pos.map(|x| x.hhmmss()).unwrap_or("-".to_string());
    let total_duration_display = s.total_duration.map(|x| x.hhmmss()).unwrap_or("-".to_string());

    Ok(Json(StatusResp {
        playlist_id: s.playlist_id,
        playlist_name: s.playlist_name,
        current_index: s.current_index,
        current_track: s.current_track,
        current_pos: s.current_pos,
        total_duration: s.total_duration,
        is_paused: s.is_paused,
        volume: s.volume,
        position: format!("{current_pos_display} / {total_duration_display}").into(),
    }))
}

async fn list_playlists(AxState(ctx): AxState<AppCtx>) -> Json<Vec<ListPlaylistItem>> {
    let items = get_playlists(&ctx.paths.playlists).unwrap_or_default();
    Json(
        items
            .into_iter()
            .map(|(f, m)| ListPlaylistItem { folder: f, meta: m })
            .collect(),
    )
}

async fn list_jobs(AxState(ctx): AxState<AppCtx>) -> Json<Vec<Job>> {
    let jobs = ctx.job_manager.current_jobs.lock().unwrap().clone();
    Json(jobs)
}

async fn publish(AxState(ctx): AxState<AppCtx>, Json(params): Json<PublishParams>) -> impl IntoResponse {
    ctx.publisher
        .publish_in_background(&params.name, &params.source_urls, params.downloader);

    Json(json!({"success": true}))
}

async fn clean(AxState(ctx): AxState<AppCtx>) -> Result<impl IntoResponse, AppError> {
    let dir = ctx.paths.tmp;

    let mut entries = tokio::fs::read_dir(&dir)
        .await
        .with_context(|| format!("Failed to read directory: {}", dir.display()))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .with_context(|| format!("Failed to read entry in directory: {}", dir.display()))?
    {
        let path = entry.path();

        if path.is_dir() {
            tokio::fs::remove_dir_all(&path)
                .await
                .with_context(|| format!("Failed to remove directory: {}", path.display()))?;
        } else {
            tokio::fs::remove_file(&path)
                .await
                .with_context(|| format!("Failed to remove file: {}", path.display()))?;
        }
    }

    Ok(Json(json!({"success": true})))
}

async fn play(AxState(ctx): AxState<AppCtx>) -> impl IntoResponse {
    ctx.player.play();
    Json(json!({"success": true}))
}

async fn pause(AxState(ctx): AxState<AppCtx>) -> impl IntoResponse {
    ctx.player.pause();
    Json(json!({"success": true}))
}

async fn prev(AxState(ctx): AxState<AppCtx>) -> impl IntoResponse {
    ctx.player.prev();
    Json(json!({"success": true}))
}

async fn next(AxState(ctx): AxState<AppCtx>) -> impl IntoResponse {
    ctx.player.next();
    Json(json!({"success": true}))
}

async fn seek(AxState(ctx): AxState<AppCtx>, Json(params): Json<SeekParams>) -> impl IntoResponse {
    ctx.player.seek(params.secs);
    Json(json!({"success": true}))
}

async fn set_volume(AxState(ctx): AxState<AppCtx>, Json(params): Json<SetVolumeParams>) -> impl IntoResponse {
    ctx.player.set_volume(params.value);
    Json(json!({"success": true}))
}

async fn set_playlist(
    AxState(ctx): AxState<AppCtx>,
    AxPath(id): AxPath<String>,
    Json(params): Json<SetPlaylistParams>,
) -> impl IntoResponse {
    // Find playlist by id
    let items = get_playlists(&ctx.paths.playlists).unwrap_or_default();
    if let Some((folder, meta)) = items.into_iter().find(|(_, m)| m.id == id) {
        let dir = ctx.paths.playlists.join(folder);
        if let Err(error) = ctx.kv.set_current_playlist_id(&meta.id) {
            tracing::warn!("kv set failed: {error:#}");
        }
        ctx.player.set_playlist_dir(dir, params.mode);
        return Json(json!({"success": true}));
    }
    Json(json!({"success": false, "message": "Not found"}))
}

async fn set_track(AxState(ctx): AxState<AppCtx>, AxPath(idx): AxPath<usize>) -> impl IntoResponse {
    ctx.player.set_index(idx);
    Json(json!({"success": true}))
}

async fn handle_socket(socket: WebSocket, who: SocketAddr, notifier: Notifier) {
    let (mut sender, mut receiver) = socket.split();

    let mut rx = notifier.subscribe();

    let mut send_task = tokio::spawn(async move {
        while let Ok(notification) = rx.recv().await
            && let Ok(text) = serde_json::to_string(&notification)
        {
            if let Err(error) = sender.send(Message::Text(text.into())).await {
                tracing::warn!("[ws] Failed to send message to WebSocket client: {error}");
                break;
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if process_ws_message(msg, who).is_break() {
                break;
            }
        }
    });

    // If any one of the tasks exit, abort the other.
    tokio::select! {
        rv_a = (&mut send_task) => {
            match rv_a {
                Ok(_) => {},
                Err(a) => tracing::warn!("[ws] Error sending messages {a:?}")
            }
            recv_task.abort();
        },
        rv_b = (&mut recv_task) => {
            match rv_b {
                Ok(_) => {},
                Err(b) => tracing::warn!("[ws] Error receiving messages {b:?}")
            }
            send_task.abort();
        }
    }

    // returning from the handler closes the websocket connection
    tracing::info!("[ws] Context {who} destroyed");
}

fn process_ws_message(msg: Message, who: SocketAddr) -> ControlFlow<(), ()> {
    match msg {
        Message::Text(t) => {
            tracing::trace!("[ws] {who} sent str: {t:?}");
        }
        Message::Binary(d) => {
            tracing::trace!("[ws] {who} sent {} bytes: {d:?}", d.len());
        }
        Message::Close(c) => {
            if let Some(cf) = c {
                tracing::info!("[ws] {who} sent close with code {} and reason `{}`", cf.code, cf.reason);
            } else {
                tracing::warn!("[ws] {who} somehow sent close message without CloseFrame");
            }
            return ControlFlow::Break(());
        }

        Message::Pong(v) => {
            tracing::trace!("[ws] {who} sent pong with {v:?}");
        }
        // You should never need to manually handle Message::Ping, as axum's websocket library
        // will do so for you automagically by replying with Pong and copying the v according to
        // spec. But if you need the contents of the pings you can see them here.
        Message::Ping(v) => {
            tracing::trace!("[ws] {who} sent ping with {v:?}");
        }
    }
    ControlFlow::Continue(())
}
