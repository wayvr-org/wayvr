use libmonado::{BlockFlags, ClientLogic, ClientState, Monado, Version};
use log::trace;
use wgui::log::LogErr;

use crate::state::AppState;

pub(super) struct InputBlocker {
    use_io_blocks: bool,
    inputs_blocked_last_frame: bool,
    poses_blocked_last_frame: bool,
}

impl InputBlocker {
    pub fn new(monado: &Monado) -> Self {
        Self {
            use_io_blocks: monado.get_api_version() >= Version::new(1, 6, 0),
            inputs_blocked_last_frame: false,
            poses_blocked_last_frame: false,
        }
    }

    pub fn unblock(&self, monado: &mut Monado) {
        self.block_inputs(monado, false, false);
    }

    pub fn update(&mut self, app: &mut AppState) {
        let Some(monado) = &mut app.monado_state else {
            return; // monado not available
        };

        let should_block_inputs = app
            .input_state
            .pointers
            .iter()
            .any(|p| p.interaction.should_block_input)
            && app.session.config.block_game_input;

        let should_block_poses = app
            .input_state
            .pointers
            .iter()
            .any(|p| p.interaction.should_block_poses)
            && app.session.config.block_poses_on_kbd_interaction;

        if should_block_inputs != self.inputs_blocked_last_frame
            || should_block_poses != self.poses_blocked_last_frame
        {
            if should_block_inputs {
                trace!("Blocking input");
            } else {
                trace!("Unblocking input");
            }
            self.block_inputs(&mut monado.ipc, should_block_inputs, should_block_poses);
        }

        self.inputs_blocked_last_frame = should_block_inputs;
        self.poses_blocked_last_frame = should_block_poses;
    }

    fn block_inputs(&self, monado: &mut Monado, block_inputs: bool, block_poses: bool) {
        let Ok(clients) = monado
            .clients()
            .log_warn("Failed to get clients from Monado")
        else {
            return;
        };
        for mut client in clients {
            let Ok(name) = client.name().log_warn("Failed to get client name") else {
                continue;
            };
            if name == "wayvr" {
                continue;
            }

            let Ok(state) = client.state().log_warn("Failed to get client state") else {
                continue;
            };

            if state.contains(ClientState::ClientSessionActive | ClientState::ClientSessionVisible)
            {
                let _ = if self.use_io_blocks {
                    let flags = match (block_inputs, block_poses) {
                        (true, true) => BlockFlags::BlockPoses | BlockFlags::BlockInputs,
                        (true, false) => BlockFlags::BlockInputs.into(),
                        (false, _) => BlockFlags::None.into(),
                    };
                    client.set_io_blocks(flags)
                } else {
                    client.set_io_active(!block_inputs)
                }
                .log_warn("Failed to set IO active for client");
            }
        }
    }
}
