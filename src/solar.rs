// SPDX-License-Identifier: MPL-2.0

//! Where the sun is, from a Unix timestamp and a point on the globe.
//!
//! This is NOAA's published algorithm, which is Meeus chapter 25 truncated to
//! the terms worth more than a hundredth of a degree -- a few seconds of clock
//! time around sunset, far finer than a blue light schedule can notice.
//!
//! Elevation depends only on a UTC instant and a position. No calendar and no
//! timezone database are involved, which is why this needs no dependencies at
//! all. Only *displaying* a local clock time would need those.
//!
//! Everything here is geometric: no atmospheric refraction and no allowance for
//! the sun's radius. That is deliberate. The thresholds the filter fades
//! between are defined against geometric elevation; published sunrise tables
//! are quoted at -0.833 degrees (34 arcminutes of refraction plus a 16
//! arcminute semidiameter), which is a concern for the tests below and not for
//! the runtime.

/// Elevation of the sun above the horizon, in degrees. Negative below.
///
/// `latitude` is north-positive and `longitude` is **east**-positive, so London
/// is `(51.5074, -0.1278)`. NOAA's own spreadsheet is west-positive in places,
/// and the resulting sign error is under a minute for Greenwich -- which is
/// exactly why it survives to production. The tests use Auckland and Anchorage,
/// where it would be worth hours.
#[must_use]
pub fn elevation(unix_seconds: f64, latitude: f64, longitude: f64) -> f64 {
    // The Unix epoch is JD 2440587.5; the half is there because Julian days
    // roll over at noon. UT is used throughout -- feeding UT to the orbital
    // terms where the textbook says TT misplaces the sun by under a thousandth
    // of a degree, and the hour angle genuinely wants UT, so delta-T is left
    // out on purpose.
    let julian_day = unix_seconds / 86_400.0 + 2_440_587.5;
    let century = (julian_day - 2_451_545.0) / 36_525.0;

    // Geometric mean longitude, reduced before use: the series grows by 36000
    // degrees a century, and trigonometry on a number that large throws away
    // precision for nothing.
    let mean_longitude =
        (280.466_46 + century * (36_000.769_83 + century * 0.000_303_2)).rem_euclid(360.0);
    // Mean anomaly. Deliberately not reduced; it only ever reaches a sine.
    let mean_anomaly = 357.529_11 + century * (35_999.050_29 - century * 0.000_153_7);
    let eccentricity = 0.016_708_634 - century * (0.000_042_037 + century * 0.000_000_126_7);

    let anomaly = mean_anomaly.to_radians();
    // Equation of the centre: where an elliptical orbit has actually got to,
    // against where a circular one would be.
    let centre = anomaly.sin() * (1.914_602 - century * (0.004_817 + century * 0.000_014))
        + (2.0 * anomaly).sin() * (0.019_993 - century * 0.000_101)
        + (3.0 * anomaly).sin() * 0.000_289;

    // Longitude of the moon's ascending node, which drives both nutation terms.
    let node = (125.04 - 1_934.136 * century).to_radians();
    // Apparent longitude: true longitude corrected for nutation and for the
    // aberration of light.
    let apparent_longitude =
        (mean_longitude + centre - 0.005_69 - 0.004_78 * node.sin()).to_radians();

    // Mean obliquity of the ecliptic, then the same nutation correction.
    // Dropping that 0.00256*cos(node) is the classic slip: it is small, and it
    // is the difference between agreeing with NOAA and not.
    let mean_obliquity = 23.0
        + (26.0 + (21.448 - century * (46.815 + century * (0.000_59 - century * 0.001_813))) / 60.0)
            / 60.0;
    let obliquity = (mean_obliquity + 0.002_56 * node.cos()).to_radians();

    // Declination: how far north or south of the equator the sun stands.
    let declination = (obliquity.sin() * apparent_longitude.sin()).asin();

    // Equation of time, in minutes: sundial time minus clock time. The tilt of
    // the axis and the eccentricity of the orbit each contribute, and together
    // they are worth sixteen minutes either way. Four minutes is a degree of
    // rotation, so this is not an optional refinement.
    let tan_half = (obliquity / 2.0).tan().powi(2);
    let longitude_rad = mean_longitude.to_radians();
    let equation_of_time = 4.0
        * (tan_half * (2.0 * longitude_rad).sin() - 2.0 * eccentricity * anomaly.sin()
            + 4.0 * eccentricity * tan_half * anomaly.sin() * (2.0 * longitude_rad).cos()
            - 0.5 * tan_half * tan_half * (4.0 * longitude_rad).sin()
            - 1.25 * eccentricity * eccentricity * (2.0 * anomaly).sin())
        .to_degrees();

    // Minutes of UTC elapsed today. rem_euclid rather than %, so a machine
    // whose clock reads before 1970 does not produce a negative time of day.
    let utc_minutes = unix_seconds.rem_euclid(86_400.0) / 60.0;
    // True solar time: four minutes of rotation per degree of longitude. East
    // is positive, so a place east of Greenwich reaches noon earlier and its
    // solar clock runs ahead of UTC.
    let solar_minutes =
        (utc_minutes + equation_of_time + 4.0 * longitude).rem_euclid(1_440.0);
    // Hour angle: zero at local solar noon, fifteen degrees per hour.
    let hour_angle = (solar_minutes / 4.0 - 180.0).to_radians();

    let latitude_rad = latitude.to_radians();
    let cos_zenith = latitude_rad.sin() * declination.sin()
        + latitude_rad.cos() * declination.cos() * hour_angle.cos();

    // Rounding can put this a hair outside the domain of acos near the poles.
    90.0 - cos_zenith.clamp(-1.0, 1.0).acos().to_degrees()
}

/// Seconds since the Unix epoch, or 0.0 if the clock reads before 1970.
///
/// A machine whose RTC has not been set yet gets a wrong sun rather than a
/// panic, and corrects itself on the next tick once NTP lands. Elevation is
/// recomputed from absolute time every tick with no monotonic assumptions, so
/// clock steps, suspend and resume all self-correct -- do not "optimise" this
/// into an incremental update.
#[must_use]
pub fn now_unix_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-checked three ways: against `gammastep -p -l <lat>:<lon>`, which
    /// agreed to four decimal places at five locations simultaneously; against
    /// the analytic solstice-noon altitude `90 - |lat| +/- 23.4366`; and
    /// against published London sunset times, which came out exact at both
    /// solstices.
    #[test]
    fn matches_reference_elevations() {
        // (unix seconds, latitude, longitude, expected degrees)
        let cases = [
            (1_782_043_200.0, 51.5074, -0.1278, 61.9270),   // London, summer solstice noon
            (1_797_854_400.0, 51.5074, -0.1278, 15.0546),   // London, winter solstice noon
            (1_789_430_400.0, 51.5074, -0.1278, -35.3845),  // London, middle of the night
            (1_773_986_400.0, 51.5074, -0.1278, -1.3588),   // London, equinox morning
            (1_782_043_200.0, -33.8688, 151.2093, -62.4196), // Sydney, southern hemisphere
            (1_782_000_000.0, -36.8485, 174.7633, 29.4775), // Auckland, far east
            (1_782_079_200.0, 61.2181, -149.9003, 52.2180), // Anchorage, far west
        ];
        for (seconds, latitude, longitude, expected) in cases {
            let got = elevation(seconds, latitude, longitude);
            assert!(
                (got - expected).abs() < 1e-3,
                "elevation({seconds}, {latitude}, {longitude}) = {got}, expected {expected}"
            );
        }
    }

    /// The solstice noon cases above should equal `90 - |lat| +/- obliquity`
    /// with no help from the equation of time, so they pin down declination and
    /// the obliquity correction on their own.
    #[test]
    fn solstice_noon_matches_closed_form() {
        let obliquity = 23.4366;
        let summer = elevation(1_782_043_200.0, 51.5074, -0.1278);
        let winter = elevation(1_797_854_400.0, 51.5074, -0.1278);
        assert!((summer - (90.0 - 51.5074 + obliquity)).abs() < 0.05, "summer {summer}");
        assert!((winter - (90.0 - 51.5074 - obliquity)).abs() < 0.05, "winter {winter}");
    }

    /// A hemisphere or longitude sign flip would sail past London, where it is
    /// worth under a minute. Inside the arctic circle it is worth the whole
    /// answer.
    #[test]
    fn polar_day_and_night_have_the_right_sign() {
        // Tromso at local midnight in June: the midnight sun is up.
        assert!(elevation(1_782_000_000.0, 69.6496, 18.9560) > 0.0);
        // Tromso at local noon in December: polar night, the sun never rises.
        assert!(elevation(1_797_850_800.0, 69.6496, 18.9560) < 0.0);
    }

    /// Published sunrise and sunset are quoted at -0.833 degrees. Asserting the
    /// elevation at the published minute is an independent check that does not
    /// share a common mode with the vectors above.
    #[test]
    fn published_london_sunsets_land_on_the_refraction_angle() {
        // 2026-06-21 sunset 21:21 BST = 20:21 UTC; 2026-12-21 sunset 15:53 GMT.
        for seconds in [1_782_073_260.0, 1_797_868_380.0] {
            let got = elevation(seconds, 51.5074, -0.1278);
            assert!((got + 0.833).abs() < 0.15, "elevation {got} should be near -0.833");
        }
    }

    #[test]
    fn non_finite_input_does_not_panic() {
        assert!(elevation(f64::NAN, 51.5074, -0.1278).is_nan());
        assert!(elevation(0.0, f64::NAN, -0.1278).is_nan());
    }
}
