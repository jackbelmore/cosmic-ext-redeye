use crate::blue_light::BlueLightFilter;
use crate::config::{Settings, Store};
use cosmic::app::{Core, Task};
use cosmic::iced::core::window;
use cosmic::iced::window::Id;
use cosmic::iced::{Alignment, Length, Rectangle};
use cosmic::prelude::*;
use cosmic::surface::action::{app_popup, destroy_popup};
use cosmic::widget::{self};

const APPLET_ICON: &[u8] = include_bytes!("../resources/icons/hicolor/scalable/apps/Redeye.svg");

pub struct App {
    core: Core,
    popup: Option<Id>,
    value: f32,
    dim: f32,
    filter: BlueLightFilter,
    status: String,
    config: Store,
}

#[derive(Debug, Clone)]
pub enum Message {
    PopupClosed(Id),
    Surface(cosmic::surface::Action),
    ValueChanged(f32),
    DimChanged(f32),
    SlidersReleased,
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

        let mut app = App {
            core,
            popup: None,
            value: settings.strength,
            dim: settings.dim,
            filter: BlueLightFilter::default(),
            status: "Off".to_string(),
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
            Message::ValueChanged(value) => {
                self.value = value;
                self.refresh_filter();
            }
            Message::DimChanged(dim) => {
                self.dim = dim;
                self.refresh_filter();
            }
            // Persist on release rather than on every drag tick, to keep a drag
            // from turning into a burst of config writes.
            Message::SlidersReleased => {
                self.config.store(Settings {
                    strength: self.value,
                    dim: self.dim,
                });
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

        Element::from(self.core.applet.applet_tooltip::<Message>(
            button,
            "Redeye",
            self.popup.is_some(),
            Message::Surface,
            None,
        ))
    }

    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        self.popup_view()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

impl App {
    fn refresh_filter(&mut self) {
        self.status = match self.filter.set_strength(self.value, self.dim) {
            Ok(status) => status.to_string(),
            Err(err) => err.to_string(),
        };
    }

    fn popup_view(&self) -> Element<'_, Message> {
        let slider = widget::slider(0.0..=100.0, self.value, Message::ValueChanged)
            .on_release(Message::SlidersReleased);
        let space_s = cosmic::theme::spacing().space_s;
        let label = widget::text(format!("Blue filter: {:.0}%", self.value));
        let dim_slider = widget::slider(0.0..=90.0, self.dim, Message::DimChanged)
            .on_release(Message::SlidersReleased);
        let dim_label = widget::text(format!("Dim: {:.0}%", self.dim));
        let status = widget::text(&self.status);
        let content = widget::column::with_capacity(5)
            .push(slider)
            .push(label)
            .push(dim_slider)
            .push(dim_label)
            .push(status)
            .width(Length::Fill)
            .align_x(Alignment::Center)
            .spacing(space_s)
            .padding(16.0);

        Element::from(self.core.applet.popup_container(content))
    }
}
