use std::{
    collections::VecDeque,
    ops::Add,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use ovr_overlay::{
    TrackedDeviceIndex,
    sys::{ETrackedDeviceProperty, EVRApplicationType, EVREventType},
};
use smallvec::smallvec;
use vulkano::{Handle, VulkanObject, device::physical::PhysicalDevice};
use wlx_common::overlays::ToastTopic;

use crate::{
    FRAME_COUNTER, RUNNING,
    backend::{
        BackendError, XrBackend,
        input::interact,
        openvr::{
            helpers::adjust_gain,
            input::{OpenVrInputSource, set_action_manifest},
            lines::LinePool,
            manifest::{install_manifest, uninstall_manifest},
            overlay::OpenVrOverlayData,
        },
        task::{OpenVrTask, OverlayTask, TaskType},
    },
    config::{save_settings, save_state},
    graphics::{GpuFutures, init_openvr_graphics},
    overlays::toast::Toast,
    state::AppState,
    subsystem::notifications::NotificationManager,
    windowing::{
        backend::{RenderResources, RenderTarget, ShouldRender},
        manager::OverlayWindowManager,
    },
};

pub mod helpers;
pub mod input;
pub mod lines;
pub mod manifest;
pub mod overlay;
pub mod playspace;

pub fn openvr_uninstall() {
    let app_type = EVRApplicationType::VRApplication_Overlay;
    let Ok(context) = ovr_overlay::Context::init(app_type) else {
        log::error!("Uninstall failed: could not reach OpenVR");
        return;
    };

    let mut app_mgr = context.applications_mngr();
    let _ = uninstall_manifest(&mut app_mgr);
}

#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub fn openvr_run(
    show_by_default: bool,
    headless: bool,
    no_autostart: bool,
) -> Result<(), BackendError> {
    let app_type = EVRApplicationType::VRApplication_Overlay;
    let Ok(context) = ovr_overlay::Context::init(app_type) else {
        log::warn!("Will not use OpenVR: Context init failed");
        return Err(BackendError::NotSupported);
    };

    log::info!("Using OpenVR runtime");

    let mut app_mgr = context.applications_mngr();
    let mut input_mgr = context.input_mngr();
    let mut system_mgr = context.system_mngr();
    let mut overlay_mgr = context.overlay_mngr();
    let mut settings_mgr = context.settings_mngr();
    let mut chaperone_mgr = context.chaperone_setup_mngr();
    let mut compositor_mgr = context.compositor_mngr();

    let device_extensions_fn = |device: &PhysicalDevice| {
        let names = compositor_mgr.get_vulkan_device_extensions_required(device.handle().as_raw());
        names.iter().map(std::string::String::as_str).collect()
    };

    let mut compositor_mgr = context.compositor_mngr();
    let instance_extensions = {
        let names = compositor_mgr.get_vulkan_instance_extensions_required();
        names.iter().map(std::string::String::as_str).collect()
    };

    let mut app = {
        let (gfx, gfx_extras) = init_openvr_graphics(instance_extensions, device_extensions_fn)?;
        AppState::from_graphics(gfx, gfx_extras, XrBackend::OpenVR)?
    };

    app.session.no_autostart = no_autostart;

    if show_by_default {
        app.tasks.enqueue_at(
            TaskType::Overlay(OverlayTask::ShowHide),
            Instant::now().add(Duration::from_secs(1)),
        );
    }

    if let Ok(ipd) = system_mgr.get_tracked_device_property::<f32>(
        TrackedDeviceIndex::HMD,
        ETrackedDeviceProperty::Prop_UserIpdMeters_Float,
    ) {
        app.input_state.ipd = (ipd * 1000.0).round();
        log::info!("IPD: {:.0} mm", app.input_state.ipd);
    }

    let _ = install_manifest(&mut app_mgr);

    let mut overlays = OverlayWindowManager::<OpenVrOverlayData>::new(&mut app, headless)?;
    let mut notifications = NotificationManager::new();
    notifications.run_dbus(&mut app.dbus);
    notifications.run_udp();

    let mut playspace = playspace::PlayspaceMover::new();
    playspace.playspace_changed(&mut compositor_mgr, &mut chaperone_mgr);

    set_action_manifest(&mut input_mgr)?;

    let mut input_source = OpenVrInputSource::new(&mut input_mgr)?;

    let Ok(refresh_rate) = system_mgr.get_tracked_device_property::<f32>(
        TrackedDeviceIndex::HMD,
        ETrackedDeviceProperty::Prop_DisplayFrequency_Float,
    ) else {
        return Err(BackendError::Fatal(anyhow!(
            "Failed to get HMD refresh rate"
        )));
    };

    log::info!("HMD running @ {refresh_rate} Hz");

    // want at least half refresh rate
    let frame_timeout = 2 * (1000.0 / refresh_rate).floor() as u32;

    let mut next_device_update = Instant::now();
    let mut due_tasks = VecDeque::with_capacity(4);

    let mut lines = LinePool::new(app.gfx.clone())?;
    let pointer_lines = [lines.allocate(), lines.allocate()];
    let mut current_lines = Vec::with_capacity(2);

    'main_loop: loop {
        let _ = overlay_mgr.wait_frame_sync(frame_timeout);

        if !RUNNING.load(Ordering::Relaxed) {
            log::warn!("Received shutdown signal.");
            break 'main_loop;
        }
        FRAME_COUNTER.fetch_add(1, Ordering::Relaxed);

        {
            // extremely cursed
            const EV_QUIT: u32 = EVREventType::VREvent_Quit as u32;
            const EV_DEV_ACTIVATED: u32 = EVREventType::VREvent_TrackedDeviceActivated as u32;
            const EV_DEV_DEACTIVATED: u32 = EVREventType::VREvent_TrackedDeviceDeactivated as u32;
            const EV_DEV_UPDATED: u32 = EVREventType::VREvent_TrackedDeviceUpdated as u32;
            const EV_SEAT_ZERO: u32 = EVREventType::VREvent_SeatedZeroPoseReset as u32;
            const EV_STAND_ZERO: u32 = EVREventType::VREvent_StandingZeroPoseReset as u32;
            const EV_CHAP_CHANGED: u32 = EVREventType::VREvent_ChaperoneUniverseHasChanged as u32;
            const EV_SCENE_CHANGED: u32 = EVREventType::VREvent_SceneApplicationChanged as u32;
            const EV_IPD_CHANGED: u32 = EVREventType::VREvent_IpdChanged as u32;

            while let Some(event) = system_mgr.poll_next_event() {
                match event.event_type {
                    EV_QUIT => {
                        log::warn!("Received quit event, shutting down.");
                        break 'main_loop;
                    }
                    EV_DEV_ACTIVATED | EV_DEV_DEACTIVATED | EV_DEV_UPDATED => {
                        next_device_update = Instant::now();
                    }
                    EV_SEAT_ZERO | EV_STAND_ZERO | EV_CHAP_CHANGED | EV_SCENE_CHANGED => {
                        playspace.playspace_changed(&mut compositor_mgr, &mut chaperone_mgr);
                    }
                    EV_IPD_CHANGED => {
                        if let Ok(ipd) = system_mgr.get_tracked_device_property::<f32>(
                            TrackedDeviceIndex::HMD,
                            ETrackedDeviceProperty::Prop_UserIpdMeters_Float,
                        ) {
                            let ipd = (ipd * 1000.0).round();
                            if (ipd - app.input_state.ipd).abs() > 0.05 {
                                log::info!("IPD: {:.1} mm -> {:.1} mm", app.input_state.ipd, ipd);
                                Toast::new(
                                    ToastTopic::IpdChange,
                                    "IPD".into(),
                                    format!("{ipd:.1} mm"),
                                )
                                .submit(&mut app);
                            }
                            app.input_state.ipd = ipd;
                        }
                    }
                    _ => {}
                }
            }
        }

        if next_device_update <= Instant::now() {
            let changed = input_source.update_devices(&mut system_mgr, &mut app);
            if changed {
                overlays.devices_changed(&mut app)?;
            }
            next_device_update = Instant::now() + Duration::from_secs(30);
        }

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
                    playspace.handle_task(&app, &mut chaperone_mgr, task);
                }
                TaskType::OpenVR(task) => match task {
                    OpenVrTask::ColorGain(channel, value) => {
                        let _ = adjust_gain(&mut settings_mgr, channel, value);
                    }
                },
                #[cfg(feature = "openxr")]
                TaskType::OpenXR(_) => {}
            }
        }

        while let Some(mut o) = overlays.pop_dropped() {
            o.destroy(&mut overlay_mgr);
        }

        let universe = playspace.get_universe();

        app.input_state.pre_update();
        input_source.update(universe.clone(), &mut input_mgr, &mut system_mgr, &mut app);
        app.input_state.post_update(&app.session);

        if app
            .input_state
            .pointers
            .iter()
            .any(|p| p.now.show_hide && !p.before.show_hide)
        {
            lines.mark_dirty(); // workaround to prevent lines from not showing
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

        overlays.values_mut().for_each(|o| o.config.tick(&mut app));

        playspace.update(&mut chaperone_mgr, &mut overlays, &app);

        current_lines.clear();

        let haptics = interact(&mut overlays, &mut app, &mut current_lines);
        for (idx, haptics) in haptics.iter().enumerate() {
            if let Some(haptics) = haptics {
                input_source.haptics(&mut input_mgr, idx, haptics);
            }
        }
        for (idx, line) in current_lines.iter().enumerate() {
            lines.draw_between(
                pointer_lines[idx],
                line.a,
                line.b,
                line.mode as usize + 1,
                &app.input_state.hmd,
            );
        }

        app.hid_provider.inner.commit();
        let mut futures = GpuFutures::default();

        lines.update(universe.clone(), &mut overlay_mgr, &mut app)?;

        for o in overlays.values_mut() {
            o.after_input(&mut overlay_mgr, &mut app)?;
        }

        #[cfg(feature = "osc")]
        if let Some(ref mut sender) = app.osc_sender {
            let _ = sender.send_params(&overlays, &app.input_state.devices);
        }

        if let Err(e) =
            crate::ipc::events::tick_events::<OpenVrOverlayData>(&mut app, &mut overlays)
        {
            log::error!("WayVR IPC tick_events failed: {e:?}");
        }

        log::trace!("Rendering frame");

        for o in overlays.values_mut() {
            if o.config.active_state.is_some() {
                let ShouldRender::Should = o.should_render(&mut app)? else {
                    continue;
                };
                let meta = o.config.backend.frame_meta().unwrap();
                let tgt = RenderTarget {
                    views: smallvec![o.ensure_staging_image(&mut app, meta.extent)?],
                };
                let mut rdr = RenderResources::new(app.gfx.clone(), tgt, &meta)?;
                o.render(&mut app, &mut rdr)?;
                o.data.image_dirty = true;
                futures.execute_results(rdr.end()?)?;
            }
        }

        log::trace!("Rendering overlays");
        futures.wait()?;

        overlays
            .values_mut()
            .for_each(|o| o.after_render(universe.clone(), &mut overlay_mgr, &app.gfx));

        lines.reset();

        // chaperone
    } // main_loop

    overlays.persist_layout(&mut app);
    if let Err(e) = save_state(&app.session.config) {
        log::error!("Could not save state: {e:?}");
    }
    if app.session.config_dirty {
        save_settings(&app.session.config)?;
        app.session.config_dirty = false;
    }

    log::warn!("OpenVR shutdown");
    unsafe { context.shutdown() };

    Ok(())
}
