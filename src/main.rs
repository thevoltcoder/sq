use anyhow::{Context, bail};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use sq::{
    filters::Filters,
    sys::{Args, run, warn_extreme_settings},
};
use std::{
    cell::RefCell,
    path::Path,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use symphonia::core::{
    formats::{FormatOptions, TrackType, probe::Hint},
    io::MediaSourceStream,
    meta::MetadataOptions,
};

slint::slint! {
    #[edition = "1.19"]

    import { Button, Slider, CheckBox, TabWidget, LineEdit, ScrollView, Palette } from "std-widgets.slint";

    export global Bridge {
        callback edited(int, bool, float);
    }

    export global Fmt {
        pure callback time(float) -> string;
    }

    export struct queueData {
        name: string,
        queue: string,
        path: string,
        file: string,
        gain: float,
        error: string,
        warnings: [string],
        delay: float,
        fade_queue: bool,
        fade_target: string,
        fade_duration: float,
        length: float,
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

    component queueRow inherits Rectangle {
        in property <queueData> data;
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

            Text {
                text: root.data.queue == "" ? "-" : root.data.queue;
                width: 65px;
                font-size: 11px;
                font-weight: 700;
                vertical-alignment: center;
            }
            Rectangle {
                width: 1px;
                background: Palette.color-scheme == ColorScheme.dark ? #555555 : #bbbbbb;
            }
            Text {
                text: root.data.name;
                horizontal-stretch: 1;
                font-size: 12px;
                font-weight: root.selected ? 700 : 400;
                overflow: TextOverflow.elide;
                vertical-alignment: center;
            }
            Rectangle {
                width: 1px;
                background: Palette.color-scheme == ColorScheme.dark ? #555555 : #bbbbbb;
            }
            Text {
                text: root.data.fade_queue ? "fade queue" : root.data.file;
                width: 140px;
                font-size: 10px;
                overflow: TextOverflow.elide;
                vertical-alignment: center;
            }
            Rectangle {
                width: 1px;
                background: Palette.color-scheme == ColorScheme.dark ? #555555 : #bbbbbb;
            }
            Text {
                text: Fmt.time(root.data.fade_queue ? root.data.fade_duration : root.data.length);
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

        in property <[queueData]> queues;
        in-out property <int> selected: 0;

        callback start(int);
        callback end-all();
        callback fade(int);
        callback browse(int);
        callback path-edited(int, string);
        callback gain-changed(int, float);
        callback show-filters(int);
        callback add-queue();
        callback delete-queue(int);
        callback name-edited(int, string);
        callback queue-edited(int, string);
        callback delay-edited(int, float);
        callback fade-queue-edited(int, bool);
        callback fade-target-edited(int, string);
        callback fade-duration-edited(int, float);

        property <string> model-path: root.queues[root.selected].path;
        property <string> model-name: root.queues[root.selected].name;
        property <string> model-queue: root.queues[root.selected].queue;
        property <float> model-gain: root.queues[root.selected].gain;
        property <float> model-delay: root.queues[root.selected].delay;
        property <bool> model-fade-queue: root.queues[root.selected].fade_queue;
        property <string> model-fade-target: root.queues[root.selected].fade_target;
        property <float> model-fade-duration: root.queues[root.selected].fade_duration;
        property <string> path: "";
        property <string> queue-name: "";
        property <string> queue: "";
        property <string> fade-target: "";
        property <string> delay-txt: "0";
        property <string> fade-duration-txt: "1";
        property <float> gain: 0.0;
        property <string> gain-txt: "0";
        property <bool> fade-queue: false;

        changed model-path => { root.path = root.model-path; }
        changed model-name => { root.queue-name = root.model-name; }
        changed model-queue => { root.queue = root.model-queue; }
        changed model-gain => { root.gain = root.model-gain; }
        changed model-delay => { root.delay-txt = "" + root.model-delay; }
        changed model-fade-queue => { root.fade-queue = root.model-fade-queue; }
        changed model-fade-target => { root.fade-target = root.model-fade-target; }
        changed model-fade-duration => { root.fade-duration-txt = "" + root.model-fade-duration; }
        changed gain => { root.gain-txt = "" + Math.round(root.gain); }

        init => { fs.focus(); }

        fs := FocusScope {
            key-pressed(event) => {
                if (event.text == Key.Escape) {
                    root.end-all();
                    return accept;
                }
                if (event.text == " ") {
                    if (root.fade-queue) { root.fade(root.selected); }
                    else { root.start(root.selected); }
                    if (root.selected + 1 < root.queues.length) {
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
                    height: (root.height) - 330px;
                    content-height: root.queues.length * 34px;
                    VerticalLayout {
                        spacing: 2px;
                        alignment: LayoutAlignment.start;
                        for t[i] in root.queues : queueRow {
                            width: (root.width) - 52px;
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
                                text <=> root.queue-name;
                                placeholder-text: "queue name";
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
                                accepted(t) => { root.path-edited(root.selected, t); }
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
                                text: "Fade queue";
                                checked: root.fade-queue;
                                toggled => {
                                    root.fade-queue = self.checked;
                                    root.fade-queue-edited(root.selected, self.checked);
                                }
                            }
                            LineEdit {
                                width: 100px;
                                text <=> root.fade-target;
                                enabled: root.fade-queue;
                                placeholder-text: "Target queue ID";
                                edited(t) => { root.fade-target-edited(root.selected, t); }
                            }
                            Text { text: "Fade duration (s)"; vertical-alignment: center; }
                            LineEdit {
                                width: 75px;
                                text <=> root.fade-duration-txt;
                                enabled: root.fade-queue;
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
                            Button { text: "+ Add queue"; clicked => { root.add-queue(); } }
                            Button { text: "Delete queue"; clicked => { root.delete-queue(root.selected); } }
                            Button {
                                text: root.fade-queue ? "Run fade queue" : "Play queue";
                                clicked => {
                                    if (root.fade-queue) { root.fade(root.selected); }
                                    else { root.start(root.selected); }
                                }
                            }
                            Button { text: "End"; clicked => { root.end-all(); } }
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
                                    for warn in root.queues[root.selected].warnings : Text {
                                        text: warn;
                                        font-size: 10px;
                                    }
                                    Text {
                                        text: root.queues[root.selected].error;
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
        in property <string> queue-name;
        title: "queue Filters: " + root.queue-name;
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
                        Param { idx: 24; label: "Bell 2 gain"; lo: 0.1; hi: 10; initial: 1; optional: false; }
                        Param { idx: 19; label: "High shelf freq"; lo: 20; hi: 18000; initial: 8000; unit: " Hz"; }
                        Param { idx: 20; label: "High shelf Q"; lo: 0.1; hi: 10; initial: 0.707; optional: false; }
                        Param { idx: 21; label: "High shelf gain"; lo: -24; hi: 24; initial: 0; unit: " dB"; optional: false; }
                    }
                }
            }
        }
    }
}

struct Queue {
    params: Arc<Mutex<Filters>>,
    session: Arc<Mutex<Option<Args>>>,
    filter_ui: FilterWindow,
}

impl Queue {
    fn new(name: &str) -> Result<Self, slint::PlatformError> {
        let params = Arc::new(Mutex::new(Filters::default()));
        let session: Arc<Mutex<Option<Args>>> = Arc::new(Mutex::new(None));
        let filter_ui = FilterWindow::new()?;
        filter_ui.set_queue_name(name.into());

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

fn audio_secs(path: &str) -> anyhow::Result<f32> {
    let Ok(file) = std::fs::File::open(path) else {
        bail!("failed to open file");
    };
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let Ok(format) = symphonia::default::get_probe().probe(
        &Hint::new(),
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) else {
        bail!("symphonia failed to probe");
    };
    let Some(track) = format.default_track(TrackType::Audio) else {
        bail!("failed to get track");
    };
    let (Some(duration), Some(time_base)) = (track.duration, track.time_base) else {
        bail!("invalid duration and/or time base");
    };
    Ok(time_base
        .calc_duration(duration)
        .context("failed to calculate duration")?
        .as_secs_f64() as f32)
}

fn set_path(d: &mut queueData, path: SharedString) {
    let file = Path::new(path.as_str())
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();

    d.file = file.into();
    d.path = path;
    d.length = 0.0;
}

fn load_audio_length(weak: slint::Weak<MainWindow>, idx: usize, path: String) {
    std::thread::spawn(move || {
        let length = audio_secs(&path).unwrap_or(0.0);

        let _ = weak.upgrade_in_event_loop(move |ui| {
            edit_row(&ui.get_queues(), idx, |d| {
                if d.path == path.as_str() {
                    d.length = length;
                }
            });
        });
    });
}

fn add_queue(
    model: &VecModel<queueData>,
    queues: &RefCell<Vec<Queue>>,
) -> Result<(), slint::PlatformError> {
    let name = format!("queue {}", queues.borrow().len() + 1);
    let queue = Queue::new(&name)?;
    queues.borrow_mut().push(queue);
    model.push(queueData {
        name: name.into(),
        queue: SharedString::new(),
        path: SharedString::new(),
        file: SharedString::new(),
        gain: 0.0,
        error: SharedString::new(),
        warnings: ModelRc::default(),
        delay: 0.0,
        fade_queue: false,
        fade_target: SharedString::new(),
        fade_duration: 1.0,
        length: 0.0,
    });
    Ok(())
}

fn edit_row(model: &impl Model<Data = queueData>, i: usize, f: impl FnOnce(&mut queueData)) {
    if let Some(mut d) = model.row_data(i) {
        f(&mut d);
        model.set_row_data(i, d);
    }
}

fn main() -> Result<(), slint::PlatformError> {
    unsafe {
        std::env::set_var("SLINT_BACKEND", "winit-software");
    }

    let main_ui = MainWindow::new()?;
    let model = Rc::new(VecModel::<queueData>::default());
    let queues = Rc::new(RefCell::new(Vec::<Queue>::new()));
    let playback_lock = Arc::new(Mutex::new(()));
    let playback_generation = Arc::new(AtomicU64::new(0));
    let fading: Arc<Mutex<Vec<(u64, Args)>>> = Arc::new(Mutex::new(Vec::new()));
    let fade_id = Arc::new(AtomicU64::new(0));

    main_ui.global::<Fmt>().on_time(|s| {
        let t = s.max(0.0) as u64;
        format!("{:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60).into()
    });

    add_queue(&model, &queues)?;
    main_ui.set_queues(ModelRc::from(model.clone()));

    main_ui.on_add_queue({
        let model = model.clone();
        let queues = queues.clone();
        move || {
            let _ = add_queue(&model, &queues);
        }
    });

    main_ui.on_name_edited({
        let model = model.clone();
        let queues = queues.clone();
        move |idx, name| {
            let idx = idx as usize;
            edit_row(&*model, idx, |d| d.name = name.clone());
            if let Some(queue) = queues.borrow().get(idx) {
                queue.filter_ui.set_queue_name(name);
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

    main_ui.on_fade_queue_edited({
        let model = model.clone();
        move |idx, enabled| {
            edit_row(&*model, idx as usize, |d| d.fade_queue = enabled);
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

    main_ui.on_delete_queue({
        let model = model.clone();
        let queues = queues.clone();
        let weak = main_ui.as_weak();
        let playback_generation = playback_generation.clone();
        move |idx| {
            let idx = idx as usize;
            if model.row_count() <= 1 || idx >= model.row_count() {
                return;
            }

            playback_generation.fetch_add(1, Ordering::SeqCst);

            let removed = queues.borrow_mut().remove(idx);
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
        let weak = main_ui.as_weak();
        move |idx, path| {
            let idx = idx as usize;
            edit_row(&*model, idx, |d| set_path(d, path.clone()));
            load_audio_length(weak.clone(), idx, path.to_string());
        }
    });

    main_ui.on_gain_changed({
        let model = model.clone();
        let queues = queues.clone();
        move |idx, gain| {
            edit_row(&*model, idx as usize, |d| d.gain = gain);
            if let Some(t) = queues.borrow().get(idx as usize)
                && let Ok(slot) = t.session.lock()
                && let Some(args) = slot.as_ref()
            {
                args.set_gain_db(gain);
            }
        }
    });

    main_ui.on_start({
        let model = model.clone();
        let queues = queues.clone();
        let weak = main_ui.as_weak();
        let playback_lock = playback_lock.clone();
        let playback_generation = playback_generation.clone();
        let fading = fading.clone();

        move |idx| {
            let idx = idx as usize;
            let Some(row) = model.row_data(idx) else {
                return;
            };

            let (params, session) = {
                let all = queues.borrow();
                let Some(queue) = all.get(idx) else {
                    return;
                };
                (queue.params.clone(), queue.session.clone())
            };

            let generation = playback_generation.fetch_add(1, Ordering::SeqCst) + 1;

            let sessions: Vec<_> = queues
                .borrow()
                .iter()
                .map(|queue| queue.session.clone())
                .collect();

            for existing in sessions {
                if let Ok(mut slot) = existing.lock() {
                    if let Some(args) = slot.take() {
                        args.stop();
                    }
                }
            }

            let purged: Vec<Args> = fading
                .lock()
                .map(|mut f| f.drain(..).map(|(_, a)| a).collect())
                .unwrap_or_default();
            for args in purged {
                args.stop();
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

            if let Ok(mut slot) = session.lock() {
                *slot = Some(args);
            }

            let weak = weak.clone();
            let playback_generation = playback_generation.clone();
            let playback_lock = playback_lock.clone();

            std::thread::spawn(move || {
                std::thread::sleep(delay);

                let _guard = match playback_lock.lock() {
                    Ok(guard) => guard,
                    Err(_) => return,
                };

                if playback_generation.load(Ordering::SeqCst) != generation {
                    return;
                }

                let _ = weak.upgrade_in_event_loop(move |ui| {
                    edit_row(&ui.get_queues(), idx, |d| d.error = "".into());
                });

                if let Err(e) = run(&thread_args) {
                    let msg = format!("ERROR: {e:#}");
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        edit_row(&ui.get_queues(), idx, |d| d.error = msg.into());
                    });
                }
            });
        }
    });

    main_ui.on_fade({
        let model = model.clone();
        let queues = queues.clone();
        move |idx| {
            let Some(queue) = model.row_data(idx as usize) else {
                return;
            };

            if queue.fade_target.is_empty() {
                return;
            }

            let target = (0..model.row_count()).find_map(|i| {
                model
                    .row_data(i)
                    .filter(|row| row.queue == queue.fade_target)
                    .map(|_| i)
            });

            let Some(target) = target else {
                return;
            };

            let session = {
                let all = queues.borrow();
                let Some(target_queue) = all.get(target) else {
                    return;
                };
                target_queue.session.clone()
            };

            let delay = Duration::from_secs_f32(queue.delay.max(0.0));
            let duration = Duration::from_secs_f32(queue.fade_duration.max(0.0));

            std::thread::spawn(move || {
                std::thread::sleep(delay);
                let args = session.lock().ok().and_then(|slot| slot.clone());
                if let Some(args) = args {
                    args.fade_to(0.0, duration);
                }
            });
        }
    });

    main_ui.on_end_all({
        let queues = queues.clone();
        let playback_generation = playback_generation.clone();
        let fading = fading.clone();
        let fade_id = fade_id.clone();

        move || {
            playback_generation.fetch_add(1, Ordering::SeqCst);

            let sessions: Vec<_> = queues
                .borrow()
                .iter()
                .map(|queue| queue.session.clone())
                .collect();

            for session in sessions {
                let args = session.lock().ok().and_then(|mut slot| slot.take());

                if let Some(args) = args {
                    let id = fade_id.fetch_add(1, Ordering::SeqCst);
                    if let Ok(mut f) = fading.lock() {
                        f.push((id, args.clone()));
                    }

                    let fading = fading.clone();
                    std::thread::spawn(move || {
                        args.fade_to(0.0, Duration::from_secs(1));
                        args.stop();
                        if let Ok(mut f) = fading.lock() {
                            f.retain(|(i, _)| *i != id);
                        }
                    });
                }
            }
        }
    });

    main_ui.on_show_filters({
        let queues = queues.clone();
        move |idx| {
            if let Some(t) = queues.borrow().get(idx as usize) {
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
                    let path = p.to_string_lossy().to_string();

                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        edit_row(&ui.get_queues(), idx, |d| {
                            set_path(d, path.clone().into());
                        });

                        load_audio_length(ui.as_weak(), idx, path);
                    });
                }
            });
        }
    });

    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), {
        let queues = queues.clone();
        let weak = main_ui.as_weak();

        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };

            let model = ui.get_queues();
            let all_params: Vec<_> = queues.borrow().iter().map(|t| t.params.clone()).collect();

            for (i, params) in all_params.iter().enumerate() {
                let mut args = Args::default();

                if let Ok(p) = params.lock() {
                    p.apply(&mut args);
                } else {
                    continue;
                }

                let list: Vec<SharedString> = warn_extreme_settings(&args)
                    .iter()
                    .map(|w| SharedString::from(w.to_string()))
                    .collect();

                let unchanged = model.row_data(i).is_some_and(|d| {
                    d.warnings.row_count() == list.len()
                        && list
                            .iter()
                            .enumerate()
                            .all(|(j, warning)| d.warnings.row_data(j).as_ref() == Some(warning))
                });

                if !unchanged {
                    edit_row(&model, i, |d| {
                        d.warnings = ModelRc::new(VecModel::from(list));
                    });
                }
            }
        }
    });

    main_ui.window().on_close_requested({
        let queues = queues.clone();
        move || {
            for t in queues.borrow().iter() {
                let _ = t.filter_ui.hide();
            }
            slint::CloseRequestResponse::HideWindow
        }
    });

    main_ui.run()
}
