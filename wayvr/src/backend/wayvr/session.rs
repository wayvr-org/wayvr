use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use smithay::{
    reexports::wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
        backend::ObjectId,
    },
    wayland::shell::xdg::ToplevelSurface,
};

use super::{
    WayVRTask,
    comp::Application,
    session_protocol::server::{
        xdg_session_manager_v1::{self, XdgSessionManagerV1},
        xdg_session_v1::{self, XdgSessionV1},
        xdg_toplevel_session_v1::{self, XdgToplevelSessionV1},
    },
};

const SESSION_PREFIX: &str = "wayvr/session/";

pub(super) fn session_key(session: &str, name: &str) -> Arc<str> {
    format!("{SESSION_PREFIX}{}:{session}/{name}", session.len()).into()
}

fn session_prefix(session: &str) -> String {
    format!("{SESSION_PREFIX}{}:{session}/", session.len())
}

pub(super) fn tag_key(app_id: &str, tag: &str) -> Arc<str> {
    format!("wayvr/tag/{}:{app_id}/{}:{tag}", app_id.len(), tag.len()).into()
}

pub(super) fn app_key(app_id: &str) -> Arc<str> {
    format!("wayvr/app/{}:{app_id}", app_id.len()).into()
}

pub struct WindowSession {
    pub key: Arc<str>,
    pub restore_requested: bool,
    pub restore_resource: Option<XdgToplevelSessionV1>,
    session_id: String,
}

#[derive(Default)]
pub struct SessionRegistry {
    sessions: HashMap<String, Option<XdgSessionV1>>,
    toplevels: HashMap<ObjectId, WindowSession>,
    pub stored_keys: HashSet<Arc<str>>,
    pub committed_toplevels: HashSet<ObjectId>,
}

impl SessionRegistry {
    pub fn new(saved_keys: HashSet<Arc<str>>) -> Self {
        Self {
            stored_keys: saved_keys,
            ..Default::default()
        }
    }

    pub fn get_toplevel(&self, toplevel: &ToplevelSurface) -> Option<&WindowSession> {
        self.toplevels.get(&toplevel.xdg_toplevel().id())
    }

    pub fn forget_toplevel(&mut self, toplevel: &ToplevelSurface) {
        self.toplevels.remove(&toplevel.xdg_toplevel().id());
        self.committed_toplevels
            .remove(&toplevel.xdg_toplevel().id());
    }
}

impl Application {
    fn detach_session(&mut self, session_id: &str, forget: bool) {
        let detached: Vec<_> = self
            .sessions
            .toplevels
            .iter()
            .filter(|(_, w)| w.session_id == session_id)
            .map(|(id, w)| (id.clone(), w.key.clone()))
            .collect();

        for (id, key) in detached {
            self.sessions.toplevels.remove(&id);
            let surface = self
                .xdg_shell
                .toplevel_surfaces()
                .iter()
                .find(|surface| surface.xdg_toplevel().id() == id)
                .cloned();

            if forget {
                self.sessions.stored_keys.remove(&key);
            } else {
                self.sessions.stored_keys.insert(key.clone());
                if let Some(surface) = &surface {
                    self.wayvr_tasks
                        .send(WayVRTask::FreezeWindowState(surface.clone(), key.clone()));
                }
            }
            if let Some(surface) = surface {
                self.wayvr_tasks
                    .send(WayVRTask::WindowIdentityChanged(surface));
            }
            if forget {
                self.wayvr_tasks.send(WayVRTask::ForgetWindowState(key));
            }
        }
    }

    fn remove_session_toplevel(&mut self, session_id: &str, name: &str) {
        let key = session_key(session_id, name);
        let live = self
            .sessions
            .toplevels
            .iter()
            .find(|(_, w)| w.session_id == session_id && w.key == key)
            .map(|(id, _)| id.clone());
        if let Some(id) = live {
            self.sessions.toplevels.remove(&id);
            if let Some(surface) = self
                .xdg_shell
                .toplevel_surfaces()
                .iter()
                .find(|surface| surface.xdg_toplevel().id() == id)
                .cloned()
            {
                self.wayvr_tasks
                    .send(WayVRTask::WindowIdentityChanged(surface));
            }
        }
        self.sessions.stored_keys.remove(&key);
        self.wayvr_tasks.send(WayVRTask::ForgetWindowState(key));
    }
}

impl GlobalDispatch<XdgSessionManagerV1, ()> for Application {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<XdgSessionManagerV1>,
        _data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<XdgSessionManagerV1, ()> for Application {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &XdgSessionManagerV1,
        request: xdg_session_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_session_manager_v1::Request::GetSession {
                id,
                reason,
                session_id,
            } => {
                if let WEnum::Unknown(_) = reason {
                    resource.post_error(
                        xdg_session_manager_v1::Error::InvalidReason,
                        "unrecognized session reason",
                    );
                    return;
                }

                let known = session_id.as_ref().is_some_and(|token| {
                    state.sessions.sessions.contains_key(token)
                        || state
                            .sessions
                            .stored_keys
                            .iter()
                            .any(|key| key.starts_with(session_prefix(token).as_str()))
                });
                let token = if known {
                    session_id.unwrap()
                } else {
                    uuid::Uuid::new_v4().to_string()
                };

                if let Some(Some(old)) = state.sessions.sessions.get(&token) {
                    if old.client().is_some_and(|c| c.id() == client.id()) {
                        resource.post_error(
                            xdg_session_manager_v1::Error::InUse,
                            "session is already in use by this client",
                        );
                        return;
                    }
                    old.replaced();
                    state.detach_session(&token, false);
                }

                let session_resource = data_init.init(id, token.clone());
                if known {
                    session_resource.restored();
                } else {
                    session_resource.created(token.clone());
                }
                state
                    .sessions
                    .sessions
                    .insert(token, Some(session_resource));
            }
            xdg_session_manager_v1::Request::Destroy => {}
        }
    }
}

impl Dispatch<XdgSessionV1, String> for Application {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &XdgSessionV1,
        request: xdg_session_v1::Request,
        session_id: &String,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if state
            .sessions
            .sessions
            .get(session_id)
            .and_then(Option::as_ref)
            != Some(resource)
        {
            return;
        }

        let restore_requested = matches!(&request, xdg_session_v1::Request::RestoreToplevel { .. });
        match request {
            xdg_session_v1::Request::AddToplevel { id, toplevel, name }
            | xdg_session_v1::Request::RestoreToplevel { id, toplevel, name } => {
                let key = session_key(session_id, &name);
                if state
                    .sessions
                    .toplevels
                    .values()
                    .any(|w| w.session_id == *session_id && w.key == key)
                    || (!restore_requested && state.sessions.stored_keys.contains(&key))
                {
                    resource.post_error(
                        xdg_session_v1::Error::NameInUse,
                        "toplevel name already in use",
                    );
                    return;
                }
                let toplevel_id = toplevel.id();
                if state.sessions.toplevels.contains_key(&toplevel_id) {
                    resource.post_error(
                        xdg_session_v1::Error::AlreadyAdded,
                        "toplevel already belongs to a session",
                    );
                    return;
                }
                if restore_requested && state.sessions.committed_toplevels.contains(&toplevel_id) {
                    resource.post_error(
                        xdg_session_v1::Error::AlreadyMapped,
                        "restore_toplevel after first commit",
                    );
                    return;
                }

                let can_restore = restore_requested && state.sessions.stored_keys.contains(&key);
                let session_resource = data_init.init(id, ());
                state.sessions.toplevels.insert(
                    toplevel_id,
                    WindowSession {
                        key,
                        session_id: session_id.clone(),
                        restore_requested: can_restore,
                        restore_resource: Some(session_resource),
                    },
                );
                if let Some(surface) = state.xdg_shell.get_toplevel(&toplevel) {
                    state
                        .wayvr_tasks
                        .send(WayVRTask::WindowIdentityChanged(surface));
                }
            }
            xdg_session_v1::Request::RemoveToplevel { name } => {
                state.remove_session_toplevel(session_id, &name);
            }
            xdg_session_v1::Request::Destroy => {
                state.detach_session(session_id, false);
                state.sessions.sessions.insert(session_id.clone(), None);
            }
            xdg_session_v1::Request::Remove => {
                state.detach_session(session_id, true);
                let prefix = session_prefix(session_id);
                let keys: Vec<_> = state
                    .sessions
                    .stored_keys
                    .iter()
                    .filter(|key| key.starts_with(prefix.as_str()))
                    .cloned()
                    .collect();
                for key in keys {
                    state.sessions.stored_keys.remove(&key);
                    state.wayvr_tasks.send(WayVRTask::ForgetWindowState(key));
                }
                state.sessions.sessions.remove(session_id);
            }
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &XdgSessionV1,
        session_id: &String,
    ) {
        if state
            .sessions
            .sessions
            .get(session_id)
            .and_then(Option::as_ref)
            == Some(resource)
        {
            state.detach_session(session_id, false);
            state.sessions.sessions.insert(session_id.clone(), None);
        }
    }
}

impl Dispatch<XdgToplevelSessionV1, ()> for Application {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &XdgToplevelSessionV1,
        request: xdg_toplevel_session_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_toplevel_session_v1::Request::Destroy => {
                for w in state.sessions.toplevels.values_mut() {
                    if w.restore_resource.as_ref() == Some(resource) {
                        w.restore_resource = None;
                    }
                }
            }
            xdg_toplevel_session_v1::Request::Rename { name } => {
                let entry = state
                    .sessions
                    .toplevels
                    .iter()
                    .find(|(_, w)| w.restore_resource.as_ref() == Some(resource))
                    .map(|(id, w)| (id.clone(), w.session_id.clone(), w.key.clone()));
                let Some((id, session_id, old_key)) = entry else {
                    return;
                };
                let new_key = session_key(&session_id, &name);
                if old_key == new_key {
                    return;
                }
                if state.sessions.stored_keys.contains(&new_key)
                    || state
                        .sessions
                        .toplevels
                        .values()
                        .any(|w| w.session_id == session_id && w.key == new_key)
                {
                    if let Some(Some(owner)) = state.sessions.sessions.get(&session_id) {
                        owner.post_error(
                            xdg_session_v1::Error::NameInUse,
                            "toplevel name already in use",
                        );
                    }
                    return;
                }

                if let Some(w) = state.sessions.toplevels.get_mut(&id) {
                    w.key = new_key.clone();
                }
                if state.sessions.stored_keys.remove(&old_key) {
                    state.sessions.stored_keys.insert(new_key.clone());
                }
                state
                    .wayvr_tasks
                    .send(WayVRTask::RenameWindowState(old_key, new_key));
                if let Some(surface) = state
                    .xdg_shell
                    .toplevel_surfaces()
                    .iter()
                    .find(|s| s.xdg_toplevel().id() == id)
                    .cloned()
                {
                    state
                        .wayvr_tasks
                        .send(WayVRTask::WindowIdentityChanged(surface));
                }
            }
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &XdgToplevelSessionV1,
        _data: &(),
    ) {
        for w in state.sessions.toplevels.values_mut() {
            if w.restore_resource.as_ref() == Some(resource) {
                w.restore_resource = None;
            }
        }
    }
}
