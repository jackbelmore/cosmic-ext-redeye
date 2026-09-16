// SPDX-License-Identifier: MPL-2.0

use crate::blue_light;
use cosmic::cosmic_config::{self, ConfigGet, ConfigSet};

pub const CONFIG_VERSION: u64 = 1;

const AUTO_KEY: &str = "auto";
const STRENGTH_KEY: &str = "strength";
const DIM_KEY: &str = "dim";
const DAY_STRENGTH_KEY: &str = "day_strength";
const DAY_DIM_KEY: &str = "day_dim";
const NIGHT_STRENGTH_KEY: &str = "night_strength";
const NIGHT_DIM_KEY: &str = "night_dim";
const LATITUDE_KEY: &str = "latitude";
const LONGITUDE_KEY: &str = "longitude";
const WARMEST_TEMPERATURE_K_KEY: &str = "warmest_temperature_k";

/// Below this, `blue_light::raw_gains`'s own domain has already saturated (its
/// floor is 1000 K), so a colder hand-edit would silently do nothing extra.
/// Treated as invalid, like an out-of-range latitude.
const WARMEST_TEMPERATURE_FLOOR_K: f32 = 1000.0;

/// Everything the applet persists.
///
/// `strength`/`dim` are the **manual** pair -- where the user last parked the
/// filter by hand. They keep the key names they have always had, so a config
/// written before scheduling existed still loads and still means the same
/// thing. The scroll wheel is why this pair has to exist separately rather than
/// being derived from the day/night presets: a wheel notch produces a value
/// that is neither preset, and the only other place to put it would be inside
/// one of them, which would silently rewrite a preset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// Follow the sun between the day and night pairs.
    pub auto: bool,
    pub strength: f32,
    pub dim: f32,
    pub day_strength: f32,
    pub day_dim: f32,
    pub night_strength: f32,
    pub night_dim: f32,
    /// North-positive. Read at startup and never written back -- see `store`.
    pub latitude: f64,
    /// East-positive.
    pub longitude: f64,
    /// The colour temperature "Warmth" reaches at 100%. No UI; hand-edit the
    /// key to push it redder (or pull it back) than the shipped default. Read
    /// at startup and never written back -- see `store`.
    pub warmest_temperature_k: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto: true,
            strength: 30.0,
            dim: 10.0,
            day_strength: 30.0,
            day_dim: 10.0,
            night_strength: 70.0,
            night_dim: 25.0,
            // London. Deliberately not the derived `0.0, 0.0`, which is a real
            // place in the Gulf of Guinea and would quietly work while being
            // wrong -- the worst kind of default.
            latitude: 51.5074,
            longitude: -0.1278,
            warmest_temperature_k: WARMEST_TEMPERATURE_FLOOR_K,
        }
    }
}

impl Settings {
    /// Fold the day and night pairs together for wherever the sun currently is.
    ///
    /// Pure, so it can be tested without a compositor.
    #[must_use]
    pub fn effective(&self, elevation_degrees: f64) -> (f32, f32) {
        if !self.auto {
            return (self.strength, self.dim);
        }

        // An unknown sun should degrade to the mildest setting, not the
        // strongest.
        if !elevation_degrees.is_finite() {
            return (self.day_strength, self.day_dim);
        }

        let span = DAY_ELEVATION - NIGHT_ELEVATION;
        // f64 for the astronomy, f32 for the sliders; the ratio is in 0..=1 so
        // the narrowing is exact to well within a quantisation step.
        #[allow(clippy::cast_possible_truncation)]
        let alpha = (((elevation_degrees - NIGHT_ELEVATION) / span) as f32).clamp(0.0, 1.0);

        (
            quantise(lerp(self.night_strength, self.day_strength, alpha), STRENGTH_STEP),
            quantise(lerp(self.night_dim, self.day_dim, alpha), DIM_STEP),
        )
    }

    /// Clamp anything a hand-edited config could get wrong. Latitude and
    /// longitude have no UI, so this is the only thing standing between a typo
    /// and a very confused sun.
    fn sanitised(mut self) -> Self {
        fn finite(value: f32, fallback: f32, max: f32) -> f32 {
            if value.is_finite() { value.clamp(0.0, max) } else { fallback }
        }
        let defaults = Self::default();
        self.strength = finite(self.strength, defaults.strength, 100.0);
        self.dim = finite(self.dim, defaults.dim, 90.0);
        self.day_strength = finite(self.day_strength, defaults.day_strength, 100.0);
        self.day_dim = finite(self.day_dim, defaults.day_dim, 90.0);
        self.night_strength = finite(self.night_strength, defaults.night_strength, 100.0);
        self.night_dim = finite(self.night_dim, defaults.night_dim, 90.0);
        if !self.latitude.is_finite() || self.latitude.abs() > 90.0 {
            self.latitude = defaults.latitude;
        }
        if !self.longitude.is_finite() || self.longitude.abs() > 180.0 {
            self.longitude = defaults.longitude;
        }
        if !self.warmest_temperature_k.is_finite()
            || self.warmest_temperature_k < WARMEST_TEMPERATURE_FLOOR_K
            || self.warmest_temperature_k >= blue_light::NEUTRAL_TEMPERATURE_K
        {
            self.warmest_temperature_k = defaults.warmest_temperature_k;
        }
        self
    }
}

/// Sun at or above this is full day; at or below `NIGHT_ELEVATION` is full
/// night. These are gammastep's and redshift's defaults, so the fade should
/// feel like the tool this applet replaced. Constants rather than config keys:
/// nobody asked to tune them, and equal values would divide by zero.
const DAY_ELEVATION: f64 = 3.0;
const NIGHT_ELEVATION: f64 = -6.0;

/// Quantisation of the interpolated values. 0.25 of a percentage point is about
/// 10 K, well below anything an eye will catch, and it keeps a slow fade from
/// rewriting the ramp -- and logging a status line -- on every single tick.
const STRENGTH_STEP: f32 = 0.25;
const DIM_STEP: f32 = 0.5;

/// Written `a(1-t) + bt` rather than `a + (b-a)t` because only this form is
/// exact at the endpoints: `0.1 + (0.3-0.1)*1.0` is 0.29999998, not 0.3. The
/// applet compares applied values for exact equality to decide whether to touch
/// the compositor at all, so an endpoint that is a hair off would make every
/// tick look like a change.
fn lerp(from: f32, to: f32, alpha: f32) -> f32 {
    from * (1.0 - alpha) + to * alpha
}

fn quantise(value: f32, step: f32) -> f32 {
    (value / step).round() * step
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

        // Whether to follow the sun is not a plain default. A config that
        // already has a strength key was written by a build with no schedule in
        // it, and an upgrade should leave the screen exactly as the user left
        // it; a fresh install should get the feature it shipped with. Each key
        // is its own file, so a missing one is an error here rather than a
        // default, and the two cases are easy to tell apart. Once the toggle is
        // touched, `auto` is on disk and this never runs again.
        let upgraded = handler.get::<f32>(STRENGTH_KEY).is_ok();

        Settings {
            auto: handler.get(AUTO_KEY).unwrap_or(!upgraded),
            strength: handler.get(STRENGTH_KEY).unwrap_or(defaults.strength),
            dim: handler.get(DIM_KEY).unwrap_or(defaults.dim),
            day_strength: handler.get(DAY_STRENGTH_KEY).unwrap_or(defaults.day_strength),
            day_dim: handler.get(DAY_DIM_KEY).unwrap_or(defaults.day_dim),
            night_strength: handler.get(NIGHT_STRENGTH_KEY).unwrap_or(defaults.night_strength),
            night_dim: handler.get(NIGHT_DIM_KEY).unwrap_or(defaults.night_dim),
            latitude: handler.get(LATITUDE_KEY).unwrap_or(defaults.latitude),
            longitude: handler.get(LONGITUDE_KEY).unwrap_or(defaults.longitude),
            warmest_temperature_k: handler
                .get(WARMEST_TEMPERATURE_K_KEY)
                .unwrap_or(defaults.warmest_temperature_k),
        }
        .sanitised()
    }

    pub fn store(&self, settings: Settings) {
        let Some(handler) = self.handler.as_ref() else {
            return;
        };

        // One transaction rather than eight independent ones, so a watcher sees
        // a single change and the writes land together.
        let tx = handler.transaction();
        let _ = tx.set(AUTO_KEY, settings.auto);
        let _ = tx.set(STRENGTH_KEY, settings.strength);
        let _ = tx.set(DIM_KEY, settings.dim);
        let _ = tx.set(DAY_STRENGTH_KEY, settings.day_strength);
        let _ = tx.set(DAY_DIM_KEY, settings.day_dim);
        let _ = tx.set(NIGHT_STRENGTH_KEY, settings.night_strength);
        let _ = tx.set(NIGHT_DIM_KEY, settings.night_dim);
        // Latitude, longitude, and warmest_temperature_k are deliberately
        // absent. None of the three has a UI; they are meant to be edited by
        // hand, so writing them back here would stamp on an edit made while
        // the applet is running. `seed_hand_editable` puts them on disk once
        // so there is something to find.
        let _ = tx.commit();
    }

    /// Put the keys that have no UI on disk if they are not there yet, so a
    /// user has something to edit: latitude, longitude, and
    /// `warmest_temperature_k`. Never overwrites.
    pub fn seed_hand_editable(&self, settings: Settings) {
        let Some(handler) = self.handler.as_ref() else {
            return;
        };
        if handler.get::<f64>(LATITUDE_KEY).is_err() {
            let _ = handler.set(LATITUDE_KEY, settings.latitude);
        }
        if handler.get::<f64>(LONGITUDE_KEY).is_err() {
            let _ = handler.set(LONGITUDE_KEY, settings.longitude);
        }
        if handler.get::<f32>(WARMEST_TEMPERATURE_K_KEY).is_err() {
            let _ = handler.set(WARMEST_TEMPERATURE_K_KEY, settings.warmest_temperature_k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            auto: true,
            strength: 34.0,
            dim: 12.0,
            day_strength: 30.0,
            day_dim: 10.0,
            night_strength: 70.0,
            night_dim: 25.0,
            ..Settings::default()
        }
    }

    // Exact equality is the assertion, not an oversight: the whole reason for
    // the a(1-t)+bt lerp form is that the endpoints come back bit-identical.
    #[allow(clippy::float_cmp)]
    #[test]
    fn full_day_and_full_night_are_exact() {
        let s = settings();
        // Well above the day threshold, and well below the night one.
        assert_eq!(s.effective(45.0), (30.0, 10.0));
        assert_eq!(s.effective(-30.0), (70.0, 25.0));
        // Exactly on the thresholds, too -- this is what the lerp form buys.
        assert_eq!(s.effective(3.0), (30.0, 10.0));
        assert_eq!(s.effective(-6.0), (70.0, 25.0));
    }

    #[test]
    fn midway_through_the_fade_is_between_the_two() {
        let s = settings();
        // Halfway between +3 and -6.
        let (strength, dim) = s.effective(-1.5);
        assert!((strength - 50.0).abs() < 0.3, "strength {strength}");
        assert!((dim - 17.5).abs() < 0.3, "dim {dim}");
    }

    #[allow(clippy::float_cmp)]
    #[test]
    fn manual_ignores_the_sun() {
        let s = Settings { auto: false, ..settings() };
        assert_eq!(s.effective(45.0), (34.0, 12.0));
        assert_eq!(s.effective(-30.0), (34.0, 12.0));
    }

    #[allow(clippy::float_cmp)]
    #[test]
    fn an_unknown_sun_degrades_to_day() {
        assert_eq!(settings().effective(f64::NAN), (30.0, 10.0));
    }

    #[test]
    fn a_hand_edited_config_cannot_produce_a_black_screen() {
        let broken = Settings {
            latitude: f64::NAN,
            longitude: 515.074,
            night_strength: f32::NAN,
            day_dim: -50.0,
            warmest_temperature_k: 50_000.0,
            ..Settings::default()
        }
        .sanitised();
        assert!(broken.latitude.is_finite() && broken.longitude.abs() <= 180.0);
        assert!(broken.night_strength.is_finite() && broken.day_dim >= 0.0);
        assert!(
            broken.warmest_temperature_k.is_finite()
                && broken.warmest_temperature_k < blue_light::NEUTRAL_TEMPERATURE_K
        );
        let (strength, dim) = broken.effective(0.0);
        assert!(strength.is_finite() && dim.is_finite());
    }
}
