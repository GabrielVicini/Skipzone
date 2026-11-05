//! Scalar newtypes for physical quantities at public API boundaries.
//!
//! Policy: a frequency cannot be passed where an altitude is expected, but
//! inside the integrator loop everything is raw `f64` in SI units for speed;
//! values are unwrapped once at the boundary.

macro_rules! scalar_unit {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, PartialOrd, Debug, Default)]
        pub struct $name(f64);

        impl $name {
            #[must_use]
            pub const fn new(value: f64) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn get(self) -> f64 {
                self.0
            }
        }

        impl core::ops::Add for $name {
            type Output = Self;
            fn add(self, rhs: Self) -> Self {
                Self(self.0 + rhs.0)
            }
        }

        impl core::ops::Sub for $name {
            type Output = Self;
            fn sub(self, rhs: Self) -> Self {
                Self(self.0 - rhs.0)
            }
        }

        impl core::ops::Mul<f64> for $name {
            type Output = Self;
            fn mul(self, rhs: f64) -> Self {
                Self(self.0 * rhs)
            }
        }

        impl core::ops::Neg for $name {
            type Output = Self;
            fn neg(self) -> Self {
                Self(-self.0)
            }
        }
    };
}

scalar_unit!(
    /// Length or distance, metres.
    Meters
);
scalar_unit!(
    /// Angle, radians.
    Radians
);
scalar_unit!(
    /// Wave frequency, hertz (cycles per second, not angular).
    Hertz
);
scalar_unit!(
    /// Electron number density, m^-3.
    PerCubicMeter
);
scalar_unit!(
    /// Collision frequency, s^-1.
    PerSecond
);
scalar_unit!(
    /// Magnetic flux density magnitude, tesla.
    Tesla
);
scalar_unit!(
    /// Absorption, nepers. Field amplitude ratio exp(-A); multiply by
    /// 20/ln(10) for decibels.
    Nepers
);
scalar_unit!(
    /// Time interval, seconds.
    Seconds
);

impl Meters {
    #[must_use]
    pub const fn from_km(km: f64) -> Self {
        Self(km * 1e3)
    }
}

impl Radians {
    #[must_use]
    pub const fn from_degrees(deg: f64) -> Self {
        Self(deg * core::f64::consts::PI / 180.0)
    }

    #[must_use]
    pub const fn to_degrees(self) -> f64 {
        self.0 * 180.0 / core::f64::consts::PI
    }
}

impl Hertz {
    /// Angular frequency `omega = 2 pi f`, rad/s.
    #[must_use]
    pub const fn angular(self) -> f64 {
        2.0 * core::f64::consts::PI * self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_round_trip() {
        assert_eq!(Meters::from_km(300.0).get(), 300_000.0);
        let a = Radians::from_degrees(90.0);
        assert!((a.get() - std::f64::consts::FRAC_PI_2).abs() < 1e-15);
        assert!((a.to_degrees() - 90.0).abs() < 1e-12);
        assert_eq!(Hertz::new(1.0).angular(), 2.0 * std::f64::consts::PI);
    }

    #[test]
    fn arithmetic_preserves_type() {
        let d = Meters::new(2.0) + Meters::new(3.0) - Meters::new(1.0);
        assert_eq!(d.get(), 4.0);
        assert_eq!((-Meters::new(2.0) * 3.0).get(), -6.0);
    }
}
