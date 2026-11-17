use anyhow::{Context, bail};
use cpal::{
    Device, SampleFormat, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use fundsp::prelude32::*;
use rubato::{Fft, FixedSync, Resampler as _, audioadapter_buffers::direct::InterleavedSlice};
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    time::Duration,
};
use symphonia::core::{
    codecs::audio::AudioDecoderOptions,
    formats::{FormatOptions, TrackType, probe::Hint},
    io::MediaSourceStream,
    meta::MetadataOptions,
};

const LOOKAHEAD_SECS: f32 = 0.005;
const RELEASE_SECS: f32 = 0.1;
const WINDOW_DIV: usize = 20;
const STRIKE_LIMIT: u32 = 4;

const PEAK_ABORT: f32 = 4.0;
const DC_BLOCK_HZ: f32 = 20.0;
const QUEUE_DEPTH: usize = 32;

#[derive(Clone, Debug)]
pub struct Args {
    pub file: Option<String>,
    pub preamp: Option<f32>,
    pub postamp: Option<f32>,
    pub hpf: Option<f32>,
    pub lpf: Option<f32>,
    pub gate: Option<NoiseGate>,
    pub eq: Vec<EqBand>,
    pub comp: Option<Compressor>,
    pub ceiling: f32,
    pub volume: Arc<AtomicU32>,
    pub stop_queued: Arc<AtomicBool>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            file: None,
            preamp: None,
            postamp: None,
            hpf: None,
            lpf: None,
            gate: None,
            eq: Vec::new(),
            comp: None,
            ceiling: -12.0,
            volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            stop_queued: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Args {
    pub fn set_volume(&self, gain: f32) {
        self.volume.store(gain.to_bits(), Ordering::Relaxed);
    }

    pub fn set_volume_db(&self, db: f32) {
        self.set_volume(10f32.powf(db / 20.0));
    }

    pub fn fade_to(&self, target: f32, dur: Duration) {
        const FLOOR: f32 = 1e-4;
        let start = f32::from_bits(self.volume.load(Ordering::Relaxed));
        let start_db = 20.0 * start.max(FLOOR).log10();
        let target_db = 20.0 * target.max(FLOOR).log10();
        let steps = (dur.as_secs_f32() * 100.0).max(1.0) as u32;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let db = start_db + (target_db - start_db) * t;
            self.set_volume(10f32.powf(db / 20.0));
            std::thread::sleep(Duration::from_millis(10));
        }
        self.set_volume(target);
    }

    pub fn stop(&self) {
        self.stop_queued.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NoiseGate {
    pub thresh: f32,
    pub depth: f32,
    pub att: f32,
    pub rel: f32,
}

#[derive(Clone, Copy, Debug)]
pub enum EqKind {
    Bell,
    LowShelf,
    HighShelf,
}

#[derive(Clone, Copy, Debug)]
pub struct EqBand {
    pub kind: EqKind,
    pub hz: f32,
    pub q: f32,
    pub gain: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Compressor {
    pub thresh: f32,
    pub ratio: f32,
    pub att: f32,
    pub rel: f32,
    pub makeup: f32,
}

fn check_nyquist(args: &Args, rate: u32) -> anyhow::Result<()> {
    let limit = rate as f32 * 0.45;
    let mut freqs: Vec<(&str, f32)> = Vec::new();
    if let Some(v) = args.hpf {
        freqs.push(("hpf", v));
    }
    if let Some(v) = args.lpf {
        freqs.push(("lpf", v));
    }
    for b in &args.eq {
        freqs.push(("eq frequency", b.hz));
    }
    for (name, hz) in freqs {
        if hz >= limit {
            bail!("{name} {hz} Hz is too close to Nyquist for {rate} Hz audio (max {limit:.0} Hz)");
        }
    }
    Ok(())
}

fn check_channels(channels: usize) -> anyhow::Result<()> {
    if channels == 0 {
        bail!("audio has no channels");
    }
    if channels > 2 {
        bail!(
            "{channels}-channel audio is refused: this build has no explicit multichannel speaker mapping. Use mono/stereo files until channel routing is implemented and verified."
        );
    }
    Ok(())
}

fn build_chain(args: &Args, rate: f64) -> Vec<Box<dyn AudioUnit>> {
    let mut chain: Vec<Box<dyn AudioUnit>> = Vec::new();

    chain.push(Box::new(highpass_hz(DC_BLOCK_HZ, 0.707)));

    if let Some(db) = args.preamp {
        chain.push(Box::new((pass() * db_amp(db)) >> shape(Tanh(1.0))));
    }

    if let Some(hz) = args.hpf {
        chain.push(Box::new(highpass_hz(hz, 0.707)));
    }

    if let Some(gate) = args.gate {
        let thresh_amp = db_amp(gate.thresh);
        let floor_gain = db_amp(gate.depth);
        let attack_s = gate.att / 1000.0;
        let release_s = gate.rel / 1000.0;

        chain.push(Box::new(
            split::<U2>()
                >> (pass()
                    | (map(|frame: &Frame<f32, U1>| frame[0].abs())
                        >> afollow(attack_s, release_s)))
                >> (pass()
                    | map(move |frame: &Frame<f32, U1>| {
                        if frame[0] >= thresh_amp {
                            1.0f32
                        } else {
                            floor_gain
                        }
                    }))
                >> (pass() | follow(0.005))
                >> map(|frame: &Frame<f32, U2>| frame[0] * frame[1]),
        ));
    }

    for b in &args.eq {
        let gain = 10f32.powf(b.gain / 20.0);
        let unit: Box<dyn AudioUnit> = match b.kind {
            EqKind::Bell => Box::new(bell_hz(b.hz, b.q, gain)),
            EqKind::LowShelf => Box::new(lowshelf_hz(b.hz, b.q, gain)),
            EqKind::HighShelf => Box::new(highshelf_hz(b.hz, b.q, gain)),
        };
        chain.push(unit);
    }

    if let Some(c) = args.comp {
        let slope = 1.0 - 1.0 / c.ratio;
        let makeup = 10f32.powf(c.makeup / 20.0);
        chain.push(Box::new(
            (pass()
                ^ (map(|i: &Frame<f32, U1>| i[0].abs())
                    >> afollow(c.att * 0.001, c.rel * 0.001)
                    >> map(move |i: &Frame<f32, U1>| {
                        let over = 20.0 * i[0].max(1e-9).log10() - c.thresh;
                        if over > 0.0 {
                            10f32.powf(-over * slope / 20.0) * makeup
                        } else {
                            makeup
                        }
                    })))
                >> map(|i: &Frame<f32, U2>| i[0] * i[1]),
        ));
    }

    if let Some(hz) = args.lpf {
        chain.push(Box::new(lowpass_hz(hz, 0.707)));
    }

    if let Some(db) = args.postamp {
        chain.push(Box::new((pass() * db_amp(db)) >> shape(Tanh(1.0))));
    }

    chain.push(Box::new(highpass_hz(DC_BLOCK_HZ, 0.707)));

    for stage in &mut chain {
        stage.set_sample_rate(rate);
    }
    chain
}

struct ChannelChain {
    stages: Vec<Box<dyn AudioUnit>>,
    a: BufferArray<U1>,
    b: BufferArray<U1>,
}

impl ChannelChain {
    fn new(args: &Args, rate: f64) -> Self {
        Self {
            stages: build_chain(args, rate),
            a: BufferArray::<U1>::new(),
            b: BufferArray::<U1>::new(),
        }
    }
}

fn apply_filters(samples: &mut [f32], channels: usize, chains: &mut [ChannelChain]) {
    if chains.iter().all(|c| c.stages.is_empty()) {
        return;
    }
    let frames = samples.len() / channels;
    let mut start = 0;
    while start < frames {
        let n = std::cmp::Ord::min(MAX_BUFFER_SIZE, frames - start);
        for (ch, chain) in chains.iter_mut().enumerate() {
            for i in 0..n {
                chain.a.set_f32(0, i, samples[(start + i) * channels + ch]);
            }
            for stage in chain.stages.iter_mut() {
                stage.process(n, &chain.a.buffer_ref(), &mut chain.b.buffer_mut());
                std::mem::swap(&mut chain.a, &mut chain.b);
            }
            for i in 0..n {
                samples[(start + i) * channels + ch] = chain.a.at_f32(0, i);
            }
        }
        start += n;
    }
}

struct Limiter {
    channels: usize,
    lookahead: usize,
    ceiling: f32,
    release_coef: f32,
    delay: Vec<f32>,
    peaks: Vec<f32>,
    delay_idx: usize,
    peak_idx: usize,
    gain: f32,
}

impl Limiter {
    fn new(rate: u32, channels: usize, ceiling: f32) -> Self {
        let lookahead = std::cmp::Ord::max((rate as f32 * LOOKAHEAD_SECS) as usize, 1);

        Self {
            channels,
            lookahead,
            ceiling,
            release_coef: 1.0 - (-1.0 / (rate as f32 * RELEASE_SECS)).exp(),
            delay: vec![0.0; lookahead * channels],

            peaks: vec![0.0; lookahead + 1],

            delay_idx: 0,
            peak_idx: 0,
            gain: 1.0,
        }
    }

    fn process(&mut self, samples: &mut [f32]) {
        for frame in samples.chunks_mut(self.channels) {
            let input_peak = frame
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));

            self.peaks[self.peak_idx] = input_peak;
            self.peak_idx = (self.peak_idx + 1) % self.peaks.len();

            let window_peak = self.peaks.iter().copied().fold(0.0_f32, f32::max);

            let target_gain = if window_peak > self.ceiling {
                self.ceiling / window_peak
            } else {
                1.0
            };

            self.gain = if target_gain < self.gain {
                target_gain
            } else {
                self.gain + (target_gain - self.gain) * self.release_coef
            };

            let delay_base = self.delay_idx * self.channels;
            for (channel, sample) in frame.iter_mut().enumerate() {
                let delayed_sample = self.delay[delay_base + channel];
                self.delay[delay_base + channel] = *sample;
                *sample = delayed_sample * self.gain;
            }

            self.delay_idx = (self.delay_idx + 1) % self.lookahead;
        }
    }

    fn drain(&mut self) -> Vec<f32> {
        let mut tail = vec![0.0_f32; self.lookahead * self.channels];
        self.process(&mut tail);
        tail
    }
}

fn mean_square(buf: &[f32]) -> f64 {
    if buf.is_empty() {
        return 0.0;
    }
    buf.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / buf.len() as f64
}

fn to_db(ms: f64) -> f64 {
    10.0 * ms.max(1e-12).log10()
}

struct ResampleStage {
    inner: Fft<f32>,
    channels: usize,
    pending: Vec<f32>,
    out: Vec<f32>,
}

impl ResampleStage {
    fn new(from: u32, to: u32, channels: usize) -> anyhow::Result<Self> {
        let inner = Fft::<f32>::new(from as usize, to as usize, 1024, channels, FixedSync::Input)?;
        let out = vec![0.0; inner.output_frames_max() * channels];
        Ok(Self {
            inner,
            channels,
            pending: Vec::new(),
            out,
        })
    }

    fn run(&mut self, interleaved: &[f32], flush: bool) -> anyhow::Result<Vec<f32>> {
        self.pending.extend_from_slice(interleaved);
        let capacity = self.out.len() / self.channels;
        let mut result = Vec::new();
        loop {
            let need = self.inner.input_frames_next() * self.channels;
            if self.pending.len() < need {
                if !flush || self.pending.is_empty() {
                    break;
                }
                self.pending.resize(need, 0.0);
            }
            let input =
                InterleavedSlice::new(&self.pending[..need], self.channels, need / self.channels)?;
            let mut output = InterleavedSlice::new_mut(&mut self.out, self.channels, capacity)?;
            let (_, written) = self.inner.process_into_buffer(&input, &mut output, None)?;
            result.extend_from_slice(&self.out[..written * self.channels]);
            self.pending.drain(..need);
        }
        Ok(result)
    }
}

struct Pipeline {
    channels: usize,
    chains: Vec<ChannelChain>,
    limiter: Limiter,
    ceiling: f32,
    strikes: u32,
    tx: SyncSender<Vec<f32>>,
}

impl Pipeline {
    fn new(
        args: &Args,
        rate: u32,
        channels: usize,
        tx: SyncSender<Vec<f32>>,
    ) -> anyhow::Result<Self> {
        check_nyquist(args, rate)?;
        let ceiling = 10f32.powf(args.ceiling / 20.0);
        Ok(Self {
            channels,
            chains: (0..channels)
                .map(|_| ChannelChain::new(args, rate as f64))
                .collect(),
            limiter: Limiter::new(rate, channels, ceiling),
            ceiling,
            strikes: 0,
            tx,
        })
    }

    fn push(&mut self, mut chunk: Vec<f32>, last: bool) -> anyhow::Result<bool> {
        let usable = chunk.len() / self.channels * self.channels;
        chunk.truncate(usable);

        if !chunk.is_empty() {
            for s in chunk.iter_mut() {
                if !s.is_finite() {
                    *s = 0.0;
                }
            }

            let in_ms = mean_square(&chunk);
            apply_filters(&mut chunk, self.channels, &mut self.chains);

            if chunk.iter().any(|s| !s.is_finite() || s.abs() > PEAK_ABORT) {
                bail!("filter output blew up (non-finite or peak above {PEAK_ABORT}), aborting");
            }

            let out_ms = mean_square(&chunk);
            let hot = out_ms > 1.0 || (out_ms > 0.25 && out_ms > in_ms * 16.0);
            self.strikes = if hot { self.strikes + 1 } else { 0 };
            if self.strikes >= STRIKE_LIMIT {
                bail!(
                    "output too loud: {:.1} dBFS RMS vs {:.1} dBFS input, aborting",
                    to_db(out_ms),
                    to_db(in_ms)
                );
            }

            for s in chunk.iter_mut() {
                if !s.is_finite() {
                    *s = 0.0;
                }
            }

            self.limiter.process(&mut chunk);
        }

        if last {
            let tail = self.limiter.drain();
            chunk.extend_from_slice(&tail);
        }

        for s in chunk.iter_mut() {
            if !s.is_finite() {
                *s = 0.0;
            }
            *s = s.clamp(-self.ceiling, self.ceiling);
        }

        if chunk.is_empty() {
            return Ok(true);
        }
        Ok(self.tx.send(chunk).is_ok())
    }
}

fn decode_thread(
    args: Args,
    shared: Arc<Shared>,
    spec_tx: mpsc::Sender<(u32, u16)>,
    rate_rx: Receiver<u32>,
    tx: SyncSender<Vec<f32>>,
) -> anyhow::Result<()> {
    let mss = MediaSourceStream::new(
        Box::new(
            std::fs::File::open(args.file.as_deref().context("no input file specified")?)
                .context("audio file not found")?,
        ),
        Default::default(),
    );
    let mut format = symphonia::default::get_probe().probe(
        &Hint::new(),
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    )?;
    let track = format
        .default_track(TrackType::Audio)
        .context("no audio track")?;
    let mut decoder = symphonia::default::get_codecs().make_audio_decoder(
        track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .context("no audio track")?,
        &AudioDecoderOptions::default(),
    )?;

    let mut pipeline: Option<Pipeline> = None;
    let mut resampler: Option<ResampleStage> = None;
    let mut window_len = 0usize;
    let (mut scratch, mut window) = (Vec::<f32>::new(), Vec::<f32>::new());

    while let Some(packet) = format.next_packet()? {
        if shared.stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let decoded = decoder.decode(&packet)?;
        if pipeline.is_none() {
            let file_rate = decoded.spec().rate();
            let channels = decoded.spec().channels().count();
            check_channels(channels)?;
            spec_tx.send((file_rate, channels as u16)).ok();
            let out_rate = rate_rx.recv().context("output device setup failed")?;
            if out_rate != file_rate {
                resampler = Some(ResampleStage::new(file_rate, out_rate, channels)?);
            }
            pipeline = Some(Pipeline::new(&args, out_rate, channels, tx.clone())?);
            window_len = std::cmp::Ord::max(file_rate as usize / WINDOW_DIV, 1) * channels;
        }
        let p = pipeline.as_mut().context("pipeline missing")?;
        decoded.copy_to_vec_interleaved(&mut scratch);
        window.extend_from_slice(&scratch);
        while window.len() >= window_len {
            let mut chunk: Vec<f32> = window.drain(..window_len).collect();
            if let Some(r) = resampler.as_mut() {
                chunk = r.run(&chunk, false)?;
            }
            if !p.push(chunk, false)? {
                return Ok(());
            }
        }
    }

    if let Some(p) = pipeline.as_mut() {
        let mut rest = std::mem::take(&mut window);
        if let Some(r) = resampler.as_mut() {
            rest = r.run(&rest, true)?;
        }
        p.push(rest, true)?;
    }
    Ok(())
}

fn output_config(device: &Device, rate: u32, channels: u16) -> anyhow::Result<StreamConfig> {
    let configs = device
        .supported_output_configs()
        .context("could not query selected output device formats")?;
    for supported in configs {
        if supported.channels() == channels
            && supported.sample_format() == SampleFormat::F32
            && supported.min_sample_rate() <= rate
            && supported.max_sample_rate() >= rate
        {
            return Ok(supported.with_sample_rate(rate).config());
        }
    }
    bail!("no f32 output config for {channels} channels at {rate} Hz")
}

fn pick_output_config(
    device: &Device,
    file_rate: u32,
    channels: u16,
) -> anyhow::Result<StreamConfig> {
    let mut candidates = vec![file_rate];
    if let Ok(default) = device.default_output_config() {
        candidates.push(default.sample_rate());
    }
    candidates.extend([48_000, 44_100, 96_000]);
    candidates
        .into_iter()
        .find_map(|rate| output_config(device, rate, channels).ok())
        .context("selected output device supports none of the common sample rates (f32)")
}

#[derive(Default)]
struct Shared {
    played: AtomicUsize,
    done: AtomicBool,
    stop: AtomicBool,
    device_error: AtomicBool,
}

struct Player {
    rx: Receiver<Vec<f32>>,
    shared: Arc<Shared>,
    volume: Arc<AtomicU32>,
    last_vol: f32,
    channels: usize,
    ceiling: f32,
    cur: Vec<f32>,
    pos: usize,
}

impl Player {
    fn new(
        rx: Receiver<Vec<f32>>,
        shared: Arc<Shared>,
        volume: Arc<AtomicU32>,
        channels: usize,
        ceiling: f32,
    ) -> Self {
        let last_vol = f32::from_bits(volume.load(Ordering::Relaxed));
        Self {
            rx,
            shared,
            volume,
            last_vol,
            channels: std::cmp::Ord::max(channels, 1),
            ceiling,
            cur: Vec::new(),
            pos: 0,
        }
    }

    fn fill(&mut self, data: &mut [f32]) {
        let ceiling = self.ceiling;
        let target = f32::from_bits(self.volume.load(Ordering::Relaxed));
        let start = self.last_vol;
        let total_frames = std::cmp::Ord::max(data.len() / self.channels, 1);
        let step = (target - start) / total_frames as f32;
        self.last_vol = target;
        let mut emitted = 0usize;
        for (i, frame) in data.chunks_mut(self.channels).enumerate() {
            let vol = start + step * (i + 1) as f32;
            if frame.len() != self.channels {
                frame.fill(0.0);
                continue;
            }

            while self.pos >= self.cur.len() {
                match self.rx.try_recv() {
                    Ok(v) => {
                        self.cur = v;
                        self.pos = 0;
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.shared.done.store(true, Ordering::Relaxed);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }

            let stopping = self.shared.stop.load(Ordering::Relaxed);
            let have_frame = !stopping && self.pos + frame.len() <= self.cur.len();

            if have_frame {
                for sample in frame.iter_mut() {
                    let value = self.cur[self.pos];
                    self.pos += 1;
                    *sample = if value.is_finite() {
                        (value * vol).clamp(-ceiling, ceiling)
                    } else {
                        0.0
                    };
                }
                emitted += frame.len();
            } else {
                if !stopping && self.pos < self.cur.len() {
                    self.pos = self.cur.len();
                }
                frame.fill(0.0);
            }
        }
        self.shared.played.fetch_add(emitted, Ordering::Relaxed);
    }
}

pub fn run(args: &Args) -> anyhow::Result<()> {
    let device = cpal::default_host()
        .default_output_device()
        .context("no default output device")?;
    let ceiling = 10.0_f32.powf(args.ceiling / 20.0);
    let (spec_tx, spec_rx) = mpsc::channel();
    let (rate_tx, rate_rx) = mpsc::channel::<u32>();
    let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(QUEUE_DEPTH);
    let shared = Arc::new(Shared::default());
    let thread_args = args.clone();
    let thread_shared = Arc::clone(&shared);
    let handle =
        std::thread::spawn(move || decode_thread(thread_args, thread_shared, spec_tx, rate_rx, tx));

    let (file_rate, channels) = match spec_rx.recv() {
        Ok(spec) => spec,
        Err(_) => {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("decoder thread panicked"))??;
            bail!("no audio decoded");
        }
    };

    let config = pick_output_config(&device, file_rate, channels)?;
    let rate = config.sample_rate;
    rate_tx.send(rate).ok();

    let err_shared = Arc::clone(&shared);
    let mut player = Player::new(
        rx,
        Arc::clone(&shared),
        Arc::clone(&args.volume),
        channels as usize,
        ceiling,
    );

    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| player.fill(data),
            move |err| {
                eprintln!("Audio output device error: {err}");
                err_shared.device_error.store(true, Ordering::Relaxed);
                err_shared.stop.store(true, Ordering::Relaxed);
                err_shared.done.store(true, Ordering::Relaxed);
            },
            None,
        )
        .context("failed to build stream?!")?;
    stream.play().context("failed to play stream?!")?;

    let mut handle = Some(handle);
    loop {
        if args.stop_queued.load(Ordering::Relaxed) {
            shared.stop.store(true, Ordering::Relaxed);
        }

        if shared.device_error.load(Ordering::Relaxed) {
            shared.stop.store(true, Ordering::Relaxed);

            drop(stream);
            if let Some(h) = handle.take() {
                let _ = h.join();
            }
            bail!("audio output device reported an error; playback stopped");
        }

        if shared.stop.load(Ordering::Relaxed) {
            drop(stream);
            if let Some(h) = handle.take() {
                let _ = h.join();
            }
            return Ok(());
        }

        if handle.as_ref().is_some_and(|h| h.is_finished()) {
            match handle.take().unwrap().join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    shared.stop.store(true, Ordering::Relaxed);
                    std::thread::sleep(Duration::from_millis(120));
                    return Err(e);
                }
                Err(_) => {
                    shared.stop.store(true, Ordering::Relaxed);
                    std::thread::sleep(Duration::from_millis(120));
                    bail!("decoder thread panicked");
                }
            }
        }
        std::io::stdout().flush().ok();

        if shared.done.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!();

    if let Some(h) = handle {
        h.join()
            .map_err(|_| anyhow::anyhow!("decoder thread panicked"))??;
    }

    std::thread::sleep(Duration::from_millis(300));
    Ok(())
}

#[cfg(test)]
mod safety_tests {
    use super::Limiter;

    #[test]
    fn limiter_keeps_sample_peaks_below_ceiling() {
        let ceiling = 0.25_f32;
        let mut limiter = Limiter::new(48_000, 2, ceiling);
        let mut audio = vec![0.0_f32; 512 * 2];
        audio[100 * 2] = 4.0;
        audio[100 * 2 + 1] = -3.0;
        audio[240 * 2] = 1.5;
        limiter.process(&mut audio);
        let tail = limiter.drain();
        audio.extend(tail);
        assert!(audio.iter().all(|sample| sample.abs() <= ceiling + 1e-6));
    }

    #[test]
    fn limiter_emits_initial_lookahead_silence() {
        let mut limiter = Limiter::new(48_000, 1, 0.25);
        let mut audio = vec![0.5_f32; 32];
        limiter.process(&mut audio);
        let tail = limiter.drain();
        audio.extend(tail);
        assert!(
            audio
                .iter()
                .take(limiter.lookahead)
                .all(|sample| *sample == 0.0)
        );
    }

    #[test]
    fn limiter_links_channels() {
        let ceiling = 0.25_f32;
        let mut limiter = Limiter::new(48_000, 2, ceiling);
        let mut audio = vec![0.0_f32; 512 * 2];
        audio[100 * 2] = 2.0;
        audio[100 * 2 + 1] = 0.1;
        limiter.process(&mut audio);
        let tail = limiter.drain();
        audio.extend(tail);
        let peak_left = audio.iter().step_by(2).fold(0.0_f32, |p, x| p.max(x.abs()));
        let peak_right = audio
            .iter()
            .skip(1)
            .step_by(2)
            .fold(0.0_f32, |p, x| p.max(x.abs()));
        assert!(peak_left <= ceiling + 1e-6);
        assert!(peak_right <= ceiling + 1e-6);
        assert!(
            peak_right > 0.0,
            "right channel should not be muted by linked limiting"
        );
    }
}

pub fn warn_extreme_settings(args: &Args) -> Vec<&str> {
    let mut res = vec![];
    if args.preamp.is_some_and(|gain| gain > 12.0) {
        res.push("WARNING: preamp boost above +12 dB can create very high internal levels");
    }
    if args.postamp.is_some_and(|gain| gain > 12.0) {
        res.push("WARNING: postamp boost above +12 dB can create very high internal levels");
    }
    if args.comp.is_some_and(|comp| comp.makeup > 6.0) {
        res.push("WARNING: compressor makeup gain above +6 dB can substantially increase output");
    }

    let total_eq_boost: f32 = args.eq.iter().map(|band| band.gain.max(0.0)).sum();
    if args.eq.iter().any(|band| band.gain > 6.0) || total_eq_boost > 12.0 {
        res.push("WARNING: large or accumulated EQ boosts can create substantial signal gain");
    }

    if args.ceiling > -11.9 {
        res.push("WARNING: output ceiling is above the conservative -12 dBFS default");
    }
    res
}
