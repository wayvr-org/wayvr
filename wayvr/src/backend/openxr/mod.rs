use std::{
    collections::VecDeque,
    ops::Add,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use glam::{Affine3A, Vec3};
use input::OpenXrInputSource;
use openxr as xr;
use skybox::{Skybox, create_skybox};
use vulkano::{Handle, VulkanObject};
use wlx_common::overlays::{StereoMode, ToastTopic};

use crate::{
    FRAME_COUNTER, RUNNING,
    backend::{
        BackendError, XrBackend,
        input::interact,
        openxr::{lines::LinePool, overlay::OpenXrOverlayData},
        task::{OpenXrTask, OverlayTask, TaskType},
    },
    config::{save_settings, save_state},
    graphics::{GpuFutures, init_openxr_graphics},
    overlays::{toast::Toast, watch::WATCH_NAME},
    state::AppState,
    subsystem::notifications::NotificationManager,
    windowing::{
        backend::{RenderResources, RenderTarget, ShouldRender},
        manager::OverlayWindowManager,
    },
};

mod blocker;
mod helpers;
mod input;
mod lines;
mod overlay;
mod playspace;
mod skybox;
mod swapchain;

const VIEW_TYPE: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;

struct XrState {
    instance: xr::Instance,
    session: xr::Session<xr::Vulkan>,
    predicted_display_time: xr::Time,
    fps: f32,
    stage: Arc<xr::Space>,
    view: Arc<xr::Space>,
}

#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub fn openxr_run(show_by_default: bool, headless: bool) -> Result<(), BackendError> {
    let (xr_instance, system) = match helpers::init_xr() {
        Ok((xr_instance, system)) => (xr_instance, system),
        Err(e) => {
            log::warn!("Will not use OpenXR: {e}");
            return Err(BackendError::NotSupported);
        }
    };

    let mut app = {
        let (gfx, gfx_extras) = init_openxr_graphics(xr_instance.clone(), system)?;
        AppState::from_graphics(gfx, gfx_extras, XrBackend::OpenXR)?
    };

    let modes = xr_instance.enumerate_environment_blend_modes(system, VIEW_TYPE)?;

    if show_by_default {
        app.tasks.enqueue_at(
            TaskType::Overlay(OverlayTask::ShowHide),
            Instant::now().add(Duration::from_secs(1)),
        );
    }

    let mut overlays = OverlayWindowManager::<OpenXrOverlayData>::new(&mut app, headless)?;
    let mut lines = LinePool::new(&app)?;
    let mut current_lines = Vec::with_capacity(2);

    let mut notifications = NotificationManager::new();
    notifications.run_dbus(&mut app.dbus);
    notifications.run_udp();

    let mut delete_queue = vec![];

    app.monado_init();

    let mut playspace = app.monado.as_mut().and_then(|m| {
        playspace::PlayspaceMover::new(m)
            .map_err(|e| log::warn!("Will not use Monado playspace mover: {e}"))
            .ok()
    });

    let mut blocker = app.monado.as_ref().map(blocker::InputBlocker::new);

    let (session, mut frame_wait, mut frame_stream) = unsafe {
        let raw_session = helpers::create_overlay_session(
            &xr_instance,
            system,
            &xr::vulkan::SessionCreateInfo {
                instance: app.gfx.instance.handle().as_raw() as _,
                physical_device: app.gfx.device.physical_device().handle().as_raw() as _,
                device: app.gfx.device.handle().as_raw() as _,
                queue_family_index: app.gfx.queue_gfx.queue_family_index(),
                queue_index: 0,
            },
        )?;
        xr::Session::from_raw(xr_instance.clone(), raw_session, Box::new(()))
    };

    let stage =
        session.create_reference_space(xr::ReferenceSpaceType::STAGE, xr::Posef::IDENTITY)?;

    let view = session.create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)?;

    let mut xr_state = XrState {
        instance: xr_instance,
        session,
        predicted_display_time: xr::Time::from_nanos(0),
        fps: 30.0,
        stage: Arc::new(stage),
        view: Arc::new(view),
    };

    let mut skybox: Option<Skybox> = None;

    let pointer_lines = [
        lines.allocate(&xr_state, &app)?,
        lines.allocate(&xr_state, &app)?,
    ];

    let watch_id = overlays.lookup(WATCH_NAME).unwrap(); // want panic

    let mut input_source = input::OpenXrInputSource::new(&xr_state)?;

    let mut session_running = false;
    let mut event_storage = xr::EventDataBuffer::new();

    let mut next_device_update = Instant::now();
    let mut due_tasks = VecDeque::with_capacity(4);

    let mut fps_counter: VecDeque<Instant> = VecDeque::new();

    let mut main_session_visible = false;
    let mut environment_blend_mode = modes[0];

    'main_loop: loop {
        let cur_frame = FRAME_COUNTER.fetch_add(1, Ordering::Relaxed);

        if !RUNNING.load(Ordering::Relaxed) {
            log::warn!("Received shutdown signal.");
            match xr_state.session.request_exit() {
                Ok(()) => log::info!("OpenXR session exit requested."),
                Err(xr::sys::Result::ERROR_SESSION_NOT_RUNNING) => break 'main_loop,
                Err(e) => {
                    log::error!("Failed to request OpenXR session exit: {e}");
                    break 'main_loop;
                }
            }
        }

        while let Some(event) = xr_state.instance.poll_event(&mut event_storage)? {
            match event {
                xr::Event::SessionStateChanged(e) => {
                    // Session state change is where we can begin and end sessions, as well as
                    // find quit messages!
                    log::info!("entered state {:?}", e.state());
                    match e.state() {
                        xr::SessionState::READY => {
                            xr_state.session.begin(VIEW_TYPE)?;
                            session_running = true;
                            reconfigure_environment_blend(
                                &app,
                                &xr_state,
                                &modes,
                                &mut skybox,
                                &mut environment_blend_mode,
                                main_session_visible,
                            );
                        }
                        xr::SessionState::STOPPING => {
                            xr_state.session.end()?;
                            session_running = false;
                        }
                        xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => {
                            break 'main_loop;
                        }
                        _ => {}
                    }
                }
                xr::Event::InstanceLossPending(_) => {
                    break 'main_loop;
                }
                xr::Event::EventsLost(e) => {
                    log::warn!("lost {} events", e.lost_event_count());
                }
                xr::Event::MainSessionVisibilityChangedEXTX(e) => {
                    main_session_visible = e.visible();
                    reconfigure_environment_blend(
                        &app,
                        &xr_state,
                        &modes,
                        &mut skybox,
                        &mut environment_blend_mode,
                        main_session_visible,
                    );
                }
                _ => {}
            }
        }

        if app.monado.is_some() && next_device_update <= Instant::now() {
            let changed = OpenXrInputSource::update_devices(&mut app);
            if changed {
                overlays.devices_changed(&mut app)?;
            }
            next_device_update = Instant::now() + Duration::from_secs(30);
        }

        if !session_running {
            std::thread::sleep(Duration::from_millis(100));
            log::trace!("session not running: continue");
            continue 'main_loop;
        }

        log::trace!("xrWaitFrame");
        let xr_frame_state = frame_wait.wait()?;
        log::trace!("xrBeginFrame");
        frame_stream.begin()?;

        xr_state.predicted_display_time = xr_frame_state.predicted_display_time;
        xr_state.fps = {
            fps_counter.push_back(Instant::now());

            while let Some(time) = fps_counter.front() {
                if time.elapsed().as_secs_f32() > 1. {
                    fps_counter.pop_front();
                } else {
                    break;
                }
            }

            let total_elapsed = fps_counter
                .front()
                .map_or(0f32, |time| time.elapsed().as_secs_f32());

            fps_counter.len() as f32 / total_elapsed
        };

        if !xr_frame_state.should_render {
            log::trace!("xrEndFrame");
            frame_stream.end(
                xr_frame_state.predicted_display_time,
                environment_blend_mode,
                &[],
            )?;
            continue 'main_loop;
        }

        app.input_state.pre_update();
        input_source.update(&xr_state, &mut app)?;
        app.input_state.post_update(&app.session);

        if let Some(ref mut blocker) = blocker {
            blocker.update(&mut app);
        }

        if app
            .input_state
            .pointers
            .iter()
            .any(|p| p.now.show_hide && !p.before.show_hide)
        {
            overlays.show_hide(&mut app);
        }

        if app
            .input_state
            .pointers
            .iter()
            .any(|p| p.now.toggle_dashboard && !p.before.toggle_dashboard)
        {
            app.tasks
                .enqueue(TaskType::Overlay(OverlayTask::ToggleDashboard));
        }

        if let Some(ref mut space_mover) = playspace {
            space_mover.update(&mut overlays, &mut app);
        }

        for o in overlays.values_mut() {
            o.after_input(&mut app)?;
        }

        #[cfg(feature = "osc")]
        if let Some(ref mut sender) = app.osc_sender {
            let _ = sender.send_params(&overlays, &app.input_state.devices);
        }

        let (_, views) = xr_state.session.locate_views(
            VIEW_TYPE,
            xr_frame_state.predicted_display_time,
            &xr_state.stage,
        )?;

        let ipd = helpers::ipd_from_views(&views);
        if (app.input_state.ipd - ipd).abs() > 0.05 {
            log::info!("IPD changed: {} -> {}", app.input_state.ipd, ipd);
            app.input_state.ipd = ipd;
            Toast::new(ToastTopic::IpdChange, "IPD".into(), format!("{ipd:.1} mm"))
                .submit(&mut app);
        }

        overlays.values_mut().for_each(|o| o.config.tick(&mut app));

        current_lines.clear();

        let haptics = interact(&mut overlays, &mut app, &mut current_lines);
        for (idx, haptics) in haptics.iter().enumerate() {
            if let Some(haptics) = haptics {
                input_source.haptics(&xr_state, idx, haptics);
            }
        }
        for (idx, line) in current_lines.iter().enumerate() {
            lines.draw_between(
                pointer_lines[idx],
                line.a,
                line.b,
                line.mode as usize + 1,
                app.input_state.hmd,
            );
        }

        app.hid_provider.inner.commit();

        let watch = overlays.mut_by_id(watch_id).unwrap(); // want panic
        let watch_state = watch.config.active_state.as_mut().unwrap();
        let watch_transform = watch_state.transform;
        if watch_state.alpha < 0.05 {
            //FIXME: Temporary workaround for Monado bug
            watch_state.transform = Affine3A::from_scale(Vec3 {
                x: 0.001,
                y: 0.001,
                z: 0.001,
            });
            watch_state.alpha = 0.02; // visible but not really. Monado freaks out if no layers are submitted.
        }

        if let Err(e) =
            crate::ipc::events::tick_events::<OpenXrOverlayData>(&mut app, &mut overlays)
        {
            log::error!("WayVR IPC tick_events failed: {e:?}");
        }

        // Begin rendering
        let mut futures = GpuFutures::default();

        if !main_session_visible && let Some(skybox) = skybox.as_mut() {
            skybox.render(&xr_state, &app, &mut futures)?;
        }

        for o in overlays.values_mut() {
            o.data.cur_visible = false;
            if o.config
                .active_state
                .as_ref()
                .is_none_or(|s| s.alpha < 0.01)
            {
                log::trace!("{}: hidden, skip render", o.config.name);
                continue;
            }

            if !o.data.init {
                log::trace!("{}: init", o.config.name);
                o.init(&mut app)?;
                o.data.init = true;
            }

            let should_render = matches!(o.should_render(&mut app)?, ShouldRender::Should);
            log::trace!("{}: should_render returned: {should_render}", o.config.name);

            if should_render {
                log::trace!("{}: render new frame", o.config.name);
                let meta = o.config.backend.frame_meta().unwrap(); // want panic
                let stereo = !matches!(meta.stereo, StereoMode::None);
                let wsi = o.ensure_swapchain_acquire(&app, &xr_state, meta.extent, stereo)?;
                let tgt = RenderTarget { views: wsi.views };
                let mut rdr = RenderResources::new(app.gfx.clone(), tgt, &meta)?;
                o.render(&mut app, &mut rdr)?;
                futures.execute_results(rdr.end()?)?;
            } else if o.data.swapchain.is_none() {
                log::trace!("{}: not showing due to missing swapchain", o.config.name);
                continue;
            } else {
                log::trace!("{}: showing stale frame", o.config.name);
            }
            o.data.cur_visible = true;
        }

        lines.render(&app, &mut futures)?;
        futures.wait()?;
        // End rendering

        // Layer composition
        let mut layers = vec![];
        if !main_session_visible && let Some(skybox) = skybox.as_mut() {
            for (idx, layer) in skybox.present(&xr_state, &app)?.into_iter().enumerate() {
                layers.push(((idx as f32).mul_add(-50.0, 200.0), layer));
            }
        }

        for o in overlays.values_mut() {
            if !o.data.cur_visible {
                continue;
            }
            // unwrap: above if only passes if active_state is some
            let active_state = o.config.active_state.as_ref().unwrap();
            let dist_sq = (app.input_state.hmd.translation - active_state.transform.translation)
                .length_squared()
                + (100f32 - o.config.z_order as f32);
            if !dist_sq.is_normal() {
                o.data.swapchain.as_mut().unwrap().ensure_image_released()?;
                continue;
            }
            for layer in o.present(&xr_state)? {
                layers.push((dist_sq, layer));
            }
        }

        for layer in lines.present(&xr_state)? {
            layers.push((0.0, layer));
        }
        // End layer composition

        // Begin layer submit
        layers.sort_by(|a, b| b.0.total_cmp(&a.0));

        let frame_ref = layers
            .iter()
            .map(|f| match f.1 {
                CompositionLayer::Quad(ref l) => l as &xr::CompositionLayerBase<xr::Vulkan>,
                CompositionLayer::Cylinder(ref l) => l as &xr::CompositionLayerBase<xr::Vulkan>,
                CompositionLayer::Equirect2(ref l) => l as &xr::CompositionLayerBase<xr::Vulkan>,
            })
            .collect::<Vec<_>>();

        log::trace!("xrEndFrame");
        frame_stream.end(
            xr_state.predicted_display_time,
            environment_blend_mode,
            &frame_ref,
        )?;
        // End layer submit

        app.dbus.tick();
        notifications.submit_pending(&mut app);

        app.tasks.retrieve_due(&mut due_tasks);
        while let Some(task) = due_tasks.pop_front() {
            match task {
                TaskType::Input(task) => {
                    app.input_state.handle_task(task);
                }
                TaskType::Overlay(task) => {
                    overlays.handle_task(&mut app, task)?;
                }
                TaskType::Playspace(task) => {
                    if let Some(playspace) = playspace.as_mut() {
                        playspace.handle_task(&mut app, task);
                    }
                }
                TaskType::OpenXR(task) => {
                    if matches!(task, OpenXrTask::SettingsChanged) {
                        reconfigure_environment_blend(
                            &app,
                            &xr_state,
                            &modes,
                            &mut skybox,
                            &mut environment_blend_mode,
                            main_session_visible,
                        );
                    }
                }
                #[cfg(feature = "openvr")]
                TaskType::OpenVR(_) => {}
            }
        }

        while let Some(o) = overlays.pop_dropped() {
            delete_queue.push((o, cur_frame + 5));
        }

        delete_queue.retain(|(_, frame)| *frame > cur_frame);

        //FIXME: Temporary workaround for Monado bug
        let watch = overlays.mut_by_id(watch_id).unwrap(); // want panic
        watch.config.active_state.as_mut().unwrap().transform = watch_transform;
    } // main_loop

    if let (Some(blocker), Some(monado)) = (blocker, app.monado.as_mut()) {
        blocker.unblock(monado);
    }

    overlays.persist_layout(&mut app);
    if let Err(e) = save_state(&app.session.config) {
        log::error!("Could not save state: {e:?}");
    }
    if app.session.config_dirty {
        save_settings(&app.session.config)?;
        app.session.config_dirty = false;
    }

    Ok(())
}

pub(super) enum CompositionLayer<'a> {
    Quad(xr::CompositionLayerQuad<'a, xr::Vulkan>),
    Cylinder(xr::CompositionLayerCylinderKHR<'a, xr::Vulkan>),
    Equirect2(xr::CompositionLayerEquirect2KHR<'a, xr::Vulkan>),
}

fn reconfigure_environment_blend(
    app: &AppState,
    xr_state: &XrState,
    modes: &[xr::EnvironmentBlendMode],
    skybox: &mut Option<Skybox>,
    environment_blend_mode: &mut xr::EnvironmentBlendMode,
    main_session_visible: bool,
) {
    *environment_blend_mode = {
        if modes.contains(&xr::EnvironmentBlendMode::ALPHA_BLEND)
            && app.session.config.use_passthrough
        {
            xr::EnvironmentBlendMode::ALPHA_BLEND
        } else {
            modes[0]
        }
    };

    let want_skybox = *environment_blend_mode == xr::EnvironmentBlendMode::OPAQUE
        && app.session.config.use_skybox
        && !main_session_visible;

    if want_skybox == skybox.is_some() {
        return;
    }

    if want_skybox {
        log::debug!("Allocating skybox.");
        *skybox = create_skybox(xr_state, app);
    } else {
        log::debug!("Destroying skybox.");
        *skybox = None;
    }
}
