// SPDX-License-Identifier: MPL-2.0

use cosmic::cosmic_config::{self, ConfigGet, ConfigSet};

pub const CONFIG_VERSION: u64 = 1;

const STRENGTH_KEY: &str = "strength";
const DIM_KEY: &str = "dim";

/// Slider positions, persisted so the filter survives a logout.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Settings {
    pub strength: f32,
    pub dim: f32,
}

/// Wrapper around the cosmic-config store that degrades to in-memory defaults
/// when the config directory is unavailable, so the applet still runs.
#[derive(Debug)]
pub struct Store {
    handler: Option<cosmic_config::Config>,
}

impl Store {
    pub fn new(app_id: &str) -> Self {
        Self {
            handler: cosmic_config::Config::new(app_id, CONFIG_VERSION).ok(),
        }
    }

    pub fn load(&self) -> Settings {
        let Some(handler) = self.handler.as_ref() else {
            return Settings::default();
        };

        let defaults = Settings::default();

        Settings {
            strength: handler.get(STRENGTH_KEY).unwrap_or(defaults.strength),
            dim: handler.get(DIM_KEY).unwrap_or(defaults.dim),
        }
    }

    pub fn store(&self, settings: Settings) {
        let Some(handler) = self.handler.as_ref() else {
            return;
        };

        let _ = handler.set(STRENGTH_KEY, settings.strength);
        let _ = handler.set(DIM_KEY, settings.dim);
    }
}
