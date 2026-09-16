use std::error::Error;
use std::fmt;
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use wayland_client::backend::WaylandError;
use wayland_client::protocol::{wl_output::WlOutput, wl_registry};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1,
    zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
};

/// The colour temperature "Warmth" is 0% at, and the upper bound
/// `config::Settings::sanitised` clamps `warmest_temperature_k` against -- the
/// two must stay ordered or the slider stops meaning anything.
pub(crate) const NEUTRAL_TEMPERATURE_K: f32 = 6500.0;
const GAMMA_BACKEND_NAME: &str = "Gamma";
/// Dim is a true per-channel multiply now, so this is a brightness floor of
/// 0.10 rather than an opacity cap. The same floor gammastep uses.
const MAX_DIM: f32 = 0.9;
/// How long to wait before retrying something the compositor refused. Without
/// this, a slider drag would retry roughly sixty times a second.
const RETRY_BACKOFF: Duration = Duration::from_secs(2);

#[derive(Debug, Default)]
pub struct BlueLightFilter {
    backend: Option<WaylandGammaFilter>,
    /// A failed connection, remembered briefly so that a drag cannot hammer a
    /// compositor that has no gamma control.
    connect_failure: Option<(Instant, FilterError)>,
}

impl BlueLightFilter {
    pub fn set_strength(
        &mut self,
        strength_percent: f32,
        dim_percent: f32,
        warmest_temperature_k: f32,
    ) -> Result<FilterStatus, FilterError> {
        let strength = sanitise(strength_percent, 1.0);
        let dim = sanitise(dim_percent, MAX_DIM);

        if strength <= f32::EPSILON && dim <= f32::EPSILON {
            self.clear();
            return Ok(FilterStatus::Inactive);
        }

        let temperature = temperature_for_strength(strength, warmest_temperature_k);
        let gains = channel_gains(temperature);
        let brightness = 1.0 - dim;

        if self.backend.is_none() {
            if let Some((at, err)) = &self.connect_failure
                && at.elapsed() < RETRY_BACKOFF
            {
                return Err(err.clone());
            }
            match WaylandGammaFilter::new() {
                Ok(backend) => {
                    self.connect_failure = None;
                    self.backend = Some(backend);
                }
                Err(err) => {
                    self.connect_failure = Some((Instant::now(), err.clone()));
                    return Err(err);
                }
            }
        }

        let backend = self.backend.as_mut().expect("backend was just initialized");
        let (applied, total) = backend.apply(temperature, gains, brightness)?;

        Ok(FilterStatus::Active {
            backend: GAMMA_BACKEND_NAME,
            temperature_kelvin: temperature.round() as u16,
            dim_percent: (dim * 100.0).round() as u8,
            applied,
            total,
        })
    }

    /// Dropping the backend destroys every gamma control, which is what restores
    /// the original ramps and releases the outputs for other clients.
    fn clear(&mut self) {
        self.backend = None;
        self.connect_failure = None;
    }
}

impl Drop for BlueLightFilter {
    fn drop(&mut self) {
        self.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterStatus {
    Inactive,
    Active {
        backend: &'static str,
        temperature_kelvin: u16,
        dim_percent: u8,
        applied: usize,
        total: usize,
    },
}

impl fmt::Display for FilterStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilterStatus::Inactive => f.write_str("Off"),
            FilterStatus::Active {
                backend,
                temperature_kelvin,
                dim_percent,
                applied,
                total,
            } => {
                write!(f, "{backend}: {temperature_kelvin} K, -{dim_percent}%")?;
                if applied != total {
                    write!(f, " ({applied}/{total} displays)")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum FilterError {
    /// The compositor never advertised `zwlr_gamma_control_manager_v1`.
    NoProtocol,
    /// It did, but every output refused. The protocol cannot tell us why: another
    /// client holding the output, an output with no gamma table, and a compositor
    /// running on a non-KMS backend all look identical from here.
    Unavailable,
    NoOutputs,
    Wayland(String),
}

impl fmt::Display for FilterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilterError::NoProtocol => f.write_str("No gamma control in this compositor"),
            FilterError::Unavailable => {
                f.write_str("Gamma control unavailable - another app (gammastep?) may hold it")
            }
            FilterError::NoOutputs => f.write_str("No displays reported"),
            FilterError::Wayland(message) => f.write_str(message),
        }
    }
}

impl Error for FilterError {}

impl From<std::io::Error> for FilterError {
    fn from(err: std::io::Error) -> Self {
        Self::Wayland(err.to_string())
    }
}

/// Turn a slider percentage into a clamped 0..=max fraction, refusing anything
/// non-finite.
///
/// This guard is load-bearing, not defensive tidiness. `NaN` fails every
/// comparison, so `strength <= f32::EPSILON` is false for it and the "nothing
/// to do, switch the filter off" branch is skipped; `NaN.clamp(..)` is still
/// `NaN`; and `NaN as u16` is a *saturating* cast that yields 0. A single
/// non-finite value would therefore write an all-zero ramp to every channel of
/// every output -- a black screen, re-asserted every few seconds, with no
/// visible UI left to undo it. "We do not know" has to mean "no filter".
fn sanitise(percent: f32, max: f32) -> f32 {
    if percent.is_finite() {
        (percent / 100.0).clamp(0.0, max)
    } else {
        0.0
    }
}

/// The colour temperature a slider position corresponds to, so the UI can label
/// what it is actually doing. Takes percent, like `set_strength`, so nothing
/// outside this module needs to know about the 0..=1 domain used internally.
#[must_use]
pub fn temperature_for_percent(strength_percent: f32, warmest_temperature_k: f32) -> f32 {
    temperature_for_strength(sanitise(strength_percent, 1.0), warmest_temperature_k)
}

fn temperature_for_strength(strength: f32, warmest_temperature_k: f32) -> f32 {
    NEUTRAL_TEMPERATURE_K - ((NEUTRAL_TEMPERATURE_K - warmest_temperature_k) * strength)
}

/// Per-channel gains for a blackbody at `temperature_kelvin`, using the Tanner
/// Helland approximation of the Planckian locus.
fn raw_gains(temperature_kelvin: f32) -> [f32; 3] {
    let t = (temperature_kelvin / 100.0).clamp(10.0, 400.0);

    let red = if t <= 66.0 {
        255.0
    } else {
        329.698_73 * (t - 60.0).powf(-0.133_204_76)
    };

    let green = if t <= 66.0 {
        99.470_8 * t.ln() - 161.119_57
    } else {
        288.122_16 * (t - 60.0).powf(-0.075_514_85)
    };

    let blue = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_73 * (t - 10.0).ln() - 305.044_8
    };

    [
        red.clamp(0.0, 255.0) / 255.0,
        green.clamp(0.0, 255.0) / 255.0,
        blue.clamp(0.0, 255.0) / 255.0,
    ]
}

/// Gains normalised against the neutral point, so 6500 K is exactly identity.
fn channel_gains(temperature_kelvin: f32) -> [f32; 3] {
    let neutral = raw_gains(NEUTRAL_TEMPERATURE_K);
    let gains = raw_gains(temperature_kelvin);

    [
        (gains[0] / neutral[0]).clamp(0.0, 1.0),
        (gains[1] / neutral[1]).clamp(0.0, 1.0),
        (gains[2] / neutral[2]).clamp(0.0, 1.0),
    ]
}

#[derive(Debug)]
struct WaylandGammaFilter {
    /// This is the applet's *second* Wayland connection, separate from the one
    /// libcosmic holds. It works because libcosmic connects first during
    /// `cosmic::applet::run` and consumes `WAYLAND_SOCKET` if it was set, leaving
    /// `WAYLAND_DISPLAY` for us. Do not move this construction any earlier.
    _connection: Connection,
    event_queue: EventQueue<GammaState>,
    state: GammaState,
}

impl WaylandGammaFilter {
    fn new() -> Result<Self, FilterError> {
        let connection = Connection::connect_to_env()
            .map_err(|err| FilterError::Wayland(format!("Could not connect to Wayland: {err}")))?;
        let mut event_queue = connection.new_event_queue();
        let queue_handle = event_queue.handle();
        connection.display().get_registry(&queue_handle, ());

        let mut state = GammaState::default();
        event_queue
            .roundtrip(&mut state)
            .map_err(|err| FilterError::Wayland(format!("Could not read Wayland globals: {err}")))?;

        if state.manager.is_none() {
            return Err(FilterError::NoProtocol);
        }
        if state.outputs.is_empty() {
            return Err(FilterError::NoOutputs);
        }

        // The second roundtrip is what makes a truthful status possible straight
        // away: `gamma_size` and `failed` only arrive after one.
        state.create_missing_controls(&queue_handle);
        event_queue.roundtrip(&mut state).map_err(|err| {
            FilterError::Wayland(format!("Could not create gamma controls: {err}"))
        })?;

        Ok(Self {
            _connection: connection,
            event_queue,
            state,
        })
    }

    /// Returns how many outputs took the ramp, and how many there are.
    fn apply(
        &mut self,
        temperature: f32,
        gains: [f32; 3],
        brightness: f32,
    ) -> Result<(usize, usize), FilterError> {
        self.pump()?;

        let queue_handle = self.event_queue.handle();
        if self.state.create_missing_controls(&queue_handle) > 0 {
            self.event_queue.roundtrip(&mut self.state).map_err(|err| {
                FilterError::Wayland(format!("Could not create gamma controls: {err}"))
            })?;
        }

        // Every in-flight `set_gamma` needs its own file: the fd is shared with
        // the compositor through SCM_RIGHTS, offset and all, and the compositor
        // reads it when it dispatches the request rather than when we send it.
        // Rewinding a shared file under a read in progress would give a short
        // read, which makes the compositor reset the output to identity.
        let mut in_flight = Vec::new();
        let mut applied = 0;

        for entry in &mut self.state.outputs {
            let (Some(control), Some(size)) = (entry.control.as_ref(), entry.ramp_size) else {
                continue;
            };
            applied += 1;

            let key = (size, temperature.to_bits(), brightness.to_bits());
            if entry.last_ramp == Some(key) {
                continue;
            }

            let mut file = tempfile::tempfile()?;
            file.write_all(&ramp_bytes(size, gains, brightness))?;
            file.seek(SeekFrom::Start(0))?;
            control.set_gamma(file.as_fd());
            entry.last_ramp = Some(key);
            in_flight.push(file);
        }

        if !in_flight.is_empty() {
            self.event_queue.flush().map_err(|err| {
                FilterError::Wayland(format!("Could not send gamma update: {err}"))
            })?;
        }
        drop(in_flight);

        if applied == 0 {
            return Err(FilterError::Unavailable);
        }
        Ok((applied, self.state.outputs.len()))
    }

    /// Drain the socket without blocking.
    ///
    /// `dispatch_pending` on its own is not enough: it only dispatches what
    /// something has already read off the socket, and nothing else reads this
    /// connection. Without an explicit read we would never see `failed`, never
    /// see an output appear or disappear, and slowly fill the receive buffer.
    fn pump(&mut self) -> Result<(), FilterError> {
        self.dispatch_pending()?;
        if let Some(guard) = self.event_queue.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                // Nothing waiting. Both wayland-backend implementations treat
                // this as non-fatal and leave the connection usable.
                Err(WaylandError::Io(err)) if err.kind() == ErrorKind::WouldBlock => {}
                Err(err) => {
                    return Err(FilterError::Wayland(format!(
                        "Could not read Wayland events: {err}"
                    )));
                }
            }
        }
        self.dispatch_pending()
    }

    fn dispatch_pending(&mut self) -> Result<(), FilterError> {
        self.event_queue
            .dispatch_pending(&mut self.state)
            .map(|_| ())
            .map_err(|err| FilterError::Wayland(format!("Could not dispatch Wayland events: {err}")))
    }
}

impl Drop for WaylandGammaFilter {
    fn drop(&mut self) {
        for entry in &mut self.state.outputs {
            if let Some(control) = entry.control.take() {
                control.destroy();
            }
        }
        if let Some((_, manager)) = self.state.manager.take() {
            manager.destroy();
        }
        // These requests are only buffered. The connection is dropped right
        // after this, and an unflushed buffer is simply discarded.
        let _ = self.event_queue.flush();
    }
}

#[derive(Debug, Default)]
struct GammaState {
    /// Paired with its registry name so that its removal is representable.
    manager: Option<(u32, ZwlrGammaControlManagerV1)>,
    outputs: Vec<OutputEntry>,
}

impl GammaState {
    /// Give every output that lacks one a gamma control, honouring the retry
    /// backoff. Returns how many were created, since each one owes us a
    /// `gamma_size` before it can be written to.
    fn create_missing_controls(&mut self, queue_handle: &QueueHandle<Self>) -> usize {
        let Some((_, manager)) = self.manager.clone() else {
            return 0;
        };

        let now = Instant::now();
        let mut created = 0;

        for entry in &mut self.outputs {
            if entry.control.is_some() || entry.retry_after.is_some_and(|at| now < at) {
                continue;
            }
            entry.control = Some(manager.get_gamma_control(&entry.output, queue_handle, entry.name));
            entry.ramp_size = None;
            entry.last_ramp = None;
            entry.retry_after = None;
            created += 1;
        }

        created
    }
}

#[derive(Debug)]
struct OutputEntry {
    /// The `wl_registry` global name: unique for the compositor's lifetime, and
    /// exactly what `GlobalRemove` reports. Keying by index into `outputs` would
    /// alias a different output the moment one is unplugged.
    name: u32,
    output: WlOutput,
    output_version: u32,
    control: Option<ZwlrGammaControlV1>,
    /// `Some` once `gamma_size` has arrived; until then the control is unusable.
    ramp_size: Option<u32>,
    /// Set when the compositor refuses us, to gate the next attempt.
    retry_after: Option<Instant>,
    /// What we last sent, so an unmoved slider costs nothing.
    last_ramp: Option<(u32, u32, u32)>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for GammaState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        &(): &(),
        _: &Connection,
        queue_handle: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_output" => {
                    let version = version.min(4);
                    let output = registry.bind::<WlOutput, _, _>(name, version, queue_handle, ());
                    state.outputs.push(OutputEntry {
                        name,
                        output,
                        output_version: version,
                        control: None,
                        ramp_size: None,
                        retry_after: None,
                        last_ramp: None,
                    });
                }
                // The protocol is frozen at version 1.
                "zwlr_gamma_control_manager_v1" => {
                    let manager =
                        registry.bind::<ZwlrGammaControlManagerV1, _, _>(name, 1, queue_handle, ());
                    state.manager = Some((name, manager));
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                if state.manager.as_ref().is_some_and(|(id, _)| *id == name) {
                    state.manager = None;
                }
                if let Some(index) = state.outputs.iter().position(|entry| entry.name == name) {
                    let mut entry = state.outputs.remove(index);
                    // Proxies are not RAII: dropping one sends nothing, and the
                    // compositor-side resource - and its ramp - would survive.
                    if let Some(control) = entry.control.take() {
                        control.destroy();
                    }
                    if entry.output_version >= 3 {
                        entry.output.release();
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrGammaControlV1, u32> for GammaState {
    fn event(
        state: &mut Self,
        control: &ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(entry) = state.outputs.iter_mut().find(|entry| entry.name == *name) else {
            control.destroy();
            return;
        };
        // An event for a control we have already replaced or given up on.
        if entry.control.as_ref() != Some(control) {
            return;
        }

        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } if size > 0 => {
                entry.ramp_size = Some(size);
            }
            // `failed` means the compositor has given up on this output - another
            // client holds it, it has no gamma table, or programming it failed.
            // A zero ramp size is just as unusable. Either way the spec wants the
            // object destroyed.
            zwlr_gamma_control_v1::Event::GammaSize { .. }
            | zwlr_gamma_control_v1::Event::Failed => {
                if let Some(control) = entry.control.take() {
                    control.destroy();
                }
                entry.ramp_size = None;
                entry.last_ramp = None;
                entry.retry_after = Some(Instant::now() + RETRY_BACKOFF);
            }
            _ => {}
        }
    }
}

// Neither interface sends us anything we act on. `ignore` rather than the plain
// form, whose body is `unreachable!()`: panicking inside a Wayland event handler
// is a worse failure mode for a panel applet than dropping a stray event.
delegate_noop!(GammaState: ignore WlOutput);
delegate_noop!(GammaState: ignore ZwlrGammaControlManagerV1);

/// One output's whole gamma table: the R ramp, then G, then B, native-endian
/// `u16`, exactly `3 * size * 2` bytes.
///
/// The compositor reads this with `read_exact` into a `vec![0u16; size * 3]` and
/// then checks for EOF, so the length has to be exact - a trailing byte is an
/// error, not slack.
///
/// The curve is a plain linear multiply on the encoded value, `i * 65536/size`
/// scaled by the gain, which is what gammastep writes and what leaves black at
/// exactly zero. Scaling by `65535/(size-1)` instead would be the same to within
/// 0.1% at white; this form matches the reference byte for byte.
// Every cast below is bounded: `i` is less than `size`, and the product is
// clamped into `u16` range before conversion.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn ramp_bytes(size: u32, gains: [f32; 3], brightness: f32) -> Vec<u8> {
    let count = size as usize;
    let step = 65536.0 / f64::from(size);
    let mut bytes = Vec::with_capacity(count * 3 * 2);

    for gain in gains {
        let scale = f64::from((gain * brightness).clamp(0.0, 1.0));
        for i in 0..count {
            let value = (i as f64 * step * scale).round().clamp(0.0, 65535.0) as u16;
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
    }

    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::float_cmp)]
    #[test]
    fn non_finite_input_cannot_reach_the_ramp() {
        // The failure this prevents is a black screen, so assert the numbers
        // rather than trusting the clamp.
        assert_eq!(sanitise(f32::NAN, 1.0), 0.0);
        assert_eq!(sanitise(f32::INFINITY, 1.0), 0.0);
        assert_eq!(sanitise(f32::NEG_INFINITY, 1.0), 0.0);
        assert_eq!(sanitise(150.0, 1.0), 1.0);
        assert_eq!(sanitise(-10.0, 1.0), 0.0);
        assert_eq!(sanitise(50.0, 1.0), 0.5);
        assert_eq!(sanitise(100.0, MAX_DIM), MAX_DIM);
    }

    #[test]
    fn a_nan_ramp_would_have_been_all_zero() {
        // Documents why the guard exists: this is what the cast does.
        assert_eq!(f64::NAN.round().clamp(0.0, 65535.0) as u16, 0);
    }

    #[test]
    fn neutral_is_identity_and_warmest_is_warm() {
        const WARMEST: f32 = 1000.0;
        assert!((temperature_for_percent(0.0, WARMEST) - 6500.0).abs() < 0.5);
        assert!((temperature_for_percent(100.0, WARMEST) - 1000.0).abs() < 0.5);
        let gains = channel_gains(6500.0);
        assert!((gains[0] - 1.0).abs() < 1e-6 && (gains[1] - 1.0).abs() < 1e-6);
        let warm = channel_gains(3000.0);
        assert!(warm[0] > warm[1] && warm[1] > warm[2], "red > green > blue at 3000 K");
    }

    #[test]
    fn ramp_is_exactly_three_channels_of_u16_with_black_at_zero() {
        let bytes = ramp_bytes(1024, [1.0, 0.8, 0.6], 1.0);
        assert_eq!(bytes.len(), 3 * 1024 * 2, "compositor read_exacts this length");
        assert_eq!(&bytes[0..2], &0u16.to_ne_bytes(), "black must stay black");
    }
}
