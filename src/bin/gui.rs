use clap::Parser;
use eframe::{egui, App, Frame};
use input_linux::Key;
use std::{path::PathBuf, sync::mpsc, thread};

use theclicker::{Args, AutoclickerState, Command, InputDevice, TheClicker};

#[derive(Clone, Copy, PartialEq, Debug)]
enum BindTarget {
    Left,
    Middle,
    Right,
    LockUnlock,
}

struct CaptureRequest {
    target: BindTarget,
    rx: mpsc::Receiver<Result<u16, String>>,
}

struct AppState {
    devices: Vec<InputDevice>,
    selected_index: Option<usize>,
    left_bind: Option<u16>,
    middle_bind: Option<u16>,
    right_bind: Option<u16>,
    lock_bind: Option<u16>,
    hold: bool,
    grab: bool,
    beep: bool,
    debug: bool,
    cooldown: u64,
    cooldown_press_release: u64,
    capture: Option<CaptureRequest>,
    clicker_rx: Option<mpsc::Receiver<AutoclickerState>>,
    state: AutoclickerState,
    message: Option<String>,
    device_error_hint: Option<String>,
}

impl AppState {
    fn new(debug: bool, beep: bool) -> Self {
        let devices = Self::load_devices();
        Self {
            devices,
            selected_index: None,
            left_bind: None,
            middle_bind: None,
            right_bind: None,
            lock_bind: None,
            hold: true,
            grab: true,
            beep,
            debug,
            cooldown: 25,
            cooldown_press_release: 0,
            capture: None,
            clicker_rx: None,
            state: AutoclickerState::default(),
            message: None,
            device_error_hint: None,
        }
    }

    fn load_devices() -> Vec<InputDevice> {
        let mut devices = InputDevice::devices();
        devices.retain(|device| {
            if let Ok(event_bits) = device.handler.event_bits() {
                event_bits.get(input_linux::EventKind::Key)
            } else {
                true
            }
        });
        devices
    }

    fn selected_device(&self) -> Option<&InputDevice> {
        self.selected_index.and_then(|idx| self.devices.get(idx))
    }

    fn selected_device_path(&self) -> Option<PathBuf> {
        self.selected_device().map(|device| device.path.clone())
    }

    fn selected_device_is_legacy(&self) -> bool {
        self.selected_device()
            .map(|device| device.filename.starts_with("mouse"))
            .unwrap_or(false)
    }

    fn bind_label(bind: Option<u16>) -> String {
        bind.map(|code| {
            if let Ok(key) = Key::from_code(code) {
                format!("{:?} ({code})", key)
            } else {
                format!("{code}")
            }
        })
        .unwrap_or_else(|| "<none>".to_owned())
    }

    fn refresh_devices(&mut self) {
        let current = self.selected_device().map(|device| device.path.clone());
        self.devices = Self::load_devices();
        self.selected_index = current.and_then(|path| {
            self.devices
                .iter()
                .position(|device| device.path == path)
        });
        if self.devices.is_empty() {
            self.device_error_hint = Some(
                "No input devices found. Make sure you are running the GUI as your normal user, not with sudo, and that your account is in the input group. After adding the group, log out and log back in.".to_owned(),
            );
        } else {
            self.device_error_hint = None;
        }
    }

    fn capture_bind(&mut self, target: BindTarget) {
        if self.capture.is_some() {
            self.message = Some("Capture already in progress".to_owned());
            return;
        }

        let path = match self.selected_device_path() {
            Some(path) => path,
            None => {
                self.message = Some("Select an input device first".to_owned());
                return;
            }
        };

        let (tx, rx) = mpsc::channel();
        self.capture = Some(CaptureRequest { target, rx });
        self.message = Some(format!("Capturing {:?} bind on {}...", target, path.display()));

        thread::spawn(move || {
            let result = match InputDevice::dev_open(path.clone()) {
                Ok(device) => {
                    let _ = device.grab(true);
                    let mut events: [input_linux::sys::input_event; 1] = unsafe { std::mem::zeroed() };
                    device.empty_read_buffer();
                    let mut captured = None;
                    while captured.is_none() {
                        match device.read(&mut events) {
                            Ok(len) => {
                                for event in &events[..len] {
                                    if event.type_ == input_linux::sys::EV_KEY as u16
                                        && matches!(event.value, 1 | 2)
                                    {
                                        captured = Some(event.code);
                                        break;
                                    }
                                }
                            }
                            Err(err) => {
                                let _ = tx.send(Err(format!("Failed to read device: {err}")));
                                break;
                            }
                        }
                    }
                    let _ = device.grab(false);
                    if let Some(code) = captured {
                        Ok(code)
                    } else {
                        Err("No key captured".to_owned())
                    }
                }
                Err(err) => Err(err),
            };
            let _ = tx.send(result);
        });
    }

    fn try_process_capture(&mut self) {
        if let Some(capture) = &self.capture {
            if let Ok(result) = capture.rx.try_recv() {
                match result {
                    Ok(code) => {
                        match capture.target {
                            BindTarget::Left => self.left_bind = Some(code),
                            BindTarget::Middle => self.middle_bind = Some(code),
                            BindTarget::Right => self.right_bind = Some(code),
                            BindTarget::LockUnlock => self.lock_bind = Some(code),
                        }
                        self.message = Some(format!("Captured key {}", code));
                    }
                    Err(err) => {
                        self.message = Some(err);
                    }
                }
                self.capture = None;
            }
        }
    }

    fn start_clicker(&mut self) {
        let selected_device = match self.selected_device() {
            Some(device) => device,
            None => {
                self.message = Some("Please select an input device before starting".to_owned());
                return;
            }
        };

        let command = if self.selected_device_is_legacy() {
            Command::RunLegacy {
                device_query: selected_device.path.to_string_lossy().to_string(),
                cooldown: self.cooldown,
                cooldown_press_release: self.cooldown_press_release,
            }
        } else {
            Command::Run {
                device_query: selected_device.path.to_string_lossy().to_string(),
                left_bind: self.left_bind,
                middle_bind: self.middle_bind,
                right_bind: self.right_bind,
                lock_unlock_bind: self.lock_bind,
                hold: self.hold,
                grab: self.grab,
                cooldown: self.cooldown,
                cooldown_press_release: self.cooldown_press_release,
            }
        };

        let clicker = TheClicker::from_command(self.debug, self.beep, command);
        self.clicker_rx = Some(clicker.spawn());
        self.message = Some("Clicker started".to_owned());
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(false, false)
    }
}

impl App for AppState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        self.try_process_capture();
        if let Some(rx) = &self.clicker_rx {
            while let Ok(new_state) = rx.try_recv() {
                self.state = new_state;
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("TheClicker GUI");
            ui.horizontal(|ui| {
                if ui.button("Refresh devices").clicked() {
                    self.refresh_devices();
                }
                if ui.button("Clear binds").clicked() {
                    self.left_bind = None;
                    self.middle_bind = None;
                    self.right_bind = None;
                    self.lock_bind = None;
                    self.message = Some("Bindings cleared".to_owned());
                }
            });

            ui.separator();

            ui.columns(2, |columns| {
                let left = &mut columns[0];
                left.label("Input device");
                if self.devices.is_empty() {
                    left.label("No input devices found");
                    if let Some(hint) = &self.device_error_hint {
                        left.separator();
                        left.label("Hint:");
                        left.label(hint.replace("\n", " "));
                    }
                } else {
                    egui::ComboBox::from_label("Device")
                        .selected_text(self.selected_device().map(|device| device.name.as_str()).unwrap_or("Select device"))
                        .show_ui(left, |ui| {
                            for (index, device) in self.devices.iter().enumerate() {
                                if ui
                                    .selectable_value(&mut self.selected_index, Some(index), device.name.as_str())
                                    .clicked()
                                {
                                    self.message = Some(format!("Selected {}", device.name));
                                }
                            }
                        });

                    if let Some(device) = self.selected_device() {
                        left.label(format!("Path: {}", device.path.display()));
                        left.label(format!("Filename: {}", device.filename));
                        left.label(format!("Legacy device: {}", self.selected_device_is_legacy()));
                    }
                }

                let right = &mut columns[1];
                right.label("Options");
                right.checkbox(&mut self.beep, "Beep on state change");
                right.checkbox(&mut self.debug, "Debug output");
                right.checkbox(&mut self.hold, "Hold mode");
                right.checkbox(&mut self.grab, "Grab device");
                right.add(egui::Slider::new(&mut self.cooldown, 25..=1000).text("Cooldown ms"));
                right.add(egui::Slider::new(&mut self.cooldown_press_release, 0..=500).text("Press/release delay ms"));
            });

            ui.separator();

            if self.selected_device_is_legacy() {
                ui.label("Selected device uses legacy clicker mode. Bind capture is disabled.");
            } else {
                ui.label("Bindings");
                ui.horizontal(|ui| {
                    ui.label(format!("Left: {}", Self::bind_label(self.left_bind)));
                    if ui.button("Capture").clicked() {
                        self.capture_bind(BindTarget::Left);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(format!("Middle: {}", Self::bind_label(self.middle_bind)));
                    if ui.button("Capture").clicked() {
                        self.capture_bind(BindTarget::Middle);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(format!("Right: {}", Self::bind_label(self.right_bind)));
                    if ui.button("Capture").clicked() {
                        self.capture_bind(BindTarget::Right);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(format!("Lock toggle: {}", Self::bind_label(self.lock_bind)));
                    if ui.button("Capture").clicked() {
                        self.capture_bind(BindTarget::LockUnlock);
                    }
                });
            }

            ui.separator();
            if ui.button("Start Clicker").clicked() {
                if self.clicker_rx.is_none() {
                    self.start_clicker();
                } else {
                    self.message = Some("Clicker is already running".to_owned());
                }
            }

            ui.separator();
            ui.label("Clicker state");
            ui.label(format!("Locked: {}", self.state.locked()));
            ui.label(format!("Left: {}", self.state.left()));
            ui.label(format!("Middle: {}", self.state.middle()));
            ui.label(format!("Right: {}", self.state.right()));

            if let Some(message) = &self.message {
                ui.separator();
                ui.colored_label(egui::Color32::YELLOW, message);
            }
        });
    }
}

fn main() {
    unsafe { std::env::set_var("WINIT_UNIX_BACKEND", "wayland"); }
    let args = Args::parse();
    let mut app = AppState::new(args.debug, args.beep);

    if let Some(command) = args.command {
        let clicker = TheClicker::from_command(args.debug, args.beep, command);
        app.clicker_rx = Some(clicker.spawn());
        app.message = Some("Clicker started from CLI command".to_owned());
    }

    let mut native_options = eframe::NativeOptions::default();
    native_options.viewport = egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_active(true)
        .with_visible(true)
        .with_always_on_top()
        .with_resizable(false)
        .with_inner_size(egui::vec2(420.0, 520.0));
    native_options.centered = true;
    match eframe::run_native(
        "TheClicker GUI",
        native_options,
        Box::new(|_cc| Box::new(app)),
    ) {
        Ok(()) => {}
        Err(err) => {
            eprintln!("GUI failed: {err}");
            std::process::exit(1);
        }
    }
}
