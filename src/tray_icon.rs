#![cfg(feature = "tray-icon")]

use std::{path::PathBuf, sync::mpsc, time::Duration};

use anyhow::anyhow;
use ksni::{menu::StandardItem, MenuItem, Tray, TrayService};

use crate::{
    controller::{ServiceCommand, ServiceController},
    model::ConnectionStatus,
    prompt::SecurePrompt,
    util,
};

const TITLE: &str = "SNX-RS VPN client";
const PING_DURATION: Duration = Duration::from_secs(1);

const ICON_ACQUIRING: &str = "network-vpn-acquiring-symbolic";
const ICON_DISABLED: &str = "network-vpn-disabled-symbolic";
const ICON_DISCONNECTED: &str = "network-vpn-disconnected-symbolic";
const ICON_CONNECTED: &str = "network-vpn-symbolic";

struct MyTray {
    command_sender: mpsc::SyncSender<Option<ServiceCommand>>,
    status: anyhow::Result<ConnectionStatus>,
    connecting: bool,
}

impl MyTray {
    fn connect(&mut self) {
        let _ = self.command_sender.send(Some(ServiceCommand::Connect));
    }

    fn disconnect(&mut self) {
        let _ = self.command_sender.send(Some(ServiceCommand::Disconnect));
    }

    fn quit(&mut self) {
        let _ = self.command_sender.send(None);
    }

    fn status_label(&self) -> String {
        if self.connecting {
            "...".to_owned()
        } else {
            match self.status {
                Ok(ref status) => {
                    if let Some(since) = status.connected_since {
                        if status.mfa_pending {
                            "Pending MFA prompt".to_owned()
                        } else {
                            format!("Connected since: {}", since.to_rfc2822())
                        }
                    } else {
                        "Tunnel disconnected".to_owned()
                    }
                }
                Err(ref e) => e.to_string(),
            }
        }
    }
    fn edit_config(&mut self) {
        if let Ok(home) = std::env::var("HOME") {
            let _ = opener::open(PathBuf::from(home).join(".config").join("snx-rs").join("snx-rs.conf"));
        }
    }
}

impl Tray for MyTray {
    fn title(&self) -> String {
        TITLE.to_owned()
    }

    fn icon_name(&self) -> String {
        if self.connecting {
            ICON_ACQUIRING.to_owned()
        } else {
            match self.status {
                Ok(ref status) => {
                    if status.connected_since.is_some() {
                        ICON_CONNECTED.to_owned()
                    } else {
                        ICON_DISCONNECTED.to_owned()
                    }
                }
                Err(_) => ICON_DISABLED.to_owned(),
            }
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            MenuItem::Standard(StandardItem {
                label: self.status_label(),
                enabled: false,
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Connect".to_string(),
                icon_name: ICON_CONNECTED.to_owned(),
                enabled: self
                    .status
                    .as_ref()
                    .is_ok_and(|status| status.connected_since.is_none() && !status.mfa_pending)
                    && !self.connecting,
                activate: Box::new(|this: &mut Self| this.connect()),
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: "Disconnect".to_string(),
                icon_name: ICON_DISCONNECTED.to_owned(),
                enabled: self
                    .status
                    .as_ref()
                    .is_ok_and(|status| status.connected_since.is_some()),
                activate: Box::new(|this: &mut Self| this.disconnect()),
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: "Edit configuration".to_string(),
                icon_name: "edit-text".to_owned(),
                activate: Box::new(|this: &mut Self| this.edit_config()),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Exit".to_string(),
                icon_name: "application-exit".to_owned(),
                activate: Box::new(|this: &mut Self| this.quit()),
                ..Default::default()
            }),
        ]
    }
}

pub fn show_tray_icon() -> anyhow::Result<()> {
    let (tx, rx) = mpsc::sync_channel(1);
    let service = TrayService::new(MyTray {
        command_sender: tx.clone(),
        status: Err(anyhow!("No service connection")),
        connecting: false,
    });
    let handle = service.handle();
    service.spawn();

    let tx_copy = tx.clone();
    std::thread::spawn(move || loop {
        let _ = tx_copy.send(Some(ServiceCommand::Status));
        std::thread::sleep(PING_DURATION);
    });

    let mut prev_command = ServiceCommand::Info;
    let mut prev_status = String::new();

    while let Ok(Some(command)) = rx.recv() {
        if let Ok(controller) = ServiceController::new(SecurePrompt::gui()) {
            if command == ServiceCommand::Connect {
                handle.update(|tray: &mut MyTray| tray.connecting = true);
            }

            let result = std::thread::scope(|s| s.spawn(|| util::block_on(controller.command(command))).join());
            let status = match result {
                Ok(result) => result,
                Err(_) => Err(anyhow!("Internal error")),
            };

            let status_str = format!("{:?}", status);
            if command != prev_command || status_str != prev_status {
                handle.update(|tray: &mut MyTray| {
                    tray.connecting = false;
                    tray.status = status;
                });
            }
            prev_command = command;
            prev_status = status_str;
        }
    }

    Ok(())
}
