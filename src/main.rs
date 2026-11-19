use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use sq::{
    filters::Filters,
    sys::{Args, run, warn_extreme_settings},
};
use std::{
    cell::RefCell,
    path::Path,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

slint::slint! {
    #[edition = "1.19"]

    import { Button, Slider, CheckBox, TabWidget, LineEdit, ScrollView, Palette } from "std-widgets.slint";

    export global Bridge {
        callback edited(int, bool, float);
    }

    export struct CueData {
        name: string,
        queue: string,
        path: string,
        file: string,
        gain: float,
        error: string,
        warnings: [string],
        delay: float,
        fade_cue: bool,
        fade_target: string,
        fade_duration: float,
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

    component CueRow inherits Rectangle {
        in property <CueData> data;
        in property <bool> selected;
        in property <int> index;

        callback select();

        background: root.selected ? (Palette.color-scheme == ColorScheme.dark ? #34465b : #d5e6fa) : (Math.mod(root.index, 2) == 0 ? (Palette.color-scheme == ColorScheme.dark ? #252525 : #f0f0f0) : (Palette.color-scheme == ColorScheme.dark ? #303030 : #ffffff));
        border-width: root.selected ? 1px : 0px;
        border-color: #3584e4;
        height: 32px;

        TouchArea {
            clicked => { root.select(); }
        }

        HorizontalLayout {
            padding-left: 6px;
            padding-right: 6px;
            spacing: 12px;
            alignment: LayoutAlignment.start;

            Text {
                text: root.data.queue == "" ? "-" : root.data.queue;
                width: 65px;
                font-size: 11px;
                font-weight: 700;
                vertical-alignment: center;
            }
            Text {
                text: (root.data.fade_cue ? "[FADE] " : "") + root.data.name;
                font-size: 12px;
                font-weight: root.selected ? 700 : 400;
                overflow: TextOverflow.elide;
                vertical-alignment: center;
            }
            Text {
                text: root.data.file == "" ? "" : root.data.file;
                font-size: 10px;
                overflow: TextOverflow.elide;
                vertical-alignment: center;
            }
            Text {
                text: "" + root.data.delay + "s";
                width: 55px;
                font-size: 10px;
                horizontal-alignment: right;
                vertical-alignment: center;
            }
        }
    }

    export component MainWindow inherits Window {
        title: "SQ";
        preferred-width: 900px;
        preferred-height: 700px;
        min-width: 700px;
        min-height: 520px;

        in property <[CueData]> cues;
        in-out property <int> selected: 0;

        callback start(int);
        callback end(int);
        callback fade(int);
        callback browse(int);
        callback path-edited(int, string);
        callback gain-changed(int, float);
        callback show-filters(int);
        callback add-cue();
        callback delete-cue(int);
        callback name-edited(int, string);
        callback queue-edited(int, string);
        callback delay-edited(int, float);
        callback fade-cue-edited(int, bool);
        callback fade-target-edited(int, string);
        callback fade-duration-edited(int, float);

        property <string> model-path: root.cues[root.selected].path;
        property <string> model-name: root.cues[root.selected].name;
        property <string> model-queue: root.cues[root.selected].queue;
        property <float> model-gain: root.cues[root.selected].gain;
        property <float> model-delay: root.cues[root.selected].delay;
        property <bool> model-fade-cue: root.cues[root.selected].fade_cue;
        property <string> model-fade-target: root.cues[root.selected].fade_target;
        property <float> model-fade-duration: root.cues[root.selected].fade_duration;
        property <string> path: "";
        property <string> cue-name: "";
        property <string> queue: "";
        property <string> fade-target: "";
        property <string> delay-txt: "0";
        property <string> fade-duration-txt: "1";
        property <float> gain: 0.0;
        property <string> gain-txt: "0";
        property <bool> fade-cue: false;

        changed model-path => { root.path = root.model-path; }
        changed model-name => { root.cue-name = root.model-name; }
        changed model-queue => { root.queue = root.model-queue; }
        changed model-gain => { root.gain = root.model-gain; }
        changed model-delay => { root.delay-txt = "" + root.model-delay; }
        changed model-fade-cue => { root.fade-cue = root.model-fade-cue; }
        changed model-fade-target => { root.fade-target = root.model-fade-target; }
        changed model-fade-duration => { root.fade-duration-txt = "" + root.model-fade-duration; }
        changed gain => { root.gain-txt = "" + Math.round(root.gain); }

        init => { fs.focus(); }

        fs := FocusScope {
            key-pressed(event) => {
                if (event.text == Key.Escape) {
                    root.end(root.selected);
                    return accept;
                }
                if (event.text == " ") {
                    if (root.fade-cue) { root.fade(root.selected); }
                    else { root.start(root.selected); }
                    if (root.selected + 1 < root.cues.length) {
                        root.selected += 1;
                    }
                    return accept;
                }
                return reject;
            }

            VerticalLayout {
                y: 0px;
                padding: 16px;
                spacing: 10px;

                ScrollView {
                    height: root.height - 330px;
                    content-height: root.cues.length * 34px;
                    VerticalLayout {
                        spacing: 2px;
                        alignment: LayoutAlignment.start;
                        for t[i] in root.cues : CueRow {
                            width: root.width - 52px;
                            height: 32px;
                            data: t;
                            index: i;
                            selected: root.selected == i;
                            select() => { root.selected = i; }
                        }
                    }
                }

                Rectangle {
                    height: 285px;
                    background: Palette.color-scheme == ColorScheme.dark ? #202020 : #e8e8e8;
                    border-radius: 5px;
                    VerticalLayout {
                        padding: 10px;
                        spacing: 7px;

                        HorizontalLayout {
                            spacing: 8px;
                            LineEdit {
                                width: 145px;
                                text <=> root.cue-name;
                                placeholder-text: "Cue name";
                                edited(t) => { root.name-edited(root.selected, t); }
                            }
                            LineEdit {
                                width: 75px;
                                text <=> root.queue;
                                placeholder-text: "Queue ID";
                                edited(t) => { root.queue-edited(root.selected, t); }
                            }
                            LineEdit {
                                text <=> root.path;
                                placeholder-text: "Audio file";
                                edited(t) => { root.path-edited(root.selected, t); }
                            }
                            Button {
                                text: "Browse...";
                                clicked => { root.browse(root.selected); }
                            }
                        }

                        HorizontalLayout {
                            spacing: 8px;
                            Text { text: "Delay (s)"; width: 75px; vertical-alignment: center; }
                            LineEdit {
                                width: 90px;
                                text <=> root.delay-txt;
                                input-type: InputType.decimal;
                                accepted(t) => {
                                    root.delay-txt = t;
                                    root.delay-edited(root.selected, Math.max(0, t.to-float()));
                                }
                            }
                            CheckBox {
                                text: "Fade cue";
                                checked: root.fade-cue;
                                toggled => {
                                    root.fade-cue = self.checked;
                                    root.fade-cue-edited(root.selected, self.checked);
                                }
                            }
                            LineEdit {
                                width: 100px;
                                text <=> root.fade-target;
                                enabled: root.fade-cue;
                                placeholder-text: "Target queue ID";
                                edited(t) => { root.fade-target-edited(root.selected, t); }
                            }
                            Text { text: "Fade duration (s)"; vertical-alignment: center; }
                            LineEdit {
                                width: 75px;
                                text <=> root.fade-duration-txt;
                                enabled: root.fade-cue;
                                input-type: InputType.decimal;
                                accepted(t) => {
                                    root.fade-duration-txt = t;
                                    root.fade-duration-edited(root.selected, Math.max(0, t.to-float()));
                                }
                            }
                        }

                        HorizontalLayout {
                            spacing: 8px;
                            Text { text: "Gain"; width: 55px; vertical-alignment: center; }
                            Slider {
                                minimum: -60;
                                maximum: 60;
                                value <=> root.gain;
                                changed(g) => { root.gain-changed(root.selected, g); }
                            }
                            LineEdit {
                                width: 60px;
                                text <=> root.gain-txt;
                                input-type: InputType.decimal;
                                accepted(t) => {
                                    root.gain = Math.clamp(t.to-float(), -60, 60);
                                    root.gain-changed(root.selected, root.gain);
                                }
                            }
                            Text { text: root.gain.round() + " dB"; width: 50px; horizontal-alignment: right; vertical-alignment: center; }
                        }

                        HorizontalLayout {
                            spacing: 8px;
                            Button { text: "+ Add cue"; clicked => { root.add-cue(); } }
                            Button { text: "Delete cue"; clicked => { root.delete-cue(root.selected); } }
                            Button {
                                text: root.fade-cue ? "Run fade cue" : "Play cue";
                                clicked => {
                                    if (root.fade-cue) { root.fade(root.selected); }
                                    else { root.start(root.selected); }
                                }
                            }
                            Button { text: "End"; clicked => { root.end(root.selected); } }
                            Button { text: "Filters"; clicked => { root.show-filters(root.selected); } }
                        }

                        Rectangle {
                            height: 52px;
                            background: Palette.color-scheme == ColorScheme.dark ? #292929 : #f5f5f5;
                            clip: true;
                            ScrollView {
                                content-height: 48px;
                                VerticalLayout {
                                    padding: 4px;
                                    spacing: 2px;
                                    for warn in root.cues[root.selected].warnings : Text {
                                        text: "WARNING: " + warn;
                                        font-size: 10px;
                                    }
                                    Text {
                                        text: root.cues[root.selected].error;
                                        font-size: 10px;
                                        color: #dd4444;
                                        overflow: TextOverflow.elide;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    export component FilterWindow inherits Window {
        in property <string> cue-name;
        title: "Cue Filters: " + root.cue-name;
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
                        Param { idx: 2; label: "Low-pass"; lo: 20; hi: 18000; initial: 16000; unit: " Hz"; }
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
                        Param { idx: 13; label: "Low shelf freq"; lo: 20; hi: 18000; initial: 100; unit: " Hz"; }
                        Param { idx: 14; label: "Low shelf Q"; lo: 0.1; hi: 10; initial: 0.707; optional: false; }
                        Param { idx: 15; label: "Low shelf gain"; lo: -24; hi: 24; initial: 0; unit: " dB"; optional: false; }
                        Param { idx: 16; label: "Bell 1 freq"; lo: 20; hi: 18000; initial: 1000; unit: " Hz"; }
                        Param { idx: 17; label: "Bell 1 Q"; lo: 0.1; hi: 10; initial: 1; optional: false; }
                        Param { idx: 18; label: "Bell 1 gain"; lo: -24; hi: 24; initial: 0; unit: " dB"; optional: false; }
                        Param { idx: 22; label: "Bell 2 freq"; lo: 20; hi: 18000; initial: 3000; unit: " Hz"; }
                        Param { idx: 23; label: "Bell 2 Q"; lo: 0.1; hi: 10; initial: 1; optional: false; }
                        Param { idx: 24; label: "Bell 2 gain"; lo: -24; hi: 24; initial: 0; unit: " dB"; optional: false; }
                        Param { idx: 19; label: "High shelf freq"; lo: 20; hi: 18000; initial: 8000; unit: " Hz"; }
                        Param { idx: 20; label: "High shelf Q"; lo: 0.1; hi: 10; initial: 0.707; optional: false; }
                        Param { idx: 21; label: "High shelf gain"; lo: -24; hi: 24; initial: 0; unit: " dB"; optional: false; }
                    }
                }
            }
        }
    }
}

struct Cue {
    params: Arc<Mutex<Filters>>,
    session: Arc<Mutex<Option<Args>>>,
    filter_ui: FilterWindow,
}

impl Cue {
    fn new(name: &str) -> Result<Self, slint::PlatformError> {
        let params = Arc::new(Mutex::new(Filters::default()));
        let session: Arc<Mutex<Option<Args>>> = Arc::new(Mutex::new(None));
        let filter_ui = FilterWindow::new()?;
        filter_ui.set_cue_name(name.into());

        filter_ui.global::<Bridge>().on_edited({
            let params = params.clone();
            let session = session.clone();
            move |idx, on, val| {
                let updated = if let Ok(mut p) = params.lock() {
                    p.set(idx as usize, on, val);
                    let mut args = Args::default();
                    p.apply(&mut args);
                    Some(args)
                } else {
                    None
                };

                if let Some(updated) = updated
                    && let Ok(slot) = session.lock()
                    && let Some(args) = slot.as_ref()
                {
                    args.update_filters(&updated);
                }
            }
        });

        Ok(Self {
            params,
            session,
            filter_ui,
        })
    }
}

fn set_path(d: &mut CueData, path: SharedString) {
    let file = Path::new(path.as_str())
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    d.file = file.into();
    d.path = path;
}

fn add_cue(
    model: &VecModel<CueData>,
    cues: &RefCell<Vec<Cue>>,
) -> Result<(), slint::PlatformError> {
    let name = format!("Cue {}", cues.borrow().len() + 1);
    let cue = Cue::new(&name)?;
    cues.borrow_mut().push(cue);
    model.push(CueData {
        name: name.into(),
        queue: SharedString::new(),
        path: SharedString::new(),
        file: SharedString::new(),
        gain: 0.0,
        error: SharedString::new(),
        warnings: ModelRc::default(),
        delay: 0.0,
        fade_cue: false,
        fade_target: SharedString::new(),
        fade_duration: 1.0,
    });
    Ok(())
}

fn edit_row(model: &impl Model<Data = CueData>, i: usize, f: impl FnOnce(&mut CueData)) {
    if let Some(mut d) = model.row_data(i) {
        f(&mut d);
        model.set_row_data(i, d);
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let main_ui = MainWindow::new()?;
    let model = Rc::new(VecModel::<CueData>::default());
    let cues = Rc::new(RefCell::new(Vec::<Cue>::new()));

    add_cue(&model, &cues)?;
    main_ui.set_cues(ModelRc::from(model.clone()));

    main_ui.on_add_cue({
        let model = model.clone();
        let cues = cues.clone();
        move || {
            let _ = add_cue(&model, &cues);
        }
    });

    main_ui.on_name_edited({
        let model = model.clone();
        let cues = cues.clone();
        move |idx, name| {
            let idx = idx as usize;
            edit_row(&*model, idx, |d| d.name = name.clone());
            if let Some(cue) = cues.borrow().get(idx) {
                cue.filter_ui.set_cue_name(name);
            }
        }
    });

    main_ui.on_queue_edited({
        let model = model.clone();
        move |idx, queue| {
            edit_row(&*model, idx as usize, |d| d.queue = queue);
        }
    });

    main_ui.on_delay_edited({
        let model = model.clone();
        move |idx, delay| {
            edit_row(&*model, idx as usize, |d| d.delay = delay.max(0.0));
        }
    });

    main_ui.on_fade_cue_edited({
        let model = model.clone();
        move |idx, enabled| {
            edit_row(&*model, idx as usize, |d| d.fade_cue = enabled);
        }
    });

    main_ui.on_fade_target_edited({
        let model = model.clone();
        move |idx, target| {
            edit_row(&*model, idx as usize, |d| d.fade_target = target);
        }
    });

    main_ui.on_fade_duration_edited({
        let model = model.clone();
        move |idx, duration| {
            edit_row(&*model, idx as usize, |d| {
                d.fade_duration = duration.max(0.0)
            });
        }
    });

    main_ui.on_delete_cue({
        let model = model.clone();
        let cues = cues.clone();
        let weak = main_ui.as_weak();
        move |idx| {
            let idx = idx as usize;
            if model.row_count() <= 1 || idx >= model.row_count() {
                return;
            }

            let removed = cues.borrow_mut().remove(idx);
            if let Ok(mut slot) = removed.session.lock() {
                if let Some(args) = slot.take() {
                    args.stop();
                }
            }
            let _ = removed.filter_ui.hide();
            model.remove(idx);

            if let Some(ui) = weak.upgrade() {
                let selected = ui.get_selected() as usize;
                let next = if selected > idx {
                    selected - 1
                } else if selected == idx {
                    selected.min(model.row_count() - 1)
                } else {
                    selected
                };
                ui.set_selected(next as i32);
            }
        }
    });

    main_ui.on_path_edited({
        let model = model.clone();
        move |idx, path| {
            edit_row(&*model, idx as usize, |d| set_path(d, path));
        }
    });

    main_ui.on_gain_changed({
        let model = model.clone();
        let cues = cues.clone();
        move |idx, gain| {
            edit_row(&*model, idx as usize, |d| d.gain = gain);
            if let Some(t) = cues.borrow().get(idx as usize)
                && let Ok(slot) = t.session.lock()
                && let Some(args) = slot.as_ref()
            {
                args.set_gain_db(gain);
            }
        }
    });

    main_ui.on_start({
        let model = model.clone();
        let cues = cues.clone();
        let weak = main_ui.as_weak();
        move |idx| {
            let idx = idx as usize;
            let Some(row) = model.row_data(idx) else {
                return;
            };
            let (params, session) = {
                let cues = cues.borrow();
                let Some(t) = cues.get(idx) else {
                    return;
                };
                (t.params.clone(), t.session.clone())
            };

            let Ok(mut slot) = session.lock() else {
                return;
            };
            if let Some(old) = slot.take() {
                old.stop();
            }

            let mut args = Args {
                file: Some(row.path.to_string()),
                ..Default::default()
            };
            args.postamp = Some(row.gain);
            if let Ok(p) = params.lock() {
                p.apply(&mut args);
            }
            args.update_filters(&args);

            let thread_args = args.clone();
            let delay = Duration::from_secs_f32(row.delay.max(0.0));
            let weak = weak.clone();
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    edit_row(&ui.get_cues(), idx, |d| d.error = "".into());
                });
                if let Err(e) = run(&thread_args) {
                    let msg = format!("ERROR: {e:#}");
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        edit_row(&ui.get_cues(), idx, |d| d.error = msg.into());
                    });
                }
            });
            *slot = Some(args);
        }
    });

    main_ui.on_fade({
        let model = model.clone();
        let cues = cues.clone();
        move |idx| {
            let Some(cue) = model.row_data(idx as usize) else {
                return;
            };
            if cue.fade_target.is_empty() {
                return;
            }
            let target = (0..model.row_count()).find_map(|i| {
                model
                    .row_data(i)
                    .filter(|row| row.queue == cue.fade_target)
                    .map(|_| i)
            });
            let Some(target) = target else {
                return;
            };
            let session = {
                let all = cues.borrow();
                let Some(target_cue) = all.get(target) else {
                    return;
                };
                target_cue.session.clone()
            };
            let delay = Duration::from_secs_f32(cue.delay.max(0.0));
            let duration = Duration::from_secs_f32(cue.fade_duration.max(0.0));
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                if let Ok(slot) = session.lock()
                    && let Some(args) = slot.as_ref()
                {
                    args.fade_to(0.0, duration);
                }
            });
        }
    });

    main_ui.on_end({
        let cues = cues.clone();
        move |idx| {
            let session = {
                let cues = cues.borrow();
                let Some(t) = cues.get(idx as usize) else {
                    return;
                };
                t.session.clone()
            };
            let Ok(mut slot) = session.lock() else {
                return;
            };
            if let Some(a) = slot.take() {
                std::thread::spawn(move || {
                    a.fade_to(0.0, Duration::from_secs(1));
                    a.stop();
                });
            }
        }
    });

    main_ui.on_show_filters({
        let cues = cues.clone();
        move |idx| {
            if let Some(t) = cues.borrow().get(idx as usize) {
                let _ = t.filter_ui.show();
            }
        }
    });

    main_ui.on_browse({
        let weak = main_ui.as_weak();
        move |idx| {
            let weak = weak.clone();
            let idx = idx as usize;
            std::thread::spawn(move || {
                let picked = rfd::FileDialog::new()
                    .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "m4a"])
                    .add_filter("All files", &["*"])
                    .pick_file();

                if let Some(p) = picked {
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        edit_row(&ui.get_cues(), idx, |d| {
                            set_path(d, p.to_string_lossy().to_string().into());
                        });
                    });
                }
            });
        }
    });

    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), {
        let cues = cues.clone();
        let weak = main_ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };

            let all_params: Vec<_> = cues.borrow().iter().map(|t| t.params.clone()).collect();

            for (i, params) in all_params.iter().enumerate() {
                let mut args = Args::default();
                if let Ok(p) = params.lock() {
                    p.apply(&mut args);
                }

                let list: Vec<SharedString> = warn_extreme_settings(&args)
                    .iter()
                    .map(|w| SharedString::from(w.to_string()))
                    .collect();

                edit_row(&ui.get_cues(), i, |d| {
                    d.warnings = ModelRc::new(VecModel::from(list));
                });
            }
        }
    });

    main_ui.window().on_close_requested({
        let cues = cues.clone();
        move || {
            for t in cues.borrow().iter() {
                let _ = t.filter_ui.hide();
            }
            slint::CloseRequestResponse::HideWindow
        }
    });

    main_ui.run()
}
