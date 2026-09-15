use crate::blue_light::{self, BlueLightFilter};
use crate::config::{Settings, Store};
use crate::solar;
use cosmic::app::{Core, Task};
use cosmic::iced::core::window;
use cosmic::iced::window::Id;
use cosmic::iced::{Length, Rectangle, Subscription, mouse};
use cosmic::prelude::*;
use cosmic::surface::action::{app_popup, destroy_popup};
use cosmic::widget::{self};

const APPLET_ICON: &[u8] = include_bytes!("../resources/icons/hicolor/scalable/apps/Redeye.svg");

/// How often to re-assert the ramp. Gamma lives on the compositor side, so
/// things can change without the user touching anything: a display is plugged
/// in, or the compositor hands gamma control to someone else. The filter tracks
/// what it last sent per output, so a tick with nothing to do is only a socket
/// poll and writes nothing.
const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Percentage points of warmth per scroll message. A notch is about five
/// points, roughly 200 K -- enough to see, small enough to aim with. Note the
/// halving: libcosmic's `mouse_area` publishes `on_scroll` twice for one wheel
/// event (once in the match arm, once in the trailing `if let`, neither
/// guarded), so a single notch arrives here as two messages. If that is ever
/// fixed upstream the step silently halves, and this comment is the only clue.
const SCROLL_STEP: f32 = 2.5;

pub struct App {
    core: Core,
    popup: Option<Id>,
    settings: Settings,
    /// Where the sun was at the last tick. Held rather than recomputed in
    /// `view()`, so the popup shows the number that was actually applied and
    /// rendering stays free of the clock.
    elevation: f64,
    filter: BlueLightFilter,
    /// The values the filter was last successfully set to.
    applied: Option<(f32, f32)>,
    status: String,
    /// Shape of the status rather than its text, so a fade does not log a line
    /// every five seconds for an hour.
    status_shape: String,
    /// Settings changed by something with no natural "finished" moment (the
    /// toggle, the scroll wheel), flushed on the next tick.
    dirty: bool,
    config: Store,
}

/// Which pair a slider belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day,
    Night,
}

#[derive(Debug, Clone)]
pub enum Message {
    PopupClosed(Id),
    Surface(cosmic::surface::Action),
    StrengthChanged(Period, f32),
    DimChanged(Period, f32),
    AutoToggled(bool),
    /// Wheel over the panel icon, in scroll notches.
    Nudge(f32),
    SlidersReleased,
    Tick,
}

impl cosmic::Application for App {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;

    const APP_ID: &str = "io.github.big-ol-pants.CosmicExtRedeye";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Self::Message>) {
        let config = Store::new(Self::APP_ID);
        let settings = config.load();

        // Put the location keys on disk so there is something to hand-edit.
        config.seed_location(settings);

        let mut app = App {
            core,
            popup: None,
            settings,
            // Computed before the first refresh, not left at 0.0: zero means
            // "sun on the horizon", which would apply a mid-fade filter for the
            // first few seconds of every login, in broad daylight.
            elevation: solar::elevation(
                solar::now_unix_seconds(),
                settings.latitude,
                settings.longitude,
            ),
            filter: BlueLightFilter::default(),
            applied: None,
            status: "Off".to_string(),
            status_shape: String::new(),
            dirty: false,
            config,
        };

        // Restore the saved filter straight away, so it is already on at login.
        app.refresh_filter();

        (app, Task::none())
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Self::Message> {
        Some(Message::PopupClosed(id))
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::PopupClosed(id) => {
                if self.popup.as_ref() == Some(&id) {
                    self.popup = None;
                }
            }
            Message::Surface(action) => {
                return cosmic::task::message(cosmic::Action::Cosmic(
                    cosmic::app::Action::Surface(action),
                ));
            }
            Message::StrengthChanged(period, value) => {
                match period {
                    Period::Day => self.settings.day_strength = value,
                    Period::Night => self.settings.night_strength = value,
                }
                self.take_manual(period);
                self.refresh_filter();
            }
            Message::DimChanged(period, value) => {
                match period {
                    Period::Day => self.settings.day_dim = value,
                    Period::Night => self.settings.night_dim = value,
                }
                self.take_manual(period);
                self.refresh_filter();
            }
            Message::AutoToggled(auto) => {
                self.settings.auto = auto;
                eprintln!("redeye: auto {}", if auto { "on" } else { "off" });
                self.dirty = true;
                self.refresh_filter();
            }
            Message::Nudge(notches) => {
                // Start from what is on screen, not from the stored manual
                // pair: if the schedule was running, that pair is wherever it
                // was last parked, possibly hours and thousands of kelvin away.
                let (strength, dim) = self.settings.effective(self.elevation);
                if self.settings.auto {
                    eprintln!("redeye: auto off (scrolled)");
                }
                self.settings.auto = false;
                // Carry dim across too, or taking over the warmth would snap
                // the brightness back to whatever it last was.
                self.settings.dim = dim;
                self.settings.strength = (strength + notches * SCROLL_STEP).clamp(0.0, 100.0);
                self.dirty = true;
                self.refresh_filter();
            }
            // Three jobs, in order: find the sun, drop the unchanged-value
            // guard so we re-apply against the compositor's current reality
            // rather than our own last value, then refresh. A display appearing
            // or gamma control coming back to us shows up here too.
            Message::Tick => {
                self.elevation = solar::elevation(
                    solar::now_unix_seconds(),
                    self.settings.latitude,
                    self.settings.longitude,
                );
                self.applied = None;
                self.refresh_filter();
                if self.dirty {
                    self.config.store(self.settings);
                    self.dirty = false;
                }
            }
            // Persist on release rather than on every drag tick, to keep a drag
            // from turning into a burst of config writes.
            Message::SlidersReleased => {
                self.config.store(self.settings);
                self.dirty = false;
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let have_popup = self.popup;
        let button = self
            .core
            .applet
            .icon_button_from_handle(widget::icon::from_svg_bytes(APPLET_ICON).symbolic(true))
            .on_press_with_rectangle(move |offset, bounds| {
                if let Some(id) = have_popup {
                    Message::Surface(destroy_popup(id))
                } else {
                    Message::Surface(app_popup::<App>(
                        move |state: &mut App| {
                            let new_id = Id::unique();
                            state.popup = Some(new_id);
                            let mut popup_settings = state.core.applet.get_popup_settings(
                                state.core.main_window_id().unwrap(),
                                new_id,
                                None,
                                None,
                                None,
                            );

                            popup_settings.positioner.anchor_rect = Rectangle {
                                x: (bounds.x - offset.x) as i32,
                                y: (bounds.y - offset.y) as i32,
                                width: bounds.width as i32,
                                height: bounds.height as i32,
                            };

                            popup_settings
                        },
                        Some(Box::new(|state: &App| {
                            state.popup_view().map(cosmic::Action::App)
                        })),
                    ))
                }
            });

        // MouseArea is layout-transparent -- it returns its child's layout node
        // verbatim -- so on_press_with_rectangle still sees the same bounds,
        // and the button captures its own press and release before this sees
        // them. The popup toggle is untouched.
        let button = widget::mouse_area(button)
            .on_scroll(|delta| Message::Nudge(scroll_notches(delta)));

        // Say which mode we are in. Auto turning itself off is the one thing
        // here a user could trigger by accident -- a stray wheel over the panel
        // -- and the only other symptom would be the screen quietly failing to
        // warm at dusk.
        let tooltip = if self.settings.auto {
            "Redeye - following the sun".to_string()
        } else {
            format!(
                "Redeye - manual, {:.0} K",
                blue_light::temperature_for_percent(self.settings.strength)
            )
        };

        Element::from(self.core.applet.applet_tooltip::<Message>(
            button,
            tooltip,
            self.popup.is_some(),
            Message::Surface,
            None,
        ))
    }

    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        self.popup_view()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        cosmic::iced::time::every(TICK_INTERVAL).map(|_| Message::Tick)
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

impl App {
    /// Dragging a slider means "show me this now": drop out of the schedule and
    /// make the pair being edited the live one.
    ///
    /// This has to happen on the first delta, not on release. If it waited for
    /// release, a tick landing mid-drag would recompute from the schedule and
    /// yank the screen out from under the user's finger.
    fn take_manual(&mut self, period: Period) {
        if self.settings.auto {
            eprintln!("redeye: auto off (slider)");
        }
        self.settings.auto = false;
        let (strength, dim) = match period {
            Period::Day => (self.settings.day_strength, self.settings.day_dim),
            Period::Night => (self.settings.night_strength, self.settings.night_dim),
        };
        self.settings.strength = strength;
        self.settings.dim = dim;
    }

    fn refresh_filter(&mut self) {
        let (strength, dim) = self.settings.effective(self.elevation);

        // A drag can emit the same value several times over; skip the work when
        // nothing moved. Failures deliberately do not count as applied, so the
        // filter keeps retrying and recovers on its own once whatever was
        // holding gamma control lets go.
        if self.applied == Some((strength, dim)) {
            return;
        }

        let (status, shape) = match self.filter.set_strength(strength, dim) {
            Ok(status) => {
                self.applied = Some((strength, dim));
                let shape = match status {
                    blue_light::FilterStatus::Inactive => "off".to_string(),
                    blue_light::FilterStatus::Active { applied, total, .. } => {
                        format!("on {applied}/{total}")
                    }
                };
                (status.to_string(), shape)
            }
            Err(err) => {
                self.applied = None;
                (err.to_string(), err.to_string())
            }
        };

        // Inside the panel this is the only diagnostic surface there is. Log on
        // the shape of the situation -- on, off, degraded, failed -- not on the
        // text: during a fade the temperature moves a few kelvin per tick, so
        // logging every change would be hundreds of lines per dusk. The numbers
        // live in the popup, which is where someone is looking when they want
        // them.
        if shape != self.status_shape {
            eprintln!("redeye: {status}");
            self.status_shape = shape;
        }
        self.status = status;
    }

    fn popup_view(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let settings = &self.settings;
        let (strength, dim) = settings.effective(self.elevation);

        let auto = widget::settings::section().add(widget::settings::item(
            "Follow the sun",
            widget::toggler(settings.auto).on_toggle(Message::AutoToggled),
        ));

        let day = widget::settings::section()
            .title("Day")
            .add(warmth_slider(Period::Day, settings.day_strength))
            .add(dim_slider(Period::Day, settings.day_dim));

        let night = widget::settings::section()
            .title("Night")
            .add(warmth_slider(Period::Night, settings.night_strength))
            .add(dim_slider(Period::Night, settings.night_dim));

        // What is on screen right now, which only matches a slider when the
        // schedule is off or the sun is well up or well down.
        let now = if settings.auto {
            format!(
                "Now {:.0} K, -{:.0}%   sun {:+.1}\u{00b0}",
                blue_light::temperature_for_percent(strength),
                dim,
                self.elevation
            )
        } else {
            format!(
                "Manual {:.0} K, -{:.0}%",
                blue_light::temperature_for_percent(strength),
                dim
            )
        };

        let content = widget::column::with_capacity(5)
            .push(auto)
            .push(day)
            .push(night)
            .push(widget::text::caption(now))
            .push(widget::text::caption(&self.status))
            .width(Length::Fill)
            .spacing(spacing.space_s)
            .padding(spacing.space_s);

        Element::from(self.core.applet.popup_container(content))
    }
}

/// Label and current value on one row, slider underneath.
///
/// `settings::item` would put the slider beside the label, which in a popup
/// this narrow leaves barely any slider to aim with.
fn labelled_slider(
    label: &'static str,
    value_text: String,
    range: std::ops::RangeInclusive<f32>,
    value: f32,
    on_change: impl Fn(f32) -> Message + 'static,
) -> Element<'static, Message> {
    widget::column::with_capacity(2)
        .push(widget::settings::item_row(vec![
            widget::text::body(label).into(),
            widget::space::horizontal().into(),
            widget::text::body(value_text).into(),
        ]))
        .push(widget::slider(range, value, on_change).on_release(Message::SlidersReleased))
        .spacing(cosmic::theme::spacing().space_xxs)
        .into()
}

fn warmth_slider(period: Period, value: f32) -> Element<'static, Message> {
    let kelvin = blue_light::temperature_for_percent(value);
    labelled_slider("Warmth", format!("{kelvin:.0} K"), 0.0..=100.0, value, move |v| {
        Message::StrengthChanged(period, v)
    })
}

fn dim_slider(period: Period, value: f32) -> Element<'static, Message> {
    labelled_slider("Dim", format!("{value:.0}%"), 0.0..=90.0, value, move |v| {
        Message::DimChanged(period, v)
    })
}

/// Normalise a wheel event to notches. Pixel deltas come from touchpads and
/// high-resolution wheels; the divisor is a feel constant, not a conversion.
fn scroll_notches(delta: mouse::ScrollDelta) -> f32 {
    match delta {
        mouse::ScrollDelta::Lines { y, .. } => y,
        mouse::ScrollDelta::Pixels { y, .. } => y / 50.0,
    }
}
