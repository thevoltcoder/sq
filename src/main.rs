use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use sq::sys;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

mod filters;

slint::slint! {
    #[edition = "1.19"]

    import { Button, Slider, CheckBox, TabWidget, LineEdit, ScrollView } from "std-widgets.slint";

    export global Bridge {
        callback edited(int, bool, float);
    }

    component Param inherits HorizontalLayout {
        in property <int> idx;
        in property <string> label;
        in property <float> lo;
        in property <float> hi;
        in property <float> initial;
        in property <string> unit: "";
        in property <bool> optional: true;
        property <bool> on: !root.optional;
        property <float> val: root.initial;
        property <string> txt: "" + Math.round(root.initial * 100) / 100;
        spacing: 10px;

        changed val => {
            root.txt = "" + Math.round(root.val * 100) / 100;
        }

        Text {
            text: root.label;
            width: 150px;
            vertical-alignment: center;
        }

        if root.optional : CheckBox {
            checked: root.on;
            toggled => {
                root.on = self.checked;
                Bridge.edited(root.idx, root.on, root.val);
            }
        }

        Slider {
            minimum: root.lo;
            maximum: root.hi;
            value: root.val;
            enabled: root.on;
            changed(v) => {
                root.val = v;
                Bridge.edited(root.idx, root.on, v);
            }
        }

        if root.unit != "" : LineEdit {
            width: 80px;
            enabled: root.on;
            text <=> root.txt;
            input-type: InputType.decimal;
            accepted(t) => {
                root.val = Math.clamp(t.to-float(), root.lo, root.hi);
                root.txt = "" + Math.round(root.val * 100) / 100;
                Bridge.edited(root.idx, root.on, root.val);
            }
        }

        Text {
            text: root.on ? "" + Math.round(root.val * 100) / 100 + root.unit : "off";
            width: 90px;
            horizontal-alignment: right;
            vertical-alignment: center;
        }
    }

    export component MainWindow inherits Window {
        title: "Audio Fade";
        min-width: 440px;
        min-height: 260px;

        callback start(string, float);
        callback end(float);
        callback show-filters();
        callback browse();
        callback gain-changed(float);

        in-out property <string> path;
        property <float> gain: 0.0;
        property <float> fade: 2.0;
        property <string> gain-txt: "0";
        property <string> fade-txt: "2";
        in property <[string]> warnings;
        in property <string> error;

        changed gain => {
            root.gain-txt = "" + Math.round(root.gain);
            root.gain-changed(root.gain);
        }

        changed fade => {
            root.fade-txt = "" + Math.round(root.fade * 10) / 10;
        }

        VerticalLayout {
            padding: 24px;
            spacing: 14px;
            alignment: LayoutAlignment.start;

            HorizontalLayout {
                spacing: 10px;
                Text {
                    text: "File";
                    width: 70px;
                    vertical-alignment: center;
                }
                LineEdit {
                    text <=> root.path;
                }
                Button {
                    text: "Browse...";
                    width: 70px;
                    clicked => {
                        root.browse();
                    }
                }
            }

            HorizontalLayout {
                spacing: 10px;
                Text {
                    text: "Gain";
                    width: 70px;
                    vertical-alignment: center;
                }
                Slider {
                    minimum: -60;
                    maximum: 60;
                    value: root.gain;
                    changed(g) => {
                        root.gain = g;
                    }
                }
                LineEdit {
                    width: 60px;
                    text <=> root.gain-txt;
                    input-type: InputType.decimal;
                    accepted(t) => {
                        root.gain = Math.clamp(t.to-float(), -60, 60);
                        root.gain-txt = "" + Math.round(root.gain);
                    }
                }
                Text {
                    text: root.gain.round() + " dB";
                    width: 60px;
                    horizontal-alignment: right;
                    vertical-alignment: center;
                }
            }

            HorizontalLayout {
                spacing: 10px;
                Text {
                    text: "Fade";
                    width: 70px;
                    vertical-alignment: center;
                }
                Slider {
                    minimum: 0;
                    maximum: 10;
                    value: root.fade;
                    changed(v) => {
                        root.fade = v;
                    }
                }
                LineEdit {
                    width: 60px;
                    text <=> root.fade-txt;
                    input-type: InputType.decimal;
                    accepted(t) => {
                        root.fade = Math.clamp(t.to-float(), 0, 10);
                        root.fade-txt = "" + Math.round(root.fade * 10) / 10;
                    }
                }
                Text {
                    text: "" + Math.round(root.fade * 10) / 10 + " s";
                    width: 60px;
                    horizontal-alignment: right;
                    vertical-alignment: center;
                }
            }

            HorizontalLayout {
                spacing: 12px;
                Button {
                    text: "Start";
                    clicked => {
                        root.start(root.path, root.gain);
                    }
                }
                Button {
                    text: "End";
                    clicked => {
                        root.end(root.fade);
                    }
                }
                Button {
                    text: "Filters";
                    clicked => {
                        root.show-filters();
                    }
                }
            }

            Rectangle {
                background: #e3e3e3;
                height: 44px;
                border-radius: 4px;
                clip: true;
                visible: root.warnings.length > 0;
                ScrollView {
                    VerticalLayout {
                        padding: 6px;
                        spacing: 4px;
                        alignment: LayoutAlignment.start;
                        for warn[i] in root.warnings: Text {
                            text: warn;
                            font-size: 10px;
                        }
                    }
                }
            }

            Text {
                visible: root.error != "";
                text: root.error;
            }
        }
    }

    export component FilterWindow inherits Window {
        title: "Audio Filters";
        min-width: 600px;
        min-height: 400px;

        TabWidget {
            Tab {
                title: "General";
                ScrollView {
                    VerticalLayout {
                        padding: 16px;
                        spacing: 12px;
                        alignment: LayoutAlignment.start;
                        Param { idx: 0; label: "Preamp"; lo: -24; hi: 24; initial: 0; unit: " dB"; }
                        Param { idx: 1; label: "High-pass"; lo: 20; hi: 2000; initial: 80; unit: " Hz"; }
                        Param { idx: 2; label: "Low-pass"; lo: 1000; hi: 18000; initial: 16000; unit: " Hz"; }
                        Param { idx: 3; label: "Ceiling"; lo: -36; hi: -6; initial: -12; unit: " dBFS"; optional: false; }
                    }
                }
            }

            Tab {
                title: "Gate";
                ScrollView {
                    VerticalLayout {
                        padding: 16px;
                        spacing: 12px;
                        alignment: LayoutAlignment.start;
                        Param { idx: 4; label: "Threshold"; lo: -80; hi: 0; initial: -40; unit: " dB"; }
                        Param { idx: 5; label: "Depth"; lo: -80; hi: 0; initial: -40; unit: " dB"; optional: false; }
                        Param { idx: 6; label: "Attack"; lo: 0.01; hi: 200; initial: 1; unit: " ms"; optional: false; }
                        Param { idx: 7; label: "Release"; lo: 0.01; hi: 1000; initial: 150; unit: " ms"; optional: false; }
                    }
                }
            }

            Tab {
                title: "Compressor";
                ScrollView {
                    VerticalLayout {
                        padding: 16px;
                        spacing: 12px;
                        alignment: LayoutAlignment.start;
                        Param { idx: 8; label: "Threshold"; lo: -60; hi: 0; initial: -18; unit: " dB"; }
                        Param { idx: 9; label: "Ratio"; lo: 1; hi: 20; initial: 4; unit: " :1"; optional: false; }
                        Param { idx: 10; label: "Attack"; lo: 0.01; hi: 200; initial: 1; unit: " ms"; optional: false; }
                        Param { idx: 11; label: "Release"; lo: 0.01; hi: 1000; initial: 150; unit: " ms"; optional: false; }
                        Param { idx: 12; label: "Makeup"; lo: 0; hi: 12; initial: 0; unit: " dB"; optional: false; }
                    }
                }
            }

            Tab {
                title: "EQ";
                ScrollView {
                    VerticalLayout {
                        padding: 16px;
                        spacing: 8px;
                        alignment: LayoutAlignment.start;
                        Param { idx: 13; label: "Low shelf freq"; lo: 20; hi: 1000; initial: 100; unit: " Hz"; }
                        Param { idx: 14; label: "Low shelf Q"; lo: 0.3; hi: 2; initial: 0.707; optional: false; }
                        Param { idx: 15; label: "Low shelf gain"; lo: -12; hi: 12; initial: 0; unit: " dB"; optional: false; }
                        Param { idx: 16; label: "Bell 1 freq"; lo: 100; hi: 10000; initial: 1000; unit: " Hz"; }
                        Param { idx: 17; label: "Bell 1 Q"; lo: 0.1; hi: 10; initial: 1; optional: false; }
                        Param { idx: 18; label: "Bell 1 gain"; lo: -12; hi: 12; initial: 0; unit: " dB"; optional: false; }
                        Param { idx: 22; label: "Bell 2 freq"; lo: 100; hi: 10000; initial: 3000; unit: " Hz"; }
                        Param { idx: 23; label: "Bell 2 Q"; lo: 0.1; hi: 10; initial: 1; optional: false; }
                        Param { idx: 24; label: "Bell 2 gain"; lo: -12; hi: 12; initial: 0; unit: " dB"; optional: false; }
                        Param { idx: 19; label: "High shelf freq"; lo: 1000; hi: 18000; initial: 8000; unit: " Hz"; }
                        Param { idx: 20; label: "High shelf Q"; lo: 0.3; hi: 2; initial: 0.707; optional: false; }
                        Param { idx: 21; label: "High shelf gain"; lo: -12; hi: 12; initial: 0; unit: " dB"; optional: false; }
                    }
                }
            }
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let params = Arc::new(Mutex::new(filters::Filters::default()));
    let session: Arc<Mutex<Option<sys::Args>>> = Arc::new(Mutex::new(None));

    let main_ui = MainWindow::new()?;
    let filter_ui = FilterWindow::new()?;

    filter_ui.global::<Bridge>().on_edited({
        let params = params.clone();
        let session = session.clone();
        move |idx, on, val| {
            let updated = if let Ok(mut p) = params.lock() {
                p.set(idx as usize, on, val);
                let mut args = sys::Args::default();
                p.apply(&mut args);
                Some(args)
            } else {
                None
            };

            if let Some(updated) = updated {
                if let Ok(slot) = session.lock() {
                    if let Some(args) = slot.as_ref() {
                        args.update_filters(&updated);
                    }
                }
            }
        }
    });

    main_ui.on_gain_changed({
        let session = session.clone();
        move |gain| {
            if let Ok(slot) = session.lock() {
                if let Some(args) = slot.as_ref() {
                    let mut updated = sys::Args::default();
                    updated.postamp = Some(gain);
                    args.update_filters(&updated);
                }
            }
        }
    });

    main_ui.on_start({
        let params = params.clone();
        let session = session.clone();
        let weak = main_ui.as_weak();
        move |path, gain| {
            let Ok(mut slot) = session.lock() else {
                return;
            };
            if let Some(old) = slot.take() {
                old.stop();
            }

            let mut args = sys::Args {
                file: Some(path.to_string()),
                ..Default::default()
            };
            args.postamp = Some(gain);
            if let Ok(p) = params.lock() {
                p.apply(&mut args);
            }
            args.update_filters(&args);

            let thread_args = args.clone();
            let weak = weak.clone();
            std::thread::spawn(move || {
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    ui.set_error("".into());
                });
                if let Err(e) = sys::run(&thread_args) {
                    let msg = format!("ERROR: {e:#}");
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        ui.set_error(msg.into());
                    });
                }
            });
            *slot = Some(args);
        }
    });

    main_ui.on_end({
        let session = session.clone();
        move |secs| {
            let Ok(mut slot) = session.lock() else {
                return;
            };
            if let Some(a) = slot.take() {
                std::thread::spawn(move || {
                    a.fade_to(0.0, Duration::from_secs_f32(secs.max(0.0)));
                    a.stop();
                });
            }
        }
    });

    main_ui.on_show_filters({
        let weak = filter_ui.as_weak();
        move || {
            if let Some(f) = weak.upgrade() {
                let _ = f.show();
            }
        }
    });

    main_ui.on_browse({
        let weak = main_ui.as_weak();
        move || {
            let weak = weak.clone();
            std::thread::spawn(move || {
                let picked = rfd::FileDialog::new()
                    .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "m4a"])
                    .add_filter("All files", &["*"])
                    .pick_file();

                if let Some(p) = picked {
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        ui.set_path(p.to_string_lossy().to_string().into());
                    });
                }
            });
        }
    });

    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), {
        let params = params.clone();
        let weak = main_ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };

            let mut args = sys::Args::default();
            if let Ok(p) = params.lock() {
                p.apply(&mut args);
            }

            let list: Vec<SharedString> = sys::warn_extreme_settings(&args)
                .iter()
                .map(|w| SharedString::from(w.to_string()))
                .collect();

            ui.set_warnings(ModelRc::new(VecModel::from(list)));
        }
    });

    main_ui.window().on_close_requested({
        let weak = filter_ui.as_weak();
        move || {
            if let Some(f) = weak.upgrade() {
                let _ = f.hide();
            }
            slint::CloseRequestResponse::HideWindow
        }
    });

    main_ui.run()
}
