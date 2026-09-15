use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsFd;

const NEUTRAL_TEMPERATURE_K: f32 = 6500.0;
const WARMEST_TEMPERATURE_K: f32 = 2400.0;
const OVERLAY_BACKEND_NAME: &str = "Wayland overlay";
/// Never let the overlay go fully opaque; the screen must stay usable.
const MAX_DIM: f32 = 0.9;

#[derive(Debug, Default)]
pub struct BlueLightFilter {
    backend: Option<WaylandOverlayFilter>,
}

impl BlueLightFilter {
    pub fn set_strength(
        &mut self,
        strength_percent: f32,
        dim_percent: f32,
    ) -> Result<FilterStatus, FilterError> {
        let strength = (strength_percent / 100.0).clamp(0.0, 1.0);
        let dim = (dim_percent / 100.0).clamp(0.0, MAX_DIM);

        if strength <= f32::EPSILON && dim <= f32::EPSILON {
            self.clear();
            return Ok(FilterStatus::Inactive);
        }

        let temperature = temperature_for_strength(strength);
        let color = overlay_color(channel_gains(temperature), dim);

        if self.backend.is_none() {
            self.backend = Some(WaylandOverlayFilter::new()?);
        }

        let backend = self.backend.as_mut().expect("backend was just initialized");
        backend.apply(color)?;

        Ok(FilterStatus::Active {
            backend: OVERLAY_BACKEND_NAME,
            temperature_kelvin: temperature.round() as u16,
            dim_percent: (dim * 100.0).round() as u8,
        })
    }

    fn clear(&mut self) {
        if let Some(mut backend) = self.backend.take() {
            backend.clear();
        }
    }
}

impl Drop for BlueLightFilter {
    fn drop(&mut self) {
        self.clear();
    }
}

#[derive(Debug)]
pub enum FilterStatus {
    Inactive,
    Active {
        backend: &'static str,
        temperature_kelvin: u16,
        dim_percent: u8,
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
            } => write!(f, "{backend}: {temperature_kelvin} K, -{dim_percent}%"),
        }
    }
}

#[derive(Debug)]
pub struct FilterError(String);

impl FilterError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for FilterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for FilterError {}

impl From<std::io::Error> for FilterError {
    fn from(err: std::io::Error) -> Self {
        Self::new(err.to_string())
    }
}

fn temperature_for_strength(strength: f32) -> f32 {
    NEUTRAL_TEMPERATURE_K - ((NEUTRAL_TEMPERATURE_K - WARMEST_TEMPERATURE_K) * strength)
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
struct WaylandOverlayFilter {
    _connection: wayland_client::Connection,
    event_queue: wayland_client::EventQueue<OverlayState>,
    state: OverlayState,
}

impl WaylandOverlayFilter {
    fn new() -> Result<Self, FilterError> {
        use wayland_client::Connection;

        let connection = Connection::connect_to_env()
            .map_err(|err| FilterError::new(format!("Could not connect to Wayland: {err}")))?;
        let mut event_queue = connection.new_event_queue();
        let queue_handle = event_queue.handle();
        let display = connection.display();
        let _registry = display.get_registry(&queue_handle, ());

        let mut state = OverlayState::default();
        event_queue.roundtrip(&mut state).map_err(|err| {
            FilterError::new(format!("Could not read Wayland overlay globals: {err}"))
        })?;

        state.create_surfaces(&queue_handle)?;
        event_queue
            .roundtrip(&mut state)
            .map_err(|err| FilterError::new(format!("Could not configure overlay: {err}")))?;

        if state.surfaces.iter().all(|surface| !surface.configured) {
            return Err(FilterError::new("Wayland overlay was not configured"));
        }

        Ok(Self {
            _connection: connection,
            event_queue,
            state,
        })
    }

    fn apply(&mut self, color: OverlayColor) -> Result<(), FilterError> {
        self.event_queue
            .dispatch_pending(&mut self.state)
            .map_err(|err| FilterError::new(format!("Could not dispatch overlay events: {err}")))?;

        if self
            .state
            .surfaces
            .iter()
            .any(|surface| !surface.configured && !surface.closed)
        {
            self.event_queue
                .roundtrip(&mut self.state)
                .map_err(|err| FilterError::new(format!("Could not configure overlay: {err}")))?;
        }

        let queue_handle = self.event_queue.handle();
        self.state.draw(&queue_handle, color)?;
        self.event_queue
            .flush()
            .map_err(|err| FilterError::new(format!("Could not flush overlay update: {err}")))?;

        Ok(())
    }

    fn clear(&mut self) {
        for surface in &self.state.surfaces {
            surface.surface.attach(None, 0, 0);
            surface.surface.commit();
            surface.layer_surface.destroy();
        }
        let _ = self.event_queue.flush();
    }
}

#[derive(Debug, Default)]
struct OverlayState {
    compositor: Option<wayland_client::protocol::wl_compositor::WlCompositor>,
    shm: Option<wayland_client::protocol::wl_shm::WlShm>,
    layer_shell: Option<wayland_layer::zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    outputs: Vec<wayland_client::protocol::wl_output::WlOutput>,
    surfaces: Vec<OverlaySurface>,
}

impl OverlayState {
    fn create_surfaces(
        &mut self,
        queue_handle: &wayland_client::QueueHandle<Self>,
    ) -> Result<(), FilterError> {
        let compositor = self
            .compositor
            .clone()
            .ok_or_else(|| FilterError::new("Wayland compositor global is unavailable"))?;
        let layer_shell = self
            .layer_shell
            .clone()
            .ok_or_else(|| FilterError::new("Wayland layer-shell global is unavailable"))?;

        if self.outputs.is_empty() {
            return Err(FilterError::new("Wayland did not report any outputs"));
        }

        for output in self.outputs.clone() {
            let index = self.surfaces.len();
            let surface = compositor.create_surface(queue_handle, index);
            let region = compositor.create_region(queue_handle, ());
            surface.set_input_region(Some(&region));
            region.destroy();

            let layer_surface = layer_shell.get_layer_surface(
                &surface,
                Some(&output),
                wayland_layer::zwlr_layer_shell_v1::Layer::Overlay,
                "cosmic-ext-redeye-blue-light".to_string(),
                queue_handle,
                index,
            );
            layer_surface.set_anchor(
                wayland_layer::zwlr_layer_surface_v1::Anchor::Top
                    | wayland_layer::zwlr_layer_surface_v1::Anchor::Bottom
                    | wayland_layer::zwlr_layer_surface_v1::Anchor::Left
                    | wayland_layer::zwlr_layer_surface_v1::Anchor::Right,
            );
            layer_surface.set_exclusive_zone(-1);
            layer_surface.set_size(0, 0);

            self.surfaces.push(OverlaySurface {
                surface: surface.clone(),
                layer_surface,
                width: 1,
                height: 1,
                configured: false,
                closed: false,
                buffer: None,
            });

            surface.commit();
        }

        Ok(())
    }

    fn draw(
        &mut self,
        queue_handle: &wayland_client::QueueHandle<Self>,
        color: OverlayColor,
    ) -> Result<(), FilterError> {
        let shm = self
            .shm
            .clone()
            .ok_or_else(|| FilterError::new("Wayland shared memory global is unavailable"))?;

        for (index, surface) in self.surfaces.iter_mut().enumerate() {
            if !surface.configured || surface.closed {
                continue;
            }

            let buffer = overlay_buffer(
                &shm,
                queue_handle,
                index,
                surface.width,
                surface.height,
                color,
            )?;
            surface.surface.attach(Some(&buffer.buffer), 0, 0);
            surface
                .surface
                .damage(0, 0, surface.width as i32, surface.height as i32);
            surface.surface.commit();
            surface.buffer = Some(buffer);
        }

        Ok(())
    }
}

#[derive(Debug)]
struct OverlaySurface {
    surface: wayland_client::protocol::wl_surface::WlSurface,
    layer_surface: wayland_layer::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    width: u32,
    height: u32,
    configured: bool,
    closed: bool,
    buffer: Option<OverlayBuffer>,
}

#[derive(Debug)]
struct OverlayBuffer {
    _file: File,
    _pool: wayland_client::protocol::wl_shm_pool::WlShmPool,
    buffer: wayland_client::protocol::wl_buffer::WlBuffer,
}

mod wayland_layer {
    pub use wayland_protocols_wlr::layer_shell::v1::client::{
        zwlr_layer_shell_v1, zwlr_layer_surface_v1,
    };
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_registry::WlRegistry, ()>
    for OverlayState
{
    fn event(
        state: &mut Self,
        registry: &wayland_client::protocol::wl_registry::WlRegistry,
        event: wayland_client::protocol::wl_registry::Event,
        _: &(),
        _: &wayland_client::Connection,
        queue_handle: &wayland_client::QueueHandle<OverlayState>,
    ) {
        if let wayland_client::protocol::wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(
                        registry
                            .bind::<wayland_client::protocol::wl_compositor::WlCompositor, _, _>(
                                name,
                                version.min(4),
                                queue_handle,
                                (),
                            ),
                    );
                }
                "wl_shm" => {
                    state.shm = Some(
                        registry.bind::<wayland_client::protocol::wl_shm::WlShm, _, _>(
                            name,
                            1,
                            queue_handle,
                            (),
                        ),
                    );
                }
                "wl_output" => {
                    state.outputs.push(
                        registry.bind::<wayland_client::protocol::wl_output::WlOutput, _, _>(
                            name,
                            version.min(4),
                            queue_handle,
                            (),
                        ),
                    );
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(
                        registry
                            .bind::<wayland_layer::zwlr_layer_shell_v1::ZwlrLayerShellV1, _, _>(
                                name,
                                version.min(4),
                                queue_handle,
                                (),
                            ),
                    );
                }
                _ => {}
            }
        }
    }
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_compositor::WlCompositor, ()>
    for OverlayState
{
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_compositor::WlCompositor,
        _: wayland_client::protocol::wl_compositor::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<OverlayState>,
    ) {
    }
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_shm::WlShm, ()> for OverlayState {
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_shm::WlShm,
        _: wayland_client::protocol::wl_shm::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<OverlayState>,
    ) {
    }
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_output::WlOutput, ()> for OverlayState {
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_output::WlOutput,
        _: wayland_client::protocol::wl_output::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<OverlayState>,
    ) {
    }
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_surface::WlSurface, usize>
    for OverlayState
{
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_surface::WlSurface,
        _: wayland_client::protocol::wl_surface::Event,
        _: &usize,
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<OverlayState>,
    ) {
    }
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_region::WlRegion, ()> for OverlayState {
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_region::WlRegion,
        _: wayland_client::protocol::wl_region::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<OverlayState>,
    ) {
    }
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_shm_pool::WlShmPool, usize>
    for OverlayState
{
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_shm_pool::WlShmPool,
        _: wayland_client::protocol::wl_shm_pool::Event,
        _: &usize,
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<OverlayState>,
    ) {
    }
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_buffer::WlBuffer, usize>
    for OverlayState
{
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_buffer::WlBuffer,
        _: wayland_client::protocol::wl_buffer::Event,
        _: &usize,
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<OverlayState>,
    ) {
    }
}

impl wayland_client::Dispatch<wayland_layer::zwlr_layer_shell_v1::ZwlrLayerShellV1, ()>
    for OverlayState
{
    fn event(
        _: &mut Self,
        _: &wayland_layer::zwlr_layer_shell_v1::ZwlrLayerShellV1,
        _: wayland_layer::zwlr_layer_shell_v1::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<OverlayState>,
    ) {
    }
}

impl wayland_client::Dispatch<wayland_layer::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, usize>
    for OverlayState
{
    fn event(
        state: &mut Self,
        layer_surface: &wayland_layer::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: wayland_layer::zwlr_layer_surface_v1::Event,
        index: &usize,
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<OverlayState>,
    ) {
        let Some(surface) = state.surfaces.get_mut(*index) else {
            return;
        };

        match event {
            wayland_layer::zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer_surface.ack_configure(serial);
                surface.width = width.max(1);
                surface.height = height.max(1);
                surface.configured = true;
            }
            wayland_layer::zwlr_layer_surface_v1::Event::Closed => {
                surface.closed = true;
            }
            _ => {}
        }
    }
}

/// A premultiplied ARGB pixel for the whole-screen overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OverlayColor {
    alpha: u8,
    red: u8,
    green: u8,
    blue: u8,
}

/// Builds the best "over" approximation of a per-channel multiply.
///
/// A gamma ramp (what GNOME and KDE use) computes `out_c = screen_c * gain_c`:
/// a different slope per channel. Alpha compositing can only produce
/// `out_c = tint_c + (1 - alpha) * screen_c`, where the slope `1 - alpha` is one
/// scalar shared by all three channels and only the intercept `tint_c` varies.
/// So colour can only be introduced by lifting blacks, which is the haze an
/// overlay can never fully escape.
///
/// Given that, take the slope from the smallest gain (making that channel exact)
/// and pick each intercept to minimise squared error across the tone range,
/// which lands at half the gain gap. Fitting the midtones this way halves the
/// black lift compared with matching white exactly.
///
/// Dimming is then a true multiply -- it scales every channel by the same
/// `keep`, so it composes exactly and costs no accuracy.
fn overlay_color(gains: [f32; 3], dim: f32) -> OverlayColor {
    let slope = gains[0].min(gains[1]).min(gains[2]);
    let keep = (1.0 - dim).clamp(0.0, 1.0);

    let intercept = |gain: f32| ((gain - slope) / 2.0).max(0.0) * keep;

    OverlayColor {
        alpha: to_u8(1.0 - keep * slope),
        red: to_u8(intercept(gains[0])),
        green: to_u8(intercept(gains[1])),
        blue: to_u8(intercept(gains[2])),
    }
}

fn to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * u8::MAX as f32).round() as u8
}

fn overlay_buffer(
    shm: &wayland_client::protocol::wl_shm::WlShm,
    queue_handle: &wayland_client::QueueHandle<OverlayState>,
    index: usize,
    width: u32,
    height: u32,
    color: OverlayColor,
) -> Result<OverlayBuffer, FilterError> {
    let stride = width
        .checked_mul(4)
        .ok_or_else(|| FilterError::new("Overlay width is too large"))?;
    let size = stride
        .checked_mul(height)
        .ok_or_else(|| FilterError::new("Overlay surface is too large"))?;
    let size_i32 = i32::try_from(size)
        .map_err(|_| FilterError::new("Overlay buffer is too large for Wayland shm"))?;

    let mut file = tempfile::tempfile()?;
    file.set_len(size as u64)?;
    write_overlay_pixels(&mut file, width, height, color)?;
    file.seek(SeekFrom::Start(0))?;

    let pool = shm.create_pool(file.as_fd(), size_i32, queue_handle, index);
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        stride as i32,
        wayland_client::protocol::wl_shm::Format::Argb8888,
        queue_handle,
        index,
    );

    Ok(OverlayBuffer {
        _file: file,
        _pool: pool,
        buffer,
    })
}

fn write_overlay_pixels(
    file: &mut File,
    width: u32,
    height: u32,
    color: OverlayColor,
) -> Result<(), FilterError> {
    let pixel = u32::from(color.alpha) << 24
        | u32::from(color.red) << 16
        | u32::from(color.green) << 8
        | u32::from(color.blue);
    let pixel = pixel.to_ne_bytes();
    let row_len = width as usize * 4;
    let mut row = Vec::with_capacity(row_len);

    for _ in 0..width {
        row.extend_from_slice(&pixel);
    }

    for _ in 0..height {
        file.write_all(&row)?;
    }

    file.flush()?;

    Ok(())
}
