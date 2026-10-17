use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use eframe::egui;
use egui::{Color32, RichText, Stroke, Vec2};

use crate::adb::{
    connect_wireless, disconnect_wireless, enable_tcpip, list_devices, send_key_event,
    set_screen_timeout_ms, set_stay_on, sleep_screen, toggle_power, wake_screen,
};
use crate::file_manager::{
    create_remote_folder, delete_remote, format_bytes, install_apk, list_directory,
    pull_item, push_item, SHORTCUTS,
};
use crate::scrcpy::launch_mirroring;
use crate::types::{AppConfig, DeviceInfo, RemoteFile, StayOnMode, TransferItem, TransferStatus};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ActiveTab {
    DevicesAndMirror,
    FileTransfer,
    Settings,
    About,
}

enum AppEvent {
    DevicesRefreshed(Result<Vec<DeviceInfo>, String>),
    FilesRefreshed(Result<Vec<RemoteFile>, String>),
    TransferUpdated {
        id: u64,
        status: TransferStatus,
        message: String,
    },
    ActionFinished(Result<String, String>),
}

pub struct ScreenCopyApp {
    config: AppConfig,
    active_tab: ActiveTab,

    // Devices state
    devices: Vec<DeviceInfo>,
    selected_device_idx: usize,
    is_refreshing_devices: bool,
    last_device_refresh: std::time::Instant,

    // Mirroring state
    active_child: Option<Child>,
    mirroring_status_msg: Option<String>,

    // File manager state
    current_remote_path: String,
    remote_files: Vec<RemoteFile>,
    is_loading_files: bool,
    file_search: String,
    new_folder_name: String,
    show_new_folder_dialog: bool,

    // Transfers
    transfers: Vec<TransferItem>,
    next_transfer_id: u64,

    // Wireless connect dialog
    wireless_address_input: String,
    tcpip_port_input: String,

    // Notifications
    status_notification: Option<(String, bool, std::time::Instant)>,

    // Background channels
    event_tx: Sender<AppEvent>,
    event_rx: Receiver<AppEvent>,
}

impl ScreenCopyApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (event_tx, event_rx) = channel();

        let mut app = Self {
            config: AppConfig::default(),
            active_tab: ActiveTab::DevicesAndMirror,
            devices: Vec::new(),
            selected_device_idx: 0,
            is_refreshing_devices: false,
            last_device_refresh: std::time::Instant::now() - std::time::Duration::from_secs(10),
            active_child: None,
            mirroring_status_msg: None,
            current_remote_path: "/sdcard/Download".to_string(),
            remote_files: Vec::new(),
            is_loading_files: false,
            file_search: String::new(),
            new_folder_name: String::new(),
            show_new_folder_dialog: false,
            transfers: Vec::new(),
            next_transfer_id: 1,
            wireless_address_input: "192.168.1.".to_string(),
            tcpip_port_input: "5555".to_string(),
            status_notification: None,
            event_tx,
            event_rx,
        };

        app.refresh_devices();
        app
    }

    fn notify(&mut self, text: impl Into<String>, is_success: bool) {
        self.status_notification = Some((text.into(), is_success, std::time::Instant::now()));
    }

    pub fn selected_device(&self) -> Option<&DeviceInfo> {
        self.devices.get(self.selected_device_idx)
    }

    pub fn selected_serial(&self) -> Option<String> {
        self.selected_device().map(|d| d.serial.clone())
    }

    fn refresh_devices(&mut self) {
        if self.is_refreshing_devices {
            return;
        }
        self.is_refreshing_devices = true;
        let tx = self.event_tx.clone();

        thread::spawn(move || {
            let res = list_devices();
            let _ = tx.send(AppEvent::DevicesRefreshed(res));
        });
    }

    fn refresh_files(&mut self) {
        let serial = match self.selected_serial() {
            Some(s) => s,
            None => {
                self.remote_files.clear();
                return;
            }
        };

        self.is_loading_files = true;
        let path = self.current_remote_path.clone();
        let tx = self.event_tx.clone();

        thread::spawn(move || {
            let res = list_directory(&serial, &path);
            let _ = tx.send(AppEvent::FilesRefreshed(res));
        });
    }

    fn queue_upload(&mut self, local_path: PathBuf, is_apk: bool) {
        let serial = match self.selected_serial() {
            Some(s) => s,
            None => {
                self.notify("Aucun appareil sélectionné pour le transfert !", false);
                return;
            }
        };

        let file_name = local_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Fichier".to_string());

        let file_size = local_path.metadata().map(|m| m.len()).unwrap_or(0);
        let remote_dest = self.current_remote_path.clone();
        let id = self.next_transfer_id;
        self.next_transfer_id += 1;

        let item = TransferItem {
            id,
            is_upload: true,
            is_apk_install: is_apk,
            local_path: local_path.to_string_lossy().to_string(),
            remote_path: format!("{}/{}", remote_dest, file_name),
            file_name: file_name.clone(),
            file_size,
            status: TransferStatus::Transferring,
            message: if is_apk {
                "Installation en cours...".to_string()
            } else {
                "Envoi en cours...".to_string()
            },
        };

        self.transfers.insert(0, item);
        let tx = self.event_tx.clone();

        thread::spawn(move || {
            if is_apk {
                match install_apk(&serial, &local_path) {
                    Ok(msg) => {
                        let _ = tx.send(AppEvent::TransferUpdated {
                            id,
                            status: TransferStatus::Completed,
                            message: msg,
                        });
                    }
                    Err(err) => {
                        let _ = tx.send(AppEvent::TransferUpdated {
                            id,
                            status: TransferStatus::Failed(err.clone()),
                            message: err,
                        });
                    }
                }
            } else {
                match push_item(&serial, &local_path, &remote_dest) {
                    Ok(msg) => {
                        let _ = tx.send(AppEvent::TransferUpdated {
                            id,
                            status: TransferStatus::Completed,
                            message: if msg.is_empty() {
                                "Transféré avec succès !".to_string()
                            } else {
                                msg
                            },
                        });
                    }
                    Err(err) => {
                        let _ = tx.send(AppEvent::TransferUpdated {
                            id,
                            status: TransferStatus::Failed(err.clone()),
                            message: err,
                        });
                    }
                }
            }
        });
    }

    fn queue_download(&mut self, remote_file: RemoteFile) {
        let serial = match self.selected_serial() {
            Some(s) => s,
            None => {
                self.notify("Aucun appareil connecté !", false);
                return;
            }
        };

        let dest_dir = PathBuf::from(&self.config.default_download_dir);
        let id = self.next_transfer_id;
        self.next_transfer_id += 1;

        let item = TransferItem {
            id,
            is_upload: false,
            is_apk_install: false,
            local_path: dest_dir.join(&remote_file.name).to_string_lossy().to_string(),
            remote_path: remote_file.path.clone(),
            file_name: remote_file.name.clone(),
            file_size: remote_file.size,
            status: TransferStatus::Transferring,
            message: "Téléchargement vers le PC...".to_string(),
        };

        self.transfers.insert(0, item);
        let tx = self.event_tx.clone();

        thread::spawn(move || {
            match pull_item(&serial, &remote_file.path, &dest_dir) {
                Ok(msg) => {
                    let _ = tx.send(AppEvent::TransferUpdated {
                        id,
                        status: TransferStatus::Completed,
                        message: if msg.is_empty() {
                            "Téléchargé sur votre PC !".to_string()
                        } else {
                            msg
                        },
                    });
                }
                Err(err) => {
                    let _ = tx.send(AppEvent::TransferUpdated {
                        id,
                        status: TransferStatus::Failed(err.clone()),
                        message: err,
                    });
                }
            }
        });
    }

    fn toggle_stay_awake_setting(&mut self) {
        self.config.stay_awake = !self.config.stay_awake;
        let stay_awake = self.config.stay_awake;
        let mode = if stay_awake {
            self.config.stay_on_mode.clone()
        } else {
            StayOnMode::SystemDefault
        };

        if let Some(serial) = self.selected_serial() {
            let tx = self.event_tx.clone();
            thread::spawn(move || {
                let res = set_stay_on(&serial, &mode);
                let _ = tx.send(AppEvent::ActionFinished(
                    res.map(|_| {
                        if stay_awake {
                            "Écran configuré en maintien allumé (Stay Awake Actif)".to_string()
                        } else {
                            "Mode veille par défaut rétabli".to_string()
                        }
                    }),
                ));
            });
        }
    }

    fn start_mirroring(&mut self) {
        let serial = match self.selected_serial() {
            Some(s) => s,
            None => {
                self.notify("Veuillez sélectionner un appareil connecté !", false);
                return;
            }
        };

        // If configured, activate stay on screen automatically
        if self.config.stay_awake {
            let _ = set_stay_on(&serial, &self.config.stay_on_mode);
        }

        match launch_mirroring(&serial, &self.config) {
            Ok(child) => {
                self.active_child = Some(child);
                self.mirroring_status_msg = Some("Session ScreenCopy en cours".to_string());
                self.notify("ScreenCopyPLUS lancé sans fenêtre de console !", true);
            }
            Err(e) => {
                self.mirroring_status_msg = Some(format!("Erreur: {}", e));
                self.notify(format!("Échec du lancement: {}", e), false);
            }
        }
    }

    fn stop_mirroring(&mut self) {
        if let Some(mut child) = self.active_child.take() {
            let _ = child.kill();
            self.mirroring_status_msg = Some("Session terminée".to_string());
            self.notify("Session ScreenCopy arrêtée.", true);
        }
    }

    fn process_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AppEvent::DevicesRefreshed(res) => {
                    self.is_refreshing_devices = false;
                    match res {
                        Ok(devs) => {
                            self.devices = devs;
                            if self.selected_device_idx >= self.devices.len() && !self.devices.is_empty() {
                                self.selected_device_idx = 0;
                            }
                            // Auto trigger stay awake on connect if configured
                            if self.config.auto_stay_awake_on_connect && self.config.stay_awake {
                                if let Some(serial) = self.selected_serial() {
                                    let mode = self.config.stay_on_mode.clone();
                                    thread::spawn(move || {
                                        let _ = set_stay_on(&serial, &mode);
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            self.notify(format!("Erreur actualisation appareils: {}", e), false);
                        }
                    }
                }
                AppEvent::FilesRefreshed(res) => {
                    self.is_loading_files = false;
                    match res {
                        Ok(files) => {
                            self.remote_files = files;
                        }
                        Err(e) => {
                            self.notify(format!("Erreur lecture dossier: {}", e), false);
                        }
                    }
                }
                AppEvent::TransferUpdated { id, status, message } => {
                    if let Some(item) = self.transfers.iter_mut().find(|t| t.id == id) {
                        item.status = status.clone();
                        item.message = message.clone();
                    }
                    if status == TransferStatus::Completed {
                        self.notify(format!("Transfert terminé: {}", message), true);
                        self.refresh_files();
                    } else if let TransferStatus::Failed(err) = &status {
                        self.notify(format!("Échec du transfert: {}", err), false);
                    }
                }
                AppEvent::ActionFinished(res) => match res {
                    Ok(msg) => self.notify(msg, true),
                    Err(err) => self.notify(err, false),
                },
            }
        }
    }

    fn check_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() {
            for file in dropped {
                if let Some(path) = file.path {
                    let is_apk = path
                        .extension()
                        .map(|e| e.to_string_lossy().to_lowercase() == "apk")
                        .unwrap_or(false);
                    self.queue_upload(path, is_apk);
                }
            }
        }
    }
}

impl eframe::App for ScreenCopyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_events();
        self.check_dropped_files(ctx);

        // Auto-refresh devices every 4 seconds
        if self.last_device_refresh.elapsed() > std::time::Duration::from_secs(4) {
            self.last_device_refresh = std::time::Instant::now();
            self.refresh_devices();
        }

        // Check if active child process is still running
        if let Some(child) = &mut self.active_child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.active_child = None;
                    self.mirroring_status_msg = Some(format!("Fermé ({})", status));
                }
                Ok(None) => {}
                Err(_) => {
                    self.active_child = None;
                }
            }
        }

        // Apply modern dark theme
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(18, 20, 24);
        visuals.window_fill = Color32::from_rgb(24, 27, 33);
        visuals.hyperlink_color = Color32::from_rgb(0, 180, 255);
        ctx.set_visuals(visuals);

        // Top Navigation Bar
        egui::TopBottomPanel::top("top_panel")
            .frame(egui::Frame::none().fill(Color32::from_rgb(24, 28, 36)).inner_margin(12.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Logo & App Name
                    ui.label(
                        RichText::new("📱 ScreenCopy")
                            .size(20.0)
                            .strong()
                            .color(Color32::from_rgb(255, 255, 255)),
                    );
                    ui.label(
                        RichText::new("PLUS")
                            .size(20.0)
                            .strong()
                            .color(Color32::from_rgb(0, 195, 255)),
                    );

                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("v5.0.1 • Rust Edition")
                            .size(11.0)
                            .color(Color32::from_rgb(120, 140, 160)),
                    );

                    ui.add_space(24.0);

                    // Navigation Tabs
                    let tabs = [
                        (ActiveTab::DevicesAndMirror, "🚀 Appareils & Mirroring"),
                        (ActiveTab::FileTransfer, "📂 Transfert de Fichiers"),
                        (ActiveTab::Settings, "⚙️ Réglages"),
                        (ActiveTab::About, "ℹ️ À Propos"),
                    ];

                    for (tab, label) in tabs {
                        let is_active = self.active_tab == tab;
                        let text = RichText::new(label).size(14.0).strong().color(if is_active {
                            Color32::from_rgb(0, 195, 255)
                        } else {
                            Color32::from_rgb(180, 190, 205)
                        });

                        if ui.selectable_label(is_active, text).clicked() {
                            self.active_tab = tab;
                            if tab == ActiveTab::FileTransfer && self.remote_files.is_empty() {
                                self.refresh_files();
                            }
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Quick Stay-Awake Status Pill
                        let stay_awake_active = self.config.stay_awake;
                        let stay_pill_color = if stay_awake_active {
                            Color32::from_rgb(34, 197, 94)
                        } else {
                            Color32::from_rgb(100, 116, 139)
                        };

                        let stay_text = if stay_awake_active {
                            "🔆 Écran Toujours Allumé: ACTIF"
                        } else {
                            "🌙 Écran Veille Normale"
                        };

                        if ui
                            .button(
                                RichText::new(stay_text)
                                    .size(12.0)
                                    .strong()
                                    .color(stay_pill_color),
                            )
                            .on_hover_text("Cliquez pour activer/désactiver le maintien de l'écran allumé sur le téléphone")
                            .clicked()
                        {
                            self.toggle_stay_awake_setting();
                        }
                    });
                });
            });

        // Bottom Status Bar
        egui::TopBottomPanel::bottom("bottom_bar")
            .frame(egui::Frame::none().fill(Color32::from_rgb(14, 16, 20)).inner_margin(8.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some((msg, success, _)) = &self.status_notification {
                        let color = if *success {
                            Color32::from_rgb(74, 222, 128)
                        } else {
                            Color32::from_rgb(248, 113, 113)
                        };
                        ui.label(RichText::new(msg).size(12.0).color(color));
                    } else if let Some(msg) = &self.mirroring_status_msg {
                        ui.label(
                            RichText::new(format!("● {}", msg))
                                .size(12.0)
                                .color(Color32::from_rgb(0, 195, 255)),
                        );
                    } else {
                        ui.label(
                            RichText::new("Prêt • Glissez-déposez des fichiers dans l'application pour les transférer")
                                .size(12.0)
                                .color(Color32::from_rgb(140, 150, 165)),
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let count = self.devices.len();
                        let dev_text = format!("{} appareil(s) détecté(s)", count);
                        ui.label(RichText::new(dev_text).size(12.0).color(Color32::from_rgb(150, 160, 175)));
                    });
                });
            });

        // Main Content Area
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(18, 20, 24)).inner_margin(16.0))
            .show(ctx, |ui| {
                // Drag & Drop Hover Overlay
                let is_hovered = ctx.input(|i| !i.raw.hovered_files.is_empty());
                if is_hovered {
                    ui.painter().rect_filled(
                        ui.clip_rect(),
                        8.0,
                        Color32::from_rgba_premultiplied(0, 180, 255, 30),
                    );
                    ui.painter().rect_stroke(
                        ui.clip_rect(),
                        8.0,
                        Stroke::new(2.0_f32, Color32::from_rgb(0, 195, 255)),
                    );
                }

                match self.active_tab {
                    ActiveTab::DevicesAndMirror => self.render_devices_tab(ui),
                    ActiveTab::FileTransfer => self.render_file_transfer_tab(ui),
                    ActiveTab::Settings => self.render_settings_tab(ui),
                    ActiveTab::About => self.render_about_tab(ui),
                }
            });

        // Request repaint if mirroring or loading
        if self.active_child.is_some() || self.is_loading_files || self.is_refreshing_devices {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

impl ScreenCopyApp {
    fn render_devices_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Contrôle de l'appareil & Mirroring").color(Color32::WHITE));
            ui.add_space(16.0);
            if ui.button("🔄 Actualiser les appareils").clicked() {
                self.refresh_devices();
            }
        });

        ui.add_space(12.0);

        if self.devices.is_empty() {
            egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 32, 40))
                .inner_margin(24.0)
                .rounding(8.0)
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("📱 Aucun appareil Android détecté").size(18.0).strong().color(Color32::from_rgb(220, 230, 240)));
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(
                                "1. Connectez votre smartphone avec un câble USB.\n2. Activez le 'Débogage USB' dans les Options de développement.\n3. Acceptez la demande d'autorisation sur l'écran du téléphone.",
                            )
                            .size(13.0)
                            .color(Color32::from_rgb(160, 175, 195)),
                        );
                        ui.add_space(12.0);
                        if ui.button(RichText::new("🔄 Re-détecter maintenant").size(14.0)).clicked() {
                            self.refresh_devices();
                        }
                    });
                });
            return;
        }

        // Device Selection Cards
        ui.label(RichText::new("Sélectionnez l'appareil :").size(13.0).color(Color32::from_rgb(160, 175, 195)));
        ui.add_space(6.0);

        ui.horizontal_wrapped(|ui| {
            for (idx, dev) in self.devices.iter().enumerate() {
                let is_selected = idx == self.selected_device_idx;
                let bg_color = if is_selected {
                    Color32::from_rgb(32, 45, 65)
                } else {
                    Color32::from_rgb(26, 30, 38)
                };
                let border_color = if is_selected {
                    Color32::from_rgb(0, 195, 255)
                } else {
                    Color32::from_rgb(50, 60, 75)
                };

                let card = egui::Frame::none()
                    .fill(bg_color)
                    .stroke(Stroke::new(if is_selected { 2.0_f32 } else { 1.0_f32 }, border_color))
                    .rounding(8.0)
                    .inner_margin(12.0);

                card.show(ui, |ui| {
                    let btn_response = ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("📱").size(18.0));
                            ui.label(RichText::new(&dev.model).size(15.0).strong().color(Color32::WHITE));
                        });
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("Série: {}", dev.serial)).size(11.0).color(Color32::from_rgb(140, 155, 175)));

                        ui.horizontal(|ui| {
                            if let Some(bat) = dev.battery_level {
                                let bat_icon = if dev.battery_charging { "⚡" } else { "🔋" };
                                ui.label(RichText::new(format!("{} {}%", bat_icon, bat)).size(11.0).color(Color32::from_rgb(74, 222, 128)));
                            }
                            if let Some(ver) = &dev.android_version {
                                ui.label(RichText::new(format!("Android {}", ver)).size(11.0).color(Color32::from_rgb(140, 180, 220)));
                            }
                        });
                    });

                    if btn_response.response.interact(egui::Sense::click()).clicked() {
                        self.selected_device_idx = idx;
                    }
                });
            }
        });

        ui.add_space(16.0);

        // Mirroring Action Card
        egui::Frame::none()
            .fill(Color32::from_rgb(24, 28, 36))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 55, 70)))
            .rounding(10.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let is_running = self.active_child.is_some();

                    if !is_running {
                        let btn = egui::Button::new(
                            RichText::new("🚀 Lancer le Mirroring ScreenCopy")
                                .size(16.0)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(0, 150, 230))
                        .min_size(Vec2::new(260.0, 44.0));

                        if ui.add(btn).clicked() {
                            self.start_mirroring();
                        }
                    } else {
                        let btn = egui::Button::new(
                            RichText::new("⏹ Arrêter le Mirroring")
                                .size(16.0)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(220, 38, 38))
                        .min_size(Vec2::new(260.0, 44.0));

                        if ui.add(btn).clicked() {
                            self.stop_mirroring();
                        }
                    }

                    ui.add_space(16.0);

                    // Stay Awake Big Checkbox
                    let mut stay_awake = self.config.stay_awake;
                    if ui
                        .checkbox(&mut stay_awake, RichText::new("🔆 Garder l'écran allumé pendant l'utilisation").size(14.0).strong().color(Color32::WHITE))
                        .changed()
                    {
                        self.config.stay_awake = stay_awake;
                        self.toggle_stay_awake_setting();
                    }

                    ui.add_space(12.0);

                    // Turn physical screen off checkbox
                    let mut turn_off = self.config.turn_off_physical_screen;
                    if ui
                        .checkbox(&mut turn_off, RichText::new("📱 Éteindre l'écran physique (économie batterie)").size(13.0).color(Color32::from_rgb(200, 210, 225)))
                        .changed()
                    {
                        self.config.turn_off_physical_screen = turn_off;
                    }
                });

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                // Quick Navigation & Power Buttons for Phone
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Raccourcis Téléphone :").size(13.0).color(Color32::from_rgb(160, 175, 195)));
                    ui.add_space(8.0);

                    if let Some(serial) = self.selected_serial() {
                        let serial_clone = serial.clone();

                        if ui.button("🔆 Réveiller").clicked() {
                            let s = serial_clone.clone();
                            thread::spawn(move || { let _ = wake_screen(&s); });
                        }
                        if ui.button("🌙 Mettre en veille").clicked() {
                            let s = serial_clone.clone();
                            thread::spawn(move || { let _ = sleep_screen(&s); });
                        }
                        if ui.button("⚡ Power").clicked() {
                            let s = serial_clone.clone();
                            thread::spawn(move || { let _ = toggle_power(&s); });
                        }
                        if ui.button("◀ Retour").clicked() {
                            let s = serial_clone.clone();
                            thread::spawn(move || { let _ = send_key_event(&s, "4"); });
                        }
                        if ui.button("🏠 Accueil").clicked() {
                            let s = serial_clone.clone();
                            thread::spawn(move || { let _ = send_key_event(&s, "3"); });
                        }
                        if ui.button("📑 Récents").clicked() {
                            let s = serial_clone.clone();
                            thread::spawn(move || { let _ = send_key_event(&s, "187"); });
                        }
                        if ui.button("🔊 Vol +").clicked() {
                            let s = serial_clone.clone();
                            thread::spawn(move || { let _ = send_key_event(&s, "24"); });
                        }
                        if ui.button("🔉 Vol -").clicked() {
                            let s = serial_clone.clone();
                            thread::spawn(move || { let _ = send_key_event(&s, "25"); });
                        }
                    }
                });
            });

        ui.add_space(16.0);

        // Fast Quick Transfer Box
        egui::Frame::none()
            .fill(Color32::from_rgb(24, 28, 36))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 55, 70)))
            .rounding(10.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚡ Transfert Rapide").size(15.0).strong().color(Color32::WHITE));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Ouvrir l'explorateur complet 📂").clicked() {
                            self.active_tab = ActiveTab::FileTransfer;
                            self.refresh_files();
                        }
                    });
                });

                ui.add_space(8.0);
                ui.label(
                    RichText::new("Glissez n'importe quel fichier ou dossier de Windows directement dans cette fenêtre pour l'envoyer au téléphone !")
                        .size(13.0)
                        .color(Color32::from_rgb(0, 195, 255)),
                );

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("📤 Envoyer des fichiers vers Téléchargements...").clicked() {
                        if let Some(files) = rfd::FileDialog::new().pick_files() {
                            for f in files {
                                self.queue_upload(f, false);
                            }
                        }
                    }

                    if ui.button("📦 Installer un fichier APK...").clicked() {
                        if let Some(file) = rfd::FileDialog::new()
                            .add_filter("Application Android", &["apk"])
                            .pick_file()
                        {
                            self.queue_upload(file, true);
                        }
                    }
                });
            });
    }

    fn render_file_transfer_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Explorateur & Transfert de Fichiers").color(Color32::WHITE));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("🔄 Actualiser").clicked() {
                    self.refresh_files();
                }
                if ui.button("➕ Nouveau Dossier").clicked() {
                    self.show_new_folder_dialog = true;
                }
                if ui.button("📦 Installer un APK...").clicked() {
                    if let Some(file) = rfd::FileDialog::new()
                        .add_filter("Application Android", &["apk"])
                        .pick_file()
                    {
                        self.queue_upload(file, true);
                    }
                }
                if ui.button("📤 Envoyer Fichier(s)...").clicked() {
                    if let Some(files) = rfd::FileDialog::new().pick_files() {
                        for f in files {
                            self.queue_upload(f, false);
                        }
                    }
                }
            });
        });

        ui.add_space(8.0);

        // Shortcuts Row
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Accès rapide :").size(12.0).color(Color32::from_rgb(140, 155, 175)));
            for sc in SHORTCUTS {
                let is_current = self.current_remote_path == sc.path;
                let btn_text = format!("{} {}", sc.icon, sc.name);
                if ui.selectable_label(is_current, btn_text).clicked() {
                    self.current_remote_path = sc.path.to_string();
                    self.refresh_files();
                }
            }
        });

        ui.add_space(8.0);

        // Breadcrumb & Search Bar
        ui.horizontal(|ui| {
            ui.label(RichText::new("📍 Dossier :").size(13.0).color(Color32::from_rgb(160, 175, 195)));
            ui.monospace(RichText::new(&self.current_remote_path).color(Color32::from_rgb(0, 195, 255)).strong());

            // Back button
            if self.current_remote_path != "/sdcard" && self.current_remote_path != "/" {
                if ui.button("⬆ Dossier Parent").clicked() {
                    if let Some(parent) = Path::new(&self.current_remote_path).parent() {
                        let p = parent.to_string_lossy().replace('\\', "/");
                        self.current_remote_path = if p.is_empty() { "/".to_string() } else { p };
                        self.refresh_files();
                    }
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.file_search).hint_text("🔍 Filtrer les fichiers..."));
            });
        });

        ui.add_space(8.0);

        // New folder popup
        if self.show_new_folder_dialog {
            egui::Window::new("Créer un nouveau dossier")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label("Nom du dossier :");
                    ui.text_edit_singleline(&mut self.new_folder_name);
                    ui.horizontal(|ui| {
                        if ui.button("Créer").clicked() && !self.new_folder_name.is_empty() {
                            if let Some(serial) = self.selected_serial() {
                                let path = format!("{}/{}", self.current_remote_path, self.new_folder_name);
                                let tx = self.event_tx.clone();
                                thread::spawn(move || {
                                    let res = create_remote_folder(&serial, &path);
                                    let _ = tx.send(AppEvent::ActionFinished(res.map(|_| "Dossier créé !".to_string())));
                                });
                            }
                            self.new_folder_name.clear();
                            self.show_new_folder_dialog = false;
                            self.refresh_files();
                        }
                        if ui.button("Annuler").clicked() {
                            self.show_new_folder_dialog = false;
                        }
                    });
                });
        }

        // Active Transfers Box (if any)
        if !self.transfers.is_empty() {
            egui::Frame::none()
                .fill(Color32::from_rgb(26, 32, 42))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 55, 75)))
                .rounding(6.0)
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Transferts récents").strong().size(12.0).color(Color32::WHITE));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Effacer l'historique").clicked() {
                                self.transfers.clear();
                            }
                        });
                    });
                    ui.add_space(4.0);

                    for item in self.transfers.iter().take(3) {
                        ui.horizontal(|ui| {
                            let icon = if item.is_upload { "📤" } else { "📥" };
                            ui.label(format!("{} {}", icon, item.file_name));
                            ui.label(RichText::new(format_bytes(item.file_size)).size(11.0).color(Color32::from_rgb(140, 150, 165)));

                            match &item.status {
                                TransferStatus::Queued => {
                                    ui.label(RichText::new("En attente...").color(Color32::YELLOW));
                                }
                                TransferStatus::Transferring => {
                                    ui.spinner();
                                    ui.label(RichText::new(&item.message).color(Color32::from_rgb(0, 195, 255)));
                                }
                                TransferStatus::Completed => {
                                    ui.label(RichText::new("✓ Terminé").color(Color32::from_rgb(74, 222, 128)));
                                }
                                TransferStatus::Failed(err) => {
                                    ui.label(RichText::new(format!("✗ {}", err)).color(Color32::from_rgb(248, 113, 113)));
                                }
                            }
                        });
                    }
                });
            ui.add_space(8.0);
        }

        // File List Table
        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.is_loading_files {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.spinner();
                    ui.label("Chargement des fichiers...");
                });
                return;
            }

            if self.remote_files.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.label("Dossier vide ou aucun fichier trouvé.");
                });
                return;
            }

            let search = self.file_search.to_lowercase();
            let mut folder_to_navigate: Option<String> = None;
            let mut file_to_download: Option<RemoteFile> = None;
            let mut file_to_delete: Option<String> = None;

            egui::Grid::new("files_grid")
                .striped(true)
                .min_col_width(80.0)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Nom").strong().color(Color32::from_rgb(180, 195, 215)));
                    ui.label(RichText::new("Taille").strong().color(Color32::from_rgb(180, 195, 215)));
                    ui.label(RichText::new("Date").strong().color(Color32::from_rgb(180, 195, 215)));
                    ui.label(RichText::new("Actions").strong().color(Color32::from_rgb(180, 195, 215)));
                    ui.end_row();

                    for file in &self.remote_files {
                        if !search.is_empty() && !file.name.to_lowercase().contains(&search) {
                            continue;
                        }

                        let icon = if file.is_dir {
                            "📁"
                        } else if file.name.ends_with(".apk") {
                            "📦"
                        } else if file.name.ends_with(".jpg") || file.name.ends_with(".png") || file.name.ends_with(".webp") {
                            "🖼️"
                        } else if file.name.ends_with(".mp4") || file.name.ends_with(".mkv") {
                            "🎬"
                        } else if file.name.ends_with(".mp3") || file.name.ends_with(".flac") {
                            "🎵"
                        } else {
                            "📄"
                        };

                        // Name column
                        let name_label = format!("{} {}", icon, file.name);
                        if file.is_dir {
                            if ui
                                .link(RichText::new(name_label).strong().color(Color32::from_rgb(0, 195, 255)))
                                .clicked()
                            {
                                folder_to_navigate = Some(file.path.clone());
                            }
                        } else {
                            ui.label(RichText::new(name_label).color(Color32::WHITE));
                        }

                        // Size column
                        if file.is_dir {
                            ui.label(RichText::new("—").color(Color32::from_rgb(120, 130, 140)));
                        } else {
                            ui.label(RichText::new(format_bytes(file.size)).color(Color32::from_rgb(180, 190, 205)));
                        }

                        // Date column
                        ui.label(RichText::new(&file.modified).size(11.0).color(Color32::from_rgb(140, 150, 165)));

                        // Action buttons
                        ui.horizontal(|ui| {
                            if !file.is_dir {
                                if ui.button("⬇ Télécharger").clicked() {
                                    file_to_download = Some(file.clone());
                                }
                            }
                            if ui.button("🗑").on_hover_text("Supprimer").clicked() {
                                file_to_delete = Some(file.path.clone());
                            }
                        });

                        ui.end_row();
                    }
                });

            if let Some(folder) = folder_to_navigate {
                self.current_remote_path = folder;
                self.refresh_files();
            }

            if let Some(file) = file_to_download {
                self.queue_download(file);
            }

            if let Some(path) = file_to_delete {
                if let Some(serial) = self.selected_serial() {
                    let tx = self.event_tx.clone();
                    thread::spawn(move || {
                        let res = delete_remote(&serial, &path);
                        let _ = tx.send(AppEvent::ActionFinished(res.map(|_| "Élément supprimé !".to_string())));
                    });
                    self.refresh_files();
                }
            }
        });
    }

    fn render_settings_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("Réglages ScreenCopyPLUS").color(Color32::WHITE));
        ui.add_space(12.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            // Screen & Power settings
            egui::Frame::none()
                .fill(Color32::from_rgb(24, 28, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 55, 70)))
                .rounding(8.0)
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("🔆 Gestion de l'Écran (Stay Awake)").size(16.0).strong().color(Color32::WHITE));
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.config.stay_on_mode, StayOnMode::Always, "Toujours allumé (Infini)");
                        ui.radio_value(&mut self.config.stay_on_mode, StayOnMode::UsbOnly, "Allumé pendant la charge USB");
                        ui.radio_value(&mut self.config.stay_on_mode, StayOnMode::SystemDefault, "Veille par défaut du système");
                    });

                    ui.add_space(8.0);
                    ui.checkbox(&mut self.config.auto_stay_awake_on_connect, "Activer automatiquement le maintien allumé dès la connexion d'un appareil");

                    ui.add_space(12.0);
                    ui.label("Délai d'extinction de l'écran (Timeout système) :");
                    ui.horizontal(|ui| {
                        if ui.button("30 secondes").clicked() {
                            if let Some(serial) = self.selected_serial() {
                                let _ = set_screen_timeout_ms(&serial, 30_000);
                                self.notify("Délai réglé sur 30 secondes", true);
                            }
                        }
                        if ui.button("2 minutes").clicked() {
                            if let Some(serial) = self.selected_serial() {
                                let _ = set_screen_timeout_ms(&serial, 120_000);
                                self.notify("Délai réglé sur 2 minutes", true);
                            }
                        }
                        if ui.button("10 minutes").clicked() {
                            if let Some(serial) = self.selected_serial() {
                                let _ = set_screen_timeout_ms(&serial, 600_000);
                                self.notify("Délai réglé sur 10 minutes", true);
                            }
                        }
                        if ui.button("30 minutes").clicked() {
                            if let Some(serial) = self.selected_serial() {
                                let _ = set_screen_timeout_ms(&serial, 1_800_000);
                                self.notify("Délai réglé sur 30 minutes", true);
                            }
                        }
                        if ui.button("Infini (24 jours)").clicked() {
                            if let Some(serial) = self.selected_serial() {
                                let _ = set_screen_timeout_ms(&serial, 2_147_483_647);
                                self.notify("Écran réglé sur veille infinie !", true);
                            }
                        }
                    });
                });

            ui.add_space(16.0);

            // Scrcpy Video & Performance settings
            egui::Frame::none()
                .fill(Color32::from_rgb(24, 28, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 55, 70)))
                .rounding(8.0)
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("🎬 Paramètres Vidéo & Mirroring").size(16.0).strong().color(Color32::WHITE));
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.config.always_on_top, "Fenêtre toujours au premier plan (--always-on-top)");
                        ui.add_space(16.0);
                        ui.checkbox(&mut self.config.fullscreen, "Mode Plein Écran (--fullscreen)");
                        ui.add_space(16.0);
                        ui.checkbox(&mut self.config.enable_audio, "Transmettre le son audio");
                    });

                    ui.add_space(12.0);

                    ui.horizontal(|ui| {
                        ui.label("Débit vidéo (Bitrate) :");
                        ui.add(egui::Slider::new(&mut self.config.bitrate_mbps, 2..=32).suffix(" Mbps"));
                    });

                    ui.horizontal(|ui| {
                        ui.label("Limite d'images par seconde (FPS) :");
                        ui.add(egui::Slider::new(&mut self.config.max_fps, 30..=120).suffix(" FPS"));
                    });

                    ui.horizontal(|ui| {
                        ui.label("Résolution maximale (0 = Définition native) :");
                        ui.add(egui::Slider::new(&mut self.config.max_size, 0..=2560).suffix(" px"));
                    });
                });

            ui.add_space(16.0);

            // Wireless ADB Connect
            egui::Frame::none()
                .fill(Color32::from_rgb(24, 28, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 55, 70)))
                .rounding(8.0)
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("📶 Connexion Sans Fil (WiFi / TCP/IP)").size(16.0).strong().color(Color32::WHITE));
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        if ui.button("1. Activer le mode TCP/IP (port 5555)").clicked() {
                            if let Some(serial) = self.selected_serial() {
                                match enable_tcpip(&serial, 5555) {
                                    Ok(msg) => self.notify(format!("TCP/IP activé: {}", msg), true),
                                    Err(err) => self.notify(err, false),
                                }
                            }
                        }
                        ui.label("(Branchez d'abord en USB, puis cliquez sur ce bouton)");
                    });

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Adresse IP de l'appareil :");
                        ui.text_edit_singleline(&mut self.wireless_address_input);

                        if ui.button("Connecter sans fil").clicked() {
                            let addr = format!("{}:{}", self.wireless_address_input.trim(), self.tcpip_port_input.trim());
                            match connect_wireless(&addr) {
                                Ok(msg) => {
                                    self.notify(msg, true);
                                    self.refresh_devices();
                                }
                                Err(err) => self.notify(err, false),
                            }
                        }

                        if ui.button("Déconnecter").clicked() {
                            let addr = format!("{}:{}", self.wireless_address_input.trim(), self.tcpip_port_input.trim());
                            let _ = disconnect_wireless(&addr);
                            self.refresh_devices();
                        }
                    });
                });

            ui.add_space(16.0);

            // Folder download path
            egui::Frame::none()
                .fill(Color32::from_rgb(24, 28, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 55, 70)))
                .rounding(8.0)
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("📁 Dossier de destination sur le PC").size(16.0).strong().color(Color32::WHITE));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.monospace(&self.config.default_download_dir);
                        if ui.button("Parcourir...").clicked() {
                            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                self.config.default_download_dir = folder.to_string_lossy().to_string();
                            }
                        }
                    });
                });
        });
    }

    fn render_about_tab(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .fill(Color32::from_rgb(24, 28, 36))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 55, 70)))
            .rounding(10.0)
            .inner_margin(24.0)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("ScreenCopyPLUS")
                        .size(24.0)
                        .strong()
                        .color(Color32::from_rgb(0, 195, 255)),
                );
                ui.add_space(4.0);
                ui.label(RichText::new("Fork officiel de Genymobile/scrcpy • Édition Rust").size(14.0).color(Color32::from_rgb(180, 195, 215)));
                ui.add_space(16.0);

                ui.label(RichText::new("Fonctionnalités Clés :").size(16.0).strong().color(Color32::WHITE));
                ui.add_space(8.0);
                ui.label("✨ Système de transfert de fichiers ultra-rapide et bidirectionnel (PC <-> Téléphone)");
                ui.label("✨ Glisser-déposer (Drag & Drop) instantané de n'importe quel fichier ou dossier");
                ui.label("✨ Aucune fenêtre d'invite de commande (CMD/Console) ouverte (100% GUI Windows natif)");
                ui.label("✨ Écran toujours allumé (Stay Awake) automatique et configurable");
                ui.label("✨ Entièrement développé en Rust pour une légèreté et des performances maximales (< 30 Mo RAM)");
                ui.label("✨ Support du mirroring ultra-rapide à 60/120 FPS avec audio et commandes tactiles");

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(16.0);

                ui.horizontal(|ui| {
                    ui.label("Dépôt original :");
                    ui.hyperlink_to("https://github.com/Genymobile/scrcpy", "https://github.com/Genymobile/scrcpy");
                });
            });
    }
}
