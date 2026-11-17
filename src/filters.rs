use super::sys;

macro_rules! params {
    ($($name:ident => ($on:expr, $val:expr)),* $(,)?) => {
        #[derive(Clone, Copy)]
        enum P {
            $($name),*
        }

        const DEFAULTS: &[(bool, f32)] = &[$(($on, $val)),*];
    };
}

params! {
    Preamp => (false, 0.0),
    Hpf => (false, 80.0),
    Lpf => (false, 16000.0),
    Ceiling => (true, -12.0),
    GateThresh => (false, -40.0),
    GateDepth => (true, -40.0),
    GateAtt => (true, 5.0),
    GateRel => (true, 100.0),
    CompThresh => (false, -18.0),
    CompRatio => (true, 4.0),
    CompAtt => (true, 10.0),
    CompRel => (true, 100.0),
    CompMakeup => (true, 0.0),
    LowHz => (false, 100.0),
    LowQ => (true, 0.707),
    LowGain => (true, 0.0),
    BellHz => (false, 1000.0),
    BellQ => (true, 1.0),
    BellGain => (true, 0.0),
    HighHz => (false, 8000.0),
    HighQ => (true, 0.707),
    HighGain => (true, 0.0),
    Bell2Hz => (false, 3000.0),
    Bell2Q => (true, 1.0),
    Bell2Gain => (true, 0.0),
}

pub struct Filters {
    values: Vec<(bool, f32)>,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            values: DEFAULTS.to_vec(),
        }
    }
}

impl Filters {
    pub fn set(&mut self, idx: usize, on: bool, val: f32) {
        if let Some(slot) = self.values.get_mut(idx) {
            *slot = (on, val);
        }
    }

    fn on(&self, p: P) -> Option<f32> {
        let (on, val) = self.values[p as usize];
        on.then_some(val)
    }

    fn val(&self, p: P) -> f32 {
        self.values[p as usize].1
    }

    fn band(&self, kind: sys::EqKind, hz: P, q: P, gain: P) -> Option<sys::EqBand> {
        self.on(hz).map(|hz| sys::EqBand {
            kind,
            hz,
            q: self.val(q),
            gain: self.val(gain),
        })
    }

    pub fn apply(&self, args: &mut sys::Args) {
        args.preamp = self.on(P::Preamp);
        args.hpf = self.on(P::Hpf);
        args.lpf = self.on(P::Lpf);
        args.ceiling = self.val(P::Ceiling);

        args.gate = self.on(P::GateThresh).map(|thresh| sys::NoiseGate {
            thresh,
            depth: self.val(P::GateDepth),
            att: self.val(P::GateAtt),
            rel: self.val(P::GateRel),
        });

        args.comp = self.on(P::CompThresh).map(|thresh| sys::Compressor {
            thresh,
            ratio: self.val(P::CompRatio).max(1.0),
            att: self.val(P::CompAtt),
            rel: self.val(P::CompRel),
            makeup: self.val(P::CompMakeup),
        });

        args.eq = [
            self.band(sys::EqKind::LowShelf, P::LowHz, P::LowQ, P::LowGain),
            self.band(sys::EqKind::Bell, P::BellHz, P::BellQ, P::BellGain),
            self.band(sys::EqKind::Bell, P::Bell2Hz, P::Bell2Q, P::Bell2Gain),
            self.band(sys::EqKind::HighShelf, P::HighHz, P::HighQ, P::HighGain),
        ]
        .into_iter()
        .flatten()
        .collect();
    }
}
