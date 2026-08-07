use std::{
    collections::{HashMap, VecDeque},
    error::Error as StdError,
    fmt,
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex, MutexGuard},
    task::{Context, Poll},
    time::Duration,
};

use dbus::{
    arg::{self, RefArg},
    channel::{BusType, Channel},
    message::{MatchRule, Message},
    nonblock::{self, MsgMatch, Process, Proxy, SyncConnection},
};
use slotmap::SlotMap;

use dbus_screencast::OrgFreedesktopPortalScreenCast;

slotmap::new_key_type! {
    pub struct ScreenCastRequestId;
}

pub mod capture;
mod dbus_screencast;

#[derive(Debug, Clone)]
pub struct PipewireStream {
    pub node_id: u32,
    pub position: Option<(i32, i32)>,
    pub size: Option<(i32, i32)>,
}

#[derive(Debug, Clone)]
pub struct ScreenCastResponse {
    pub streams: Vec<PipewireStream>,
    pub restore_token: Option<String>,
}

#[derive(Default, Debug, Clone)]
pub struct ScreenCastParams {
    /// Optional restore token.
    pub token: Option<Rc<str>>,

    /// If true, use the first available: EMBEDDED, METADATA/FALLBACK, HIDDEN.
    pub embed_mouse: bool,

    /// false: only request MONITOR
    /// true: request MONITOR, WINDOW, VIRTUAL
    pub screens_only: bool,

    /// true: EXPLICITLY_REVOKED
    /// false: DO_NOT
    pub persist: bool,

    /// Allow the user to select multiple sources.
    pub allow_multiple: bool,
}

#[derive(Debug, Clone)]
pub enum ScreenCastResult {
    Ok(ScreenCastResponse),
    Queued,
    Pending,
    WaitingForUser,
    Failed(ScreenCastError),
}

impl PartialEq for ScreenCastResult {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Ok(_), Self::Ok(_))
                | (Self::Queued, Self::Queued)
                | (Self::Pending, Self::Pending)
                | (Self::WaitingForUser, Self::WaitingForUser)
                | (Self::Failed(_), Self::Failed(_))
        )
    }
}

#[derive(Debug, Clone)]
pub enum ScreenCastError {
    Dbus(String),
    DbusDisconnected,
    InvalidObjectPath(String),
    UnknownRequest,
    UnsupportedSourceTypes { requested: u32, available: u32 },
    UnsupportedCursorMode { available: u32 },
    PortalResponse(u32),
    MissingField(&'static str),
    InvalidResponse(&'static str),
}

impl fmt::Display for ScreenCastError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dbus(e) => write!(f, "D-Bus error: {e}"),
            Self::DbusDisconnected => write!(f, "D-Bus connection is disconnected"),
            Self::InvalidObjectPath(e) => write!(f, "invalid D-Bus object path: {e}"),
            Self::UnknownRequest => write!(f, "unknown screen select request"),
            Self::UnsupportedSourceTypes {
                requested,
                available,
            } => {
                write!(
                    f,
                    "unsupported source types: requested={requested:#x}, available={available:#x}"
                )
            }
            Self::UnsupportedCursorMode { available } => {
                write!(f, "unsupported cursor mode: available={available:#x}")
            }
            Self::PortalResponse(code) => write!(f, "portal returned response code {code}"),
            Self::MissingField(name) => write!(f, "portal response missing field {name}"),
            Self::InvalidResponse(what) => write!(f, "invalid portal response: {what}"),
        }
    }
}

impl StdError for ScreenCastError {}

impl From<dbus::Error> for ScreenCastError {
    fn from(value: dbus::Error) -> Self {
        Self::Dbus(value.to_string())
    }
}

/// manages pipewire screen requests
pub struct ScreenCastManager {
    conn: Arc<SyncConnection>,
    timeout: Duration,
    sender_component: String,

    source_types: OneShot<u32>,
    cursor_modes: OneShot<u32>,

    requests: SlotMap<ScreenCastRequestId, RequestEntry>,
    queue: VecDeque<ScreenCastRequestId>,
    active: Option<ScreenCastRequestId>,

    responses: Arc<Mutex<HashMap<String, Result<PortalResponse, ScreenCastError>>>>,
    cleanup: Vec<DbusFuture<()>>,

    next_token: u64,
}

impl ScreenCastManager {
    pub fn new() -> Result<Self, ScreenCastError> {
        let channel = Channel::get_private(BusType::Session)?;
        let conn = Arc::new(SyncConnection::from(channel));

        let timeout = Duration::from_secs(120);
        let proxy = portal_proxy(conn.clone(), timeout);

        let sender_component = sender_path_component(&format!("{}", conn.unique_name()));

        Ok(Self {
            conn,
            timeout,
            sender_component,

            source_types: OneShot::Pending(proxy.available_source_types()),
            cursor_modes: OneShot::Pending(proxy.available_cursor_modes()),

            requests: SlotMap::with_key(),
            queue: VecDeque::new(),
            active: None,

            responses: Arc::new(Mutex::new(HashMap::new())),
            cleanup: Vec::new(),

            next_token: 0,
        })
    }

    pub fn request(
        &mut self,
        params: ScreenCastParams,
    ) -> Result<ScreenCastRequestId, ScreenCastError> {
        let id = self.requests.insert(RequestEntry {
            params,
            state: RequestState::Queued,
        });

        self.queue.push_back(id);
        Ok(id)
    }

    pub fn check(&mut self, request_id: &ScreenCastRequestId) -> ScreenCastResult {
        if !self.requests.contains_key(*request_id) {
            return ScreenCastResult::Failed(ScreenCastError::UnknownRequest);
        }

        if let Err(e) = self.pump_dbus() {
            return ScreenCastResult::Failed(e);
        }

        self.poll_cleanup();
        self.poll_globals();

        self.ensure_active();

        // drive several immediate transitions, but never spin forever
        for _ in 0..12 {
            if !self.drive_active_once() {
                break;
            }

            self.ensure_active();
        }

        let result = self.result_for(*request_id);

        if is_terminal(&result) {
            let _ = self.requests.remove(*request_id);

            if self.active == Some(*request_id) {
                self.active = None;
            }

            self.queue.retain(|id| id != request_id);
        }

        result
    }

    fn pump_dbus(&self) -> Result<(), ScreenCastError> {
        let channel: &Channel = self.conn.as_ref().as_ref();
        channel
            .read_write(Some(Duration::ZERO))
            .map_err(|_| ScreenCastError::DbusDisconnected)?;

        self.conn.process_all();
        Ok(())
    }

    fn poll_cleanup(&mut self) {
        let mut i = 0;
        while i < self.cleanup.len() {
            match poll_boxed(&mut self.cleanup[i]) {
                Poll::Ready(_) => {
                    #[allow(clippy::let_underscore_future)]
                    let _ = self.cleanup.swap_remove(i);
                }
                Poll::Pending => i += 1,
            }
        }
    }

    fn poll_globals(&mut self) {
        poll_one_shot(&mut self.source_types);
        poll_one_shot(&mut self.cursor_modes);
    }

    fn globals(&self) -> Result<Option<Globals>, ScreenCastError> {
        match (&self.source_types, &self.cursor_modes) {
            (OneShot::Ready(source_types), OneShot::Ready(cursor_modes)) => Ok(Some(Globals {
                source_types: *source_types,
                cursor_modes: *cursor_modes,
            })),
            (OneShot::Failed(e), _) | (_, OneShot::Failed(e)) => Err(e.clone()),
            _ => Ok(None),
        }
    }

    fn ensure_active(&mut self) {
        if self.active.is_some() {
            return;
        }

        while let Some(id) = self.queue.pop_front() {
            let Some(entry) = self.requests.get_mut(id) else {
                continue;
            };

            if matches!(entry.state, RequestState::Queued) {
                entry.state = RequestState::Init;
                self.active = Some(id);
                return;
            }
        }
    }

    fn drive_active_once(&mut self) -> bool {
        let Some(id) = self.active else {
            return false;
        };

        let Some(params) = self.requests.get(id).map(|r| r.params.clone()) else {
            self.active = None;
            return true;
        };

        let state = match self.requests.get_mut(id) {
            Some(entry) => std::mem::replace(&mut entry.state, RequestState::Queued),
            None => {
                self.active = None;
                return true;
            }
        };

        let next_state = match self.advance_state(params, state) {
            Ok(state) => state,
            Err(error) => RequestState::Terminal(ScreenCastResult::Failed(error)),
        };

        let progressed = !matches!(next_state, RequestState::Queued);

        let terminal = matches!(next_state, RequestState::Terminal(_));

        if let Some(entry) = self.requests.get_mut(id) {
            entry.state = next_state;
        } else {
            self.active = None;
            return true;
        }

        if terminal {
            self.active = None;
        }

        progressed
    }

    fn advance_state(
        &mut self,
        params: ScreenCastParams,
        state: RequestState,
    ) -> Result<RequestState, ScreenCastError> {
        match state {
            RequestState::Queued => Ok(RequestState::Queued),

            RequestState::Init => {
                if self.globals()?.is_none() {
                    return Ok(RequestState::Init);
                }

                let session_token = self.next_token("pw_session");
                self.begin_step(PortalStep::CreateSession { session_token })
            }

            RequestState::AddMatch {
                step,
                handle_token,
                request_path,
                mut future,
            } => match poll_boxed(&mut future) {
                Poll::Pending => Ok(RequestState::AddMatch {
                    step,
                    handle_token,
                    request_path,
                    future,
                }),
                Poll::Ready(Err(e)) => Err(e.into()),
                Poll::Ready(Ok(match_handle)) => {
                    let reply = self.call_step(&params, &step, &handle_token)?;
                    Ok(RequestState::MethodCall {
                        step,
                        request_path,
                        match_handle,
                        reply,
                    })
                }
            },

            RequestState::MethodCall {
                step,
                request_path,
                match_handle,
                mut reply,
            } => match poll_unpin(&mut reply) {
                Poll::Pending => Ok(RequestState::MethodCall {
                    step,
                    request_path,
                    match_handle,
                    reply,
                }),
                Poll::Ready(Err(e)) => {
                    self.schedule_remove_match(match_handle);
                    Err(e.into())
                }
                Poll::Ready(Ok(_handle)) => Ok(RequestState::WaitResponse {
                    step,
                    request_path,
                    match_handle,
                }),
            },

            RequestState::WaitResponse {
                step,
                request_path,
                match_handle,
            } => {
                let Some(response) = self.take_response(&request_path) else {
                    return Ok(RequestState::WaitResponse {
                        step,
                        request_path,
                        match_handle,
                    });
                };

                self.schedule_remove_match(match_handle);

                let response = response?;

                if response.response != 0 {
                    return Ok(RequestState::Terminal(ScreenCastResult::Failed(
                        ScreenCastError::PortalResponse(response.response),
                    )));
                }

                match step {
                    PortalStep::CreateSession { .. } => {
                        let session_handle = required_string(&response.results, "session_handle")?;
                        let session_handle = dbus::Path::new(session_handle)
                            .map_err(ScreenCastError::InvalidObjectPath)?;

                        self.begin_step(PortalStep::SelectSources { session_handle })
                    }

                    PortalStep::SelectSources { session_handle } => {
                        self.begin_step(PortalStep::Start { session_handle })
                    }

                    PortalStep::Start { .. } => {
                        let response = parse_start_response(&response.results)?;
                        Ok(RequestState::Terminal(ScreenCastResult::Ok(response)))
                    }
                }
            }

            RequestState::Terminal(result) => Ok(RequestState::Terminal(result)),
        }
    }

    fn begin_step(&mut self, step: PortalStep) -> Result<RequestState, ScreenCastError> {
        let handle_token = self.next_token("pw_request");
        let request_path = self.request_path(&handle_token)?;

        let future = make_response_match(
            self.conn.clone(),
            self.responses.clone(),
            request_path.clone(),
        )?;

        Ok(RequestState::AddMatch {
            step,
            handle_token,
            request_path,
            future,
        })
    }

    fn call_step(
        &self,
        params: &ScreenCastParams,
        step: &PortalStep,
        handle_token: &str,
    ) -> Result<nonblock::MethodReply<dbus::Path<'static>>, ScreenCastError> {
        let proxy = portal_proxy(self.conn.clone(), self.timeout);

        match step {
            PortalStep::CreateSession { session_token } => {
                let mut options = arg::PropMap::new();
                prop_insert(&mut options, "handle_token", handle_token.to_owned());
                prop_insert(
                    &mut options,
                    "session_handle_token",
                    session_token.to_owned(),
                );

                Ok(proxy.create_session(options))
            }

            PortalStep::SelectSources { session_handle } => {
                let globals = self.globals()?.ok_or(ScreenCastError::InvalidResponse(
                    "global portal properties not ready",
                ))?;

                let requested_types = if params.screens_only {
                    SOURCE_MONITOR
                } else {
                    SOURCE_MONITOR | SOURCE_WINDOW | SOURCE_VIRTUAL
                };

                let source_types = requested_types & globals.source_types;
                if source_types == 0 {
                    return Err(ScreenCastError::UnsupportedSourceTypes {
                        requested: requested_types,
                        available: globals.source_types,
                    });
                }

                let cursor_mode = choose_cursor_mode(globals.cursor_modes, params.embed_mouse)
                    .ok_or(ScreenCastError::UnsupportedCursorMode {
                        available: globals.cursor_modes,
                    })?;

                let mut options = arg::PropMap::new();
                prop_insert(&mut options, "handle_token", handle_token.to_owned());
                prop_insert(&mut options, "types", source_types);
                prop_insert(&mut options, "multiple", params.allow_multiple);
                prop_insert(&mut options, "cursor_mode", cursor_mode);
                prop_insert(
                    &mut options,
                    "persist_mode",
                    if params.persist { 2u32 } else { 0u32 },
                );

                if let Some(token) = &params.token {
                    prop_insert(&mut options, "restore_token", token.to_string());
                }

                Ok(proxy.select_sources(session_handle.clone(), options))
            }

            PortalStep::Start { session_handle } => {
                let mut options = arg::PropMap::new();
                prop_insert(&mut options, "handle_token", handle_token.to_owned());

                Ok(proxy.start(session_handle.clone(), "", options))
            }
        }
    }

    fn request_path(&self, handle_token: &str) -> Result<String, ScreenCastError> {
        let path = format!(
            "/org/freedesktop/portal/desktop/request/{}/{}",
            self.sender_component, handle_token
        );

        let _ = dbus::Path::new(path.clone()).map_err(ScreenCastError::InvalidObjectPath)?;

        Ok(path)
    }

    fn next_token(&mut self, prefix: &str) -> String {
        self.next_token = self.next_token.saturating_add(1);
        format!("{prefix}_{}", self.next_token)
    }

    fn take_response(&self, request_path: &str) -> Option<Result<PortalResponse, ScreenCastError>> {
        lock_response_map(&self.responses).remove(request_path)
    }

    fn schedule_remove_match(&mut self, match_handle: MsgMatch) {
        let token = match_handle.token();
        let conn = self.conn.clone();

        self.cleanup
            .push(Box::pin(async move { conn.remove_match(token).await }));
    }

    fn result_for(&self, id: ScreenCastRequestId) -> ScreenCastResult {
        let Some(entry) = self.requests.get(id) else {
            return ScreenCastResult::Failed(ScreenCastError::UnknownRequest);
        };

        match &entry.state {
            RequestState::Queued => ScreenCastResult::Queued,
            RequestState::Init
            | RequestState::AddMatch { .. }
            | RequestState::MethodCall { .. } => ScreenCastResult::Pending,
            RequestState::WaitResponse { step, .. } => match step {
                PortalStep::Start { .. } => ScreenCastResult::WaitingForUser,
                _ => ScreenCastResult::Pending,
            },
            RequestState::Terminal(result) => result.clone(),
        }
    }
}

struct RequestEntry {
    params: ScreenCastParams,
    state: RequestState,
}

enum RequestState {
    Queued,
    Init,
    AddMatch {
        step: PortalStep,
        handle_token: String,
        request_path: String,
        future: DbusFuture<MsgMatch>,
    },
    MethodCall {
        step: PortalStep,
        request_path: String,
        match_handle: MsgMatch,
        reply: nonblock::MethodReply<dbus::Path<'static>>,
    },
    WaitResponse {
        step: PortalStep,
        request_path: String,
        match_handle: MsgMatch,
    },
    Terminal(ScreenCastResult),
}

#[derive(Clone)]
enum PortalStep {
    CreateSession { session_token: String },
    SelectSources { session_handle: dbus::Path<'static> },
    Start { session_handle: dbus::Path<'static> },
}

#[derive(Debug, Clone, Copy)]
struct Globals {
    source_types: u32,
    cursor_modes: u32,
}

enum OneShot<T> {
    Pending(nonblock::MethodReply<T>),
    Ready(T),
    Failed(ScreenCastError),
}

#[derive(Debug)]
struct PortalResponse {
    response: u32,
    results: arg::PropMap,
}

type DbusFuture<T> = Pin<Box<dyn Future<Output = Result<T, dbus::Error>>>>;

const PORTAL_DEST: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";

const SOURCE_MONITOR: u32 = 1;
const SOURCE_WINDOW: u32 = 2;
const SOURCE_VIRTUAL: u32 = 4;

const CURSOR_HIDDEN: u32 = 1;
const CURSOR_EMBEDDED: u32 = 2;
const CURSOR_METADATA: u32 = 4;

fn portal_proxy(
    conn: Arc<SyncConnection>,
    timeout: Duration,
) -> Proxy<'static, Arc<SyncConnection>> {
    Proxy::new(PORTAL_DEST, PORTAL_PATH, timeout, conn)
}

fn make_response_match(
    conn: Arc<SyncConnection>,
    responses: Arc<Mutex<HashMap<String, Result<PortalResponse, ScreenCastError>>>>,
    request_path: String,
) -> Result<DbusFuture<MsgMatch>, ScreenCastError> {
    let path = dbus::Path::new(request_path.clone()).map_err(ScreenCastError::InvalidObjectPath)?;

    let rule = MatchRule::new_signal(REQUEST_IFACE, "Response")
        .with_sender(PORTAL_DEST)
        .with_path(path);

    Ok(Box::pin(async move {
        let match_handle = conn.add_match(rule).await?;

        let key = request_path.clone();
        let responses = responses.clone();

        Ok(match_handle.msg_cb(move |msg: Message| {
            let parsed = parse_portal_response_message(&msg);
            lock_response_map(&responses).insert(key.clone(), parsed);
            false
        }))
    }))
}

fn parse_portal_response_message(msg: &Message) -> Result<PortalResponse, ScreenCastError> {
    let (response, results): (u32, arg::PropMap) = msg
        .read2()
        .map_err(|_| ScreenCastError::InvalidResponse("Request::Response signal"))?;

    Ok(PortalResponse { response, results })
}

fn parse_start_response(results: &arg::PropMap) -> Result<ScreenCastResponse, ScreenCastError> {
    Ok(ScreenCastResponse {
        streams: parse_streams(results)?,
        restore_token: optional_string(results, "restore_token"),
    })
}

fn parse_streams(results: &arg::PropMap) -> Result<Vec<PipewireStream>, ScreenCastError> {
    let streams = required_arg(results, "streams")?;
    let streams = unvariant(streams);

    let iter = streams
        .as_iter()
        .ok_or(ScreenCastError::InvalidResponse("streams array"))?;

    let mut out = Vec::new();

    for stream in iter {
        let mut fields = stream
            .as_iter()
            .ok_or(ScreenCastError::InvalidResponse("stream tuple"))?;

        let node_id = fields
            .next()
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())
            .ok_or(ScreenCastError::InvalidResponse("stream node id"))?;

        let props = fields
            .next()
            .ok_or(ScreenCastError::InvalidResponse("stream properties"))?;

        let position = dict_get(props, "position").and_then(tuple_i32);
        let size = dict_get(props, "size").and_then(tuple_i32);

        out.push(PipewireStream {
            node_id,
            position,
            size,
        });
    }

    if out.is_empty() {
        return Err(ScreenCastError::InvalidResponse("empty streams array"));
    }

    Ok(out)
}

fn required_string(map: &arg::PropMap, key: &'static str) -> Result<String, ScreenCastError> {
    optional_string(map, key).ok_or(ScreenCastError::MissingField(key))
}

fn optional_string(map: &arg::PropMap, key: &str) -> Option<String> {
    map.get(key)
        .and_then(|v| unvariant(&*v.0).as_str())
        .map(str::to_owned)
}

fn required_arg<'a>(
    map: &'a arg::PropMap,
    key: &'static str,
) -> Result<&'a dyn RefArg, ScreenCastError> {
    map.get(key)
        .map(|v| &*v.0 as &dyn RefArg)
        .ok_or(ScreenCastError::MissingField(key))
}

fn dict_get<'a>(dict: &'a dyn RefArg, key: &str) -> Option<&'a dyn RefArg> {
    let dict = unvariant(dict);
    let mut iter = dict.as_iter()?;

    loop {
        let k = iter.next()?;
        let v = iter.next()?;

        if k.as_str() == Some(key) {
            return Some(unvariant(v));
        }
    }
}

fn tuple_i32(value: &dyn RefArg) -> Option<(i32, i32)> {
    let value = unvariant(value);
    let mut iter = value.as_iter()?;

    let x = i32::try_from(iter.next()?.as_i64()?).ok()?;
    let y = i32::try_from(iter.next()?.as_i64()?).ok()?;

    Some((x, y))
}

fn unvariant(value: &dyn RefArg) -> &dyn RefArg {
    if value.arg_type() != arg::ArgType::Variant {
        return value;
    }

    value
        .as_iter()
        .and_then(|mut iter| iter.next())
        .unwrap_or(value)
}

fn prop_insert<T>(map: &mut arg::PropMap, key: &str, value: T)
where
    T: RefArg + 'static,
{
    map.insert(key.to_owned(), arg::Variant(Box::new(value)));
}

fn choose_cursor_mode(available: u32, embed_mouse: bool) -> Option<u32> {
    let choices: &[u32] = if embed_mouse {
        // The portal names this METADATA; this is the closest "fallback" mode.
        &[CURSOR_EMBEDDED, CURSOR_METADATA, CURSOR_HIDDEN]
    } else {
        &[CURSOR_HIDDEN]
    };

    choices.iter().copied().find(|mode| available & *mode != 0)
}

fn poll_one_shot<T>(value: &mut OneShot<T>) {
    let old = std::mem::replace(
        value,
        OneShot::Failed(ScreenCastError::InvalidResponse(
            "internal one-shot placeholder",
        )),
    );

    *value = match old {
        OneShot::Pending(mut future) => match poll_unpin(&mut future) {
            Poll::Ready(Ok(value)) => OneShot::Ready(value),
            Poll::Ready(Err(e)) => OneShot::Failed(e.into()),
            Poll::Pending => OneShot::Pending(future),
        },
        other => other,
    };
}

fn poll_unpin<F>(future: &mut F) -> Poll<F::Output>
where
    F: Future + Unpin,
{
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    Pin::new(future).poll(&mut cx)
}

fn poll_boxed<T>(future: &mut DbusFuture<T>) -> Poll<Result<T, dbus::Error>> {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    future.as_mut().poll(&mut cx)
}

fn sender_path_component(unique_name: &str) -> String {
    let name = unique_name.strip_prefix(':').unwrap_or(unique_name);

    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }

    if out.is_empty() {
        "unknown".to_owned()
    } else {
        out
    }
}

fn lock_response_map(
    responses: &Arc<Mutex<HashMap<String, Result<PortalResponse, ScreenCastError>>>>,
) -> MutexGuard<'_, HashMap<String, Result<PortalResponse, ScreenCastError>>> {
    match responses.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn is_terminal(result: &ScreenCastResult) -> bool {
    matches!(
        result,
        ScreenCastResult::Ok(_) | ScreenCastResult::Failed(_)
    )
}

/// Helper function to make a single screen cast selection
pub fn screen_cast_select_blocking(
    params: ScreenCastParams,
) -> Result<ScreenCastResponse, ScreenCastError> {
    let mut manager = ScreenCastManager::new()?;
    let request_id = manager.request(params)?;

    loop {
        match manager.check(&request_id) {
            ScreenCastResult::Ok(response) => return Ok(response),
            ScreenCastResult::Failed(error) => return Err(error),

            ScreenCastResult::Queued
            | ScreenCastResult::Pending
            | ScreenCastResult::WaitingForUser => {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
