//! Maidenhead locator conversion.
//!
//! The Maidenhead system (IARU, 1980) partitions the globe by repeatedly
//! subdividing, alternating letters and digits, starting from the antipode of
//! the prime meridian at the south pole:
//!
//! | pair | longitude step | latitude step | alphabet |
//! |------|----------------|---------------|----------|
//! | 1 field     | 20 deg  | 10 deg    | `A`..`R` (18) |
//! | 2 square    | 2 deg   | 1 deg     | `0`..`9` (10) |
//! | 3 subsquare | 5 min   | 2.5 min   | `a`..`x` (24) |
//! | 4 extended  | 30 s    | 15 s      | `0`..`9` (10) |
//!
//! Longitude always steps twice as coarsely as latitude, which is what makes
//! the cells roughly square in the middle latitudes. Encoding is therefore just
//! a mixed-radix expansion of `(lon + 180) / 2` and `lat + 90` in degrees, and
//! decoding is the same expansion run backwards to the CENTRE of the named
//! cell - the convention every amateur-radio tool uses, and the only choice
//! that makes `decode(encode(p))` sit within half a cell of `p`.

/// Longitude cell width in degrees for each pair, most significant first.
const LON_STEPS: [f64; 4] = [20.0, 2.0, 2.0 / 24.0, 2.0 / 240.0];
/// Latitude cell height in degrees for each pair (always half the longitude).
const LAT_STEPS: [f64; 4] = [10.0, 1.0, 1.0 / 24.0, 1.0 / 240.0];

/// Locator precision emitted by [`encode`]: 6 characters (subsquare), the
/// standard exchange in amateur radio.
pub const ENCODED_PAIRS: usize = 3;

/// Digit/letter alphabets per pair, alternating letters and digits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Alphabet {
    /// `A`..`R` for pair 1, `A`..`X` for pair 3 (case-insensitive on input).
    Letters(u32),
    /// `0`..`9`.
    Digits,
}

const ALPHABETS: [Alphabet; 4] = [
    Alphabet::Letters(18),
    Alphabet::Digits,
    Alphabet::Letters(24),
    Alphabet::Digits,
];

impl Alphabet {
    fn count(self) -> u32 {
        match self {
            Self::Letters(n) => n,
            Self::Digits => 10,
        }
    }

    fn to_char(self, index: u32, upper: bool) -> char {
        let base = match self {
            Self::Letters(_) if upper => b'A',
            Self::Letters(_) => b'a',
            Self::Digits => b'0',
        };
        char::from(base + u8::try_from(index).unwrap_or(0))
    }

    fn index_of(self, c: char) -> Option<u32> {
        let n = self.count();
        let index = match self {
            Self::Letters(_) => {
                let c = c.to_ascii_uppercase();
                c.is_ascii_uppercase()
                    .then(|| u32::from(c) - u32::from(b'A'))?
            }
            Self::Digits => c.is_ascii_digit().then(|| c.to_digit(10))??,
        };
        (index < n).then_some(index)
    }
}

/// Encode a position as a locator of `pairs` character pairs (clamped to 1..=4).
///
/// Latitude is clamped to the poles and longitude wrapped, so this is total: a
/// map click can never produce a locator the parser would reject.
#[must_use]
pub fn encode(lat_deg: f64, lon_deg: f64, pairs: usize) -> String {
    let pairs = pairs.clamp(1, 4);
    let mut lon = (lon_deg + 180.0).rem_euclid(360.0);
    let mut lat = (lat_deg.clamp(-90.0, 90.0) + 90.0).min(179.999_999);
    let mut out = String::with_capacity(pairs * 2);
    for i in 0..pairs {
        let alphabet = ALPHABETS[i];
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let lon_index = ((lon / LON_STEPS[i]).floor() as u32).min(alphabet.count() - 1);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let lat_index = ((lat / LAT_STEPS[i]).floor() as u32).min(alphabet.count() - 1);
        lon -= f64::from(lon_index) * LON_STEPS[i];
        lat -= f64::from(lat_index) * LAT_STEPS[i];
        // Uppercase fields, lowercase subsquares: the conventional casing.
        let upper = i == 0;
        out.push(alphabet.to_char(lon_index, upper));
        out.push(alphabet.to_char(lat_index, upper));
    }
    out
}

/// Decode a 2-, 4-, 6- or 8-character locator to the CENTRE of its cell,
/// as `(latitude, longitude)` in degrees. Returns `None` if the string is not a
/// valid locator.
#[must_use]
pub fn decode(locator: &str) -> Option<(f64, f64)> {
    let chars: Vec<char> = locator.trim().chars().collect();
    if chars.len() < 2 || chars.len() > 8 || !chars.len().is_multiple_of(2) {
        return None;
    }
    let pairs = chars.len() / 2;
    let (mut lon, mut lat) = (0.0_f64, 0.0_f64);
    for i in 0..pairs {
        let alphabet = ALPHABETS[i];
        lon += f64::from(alphabet.index_of(chars[2 * i])?) * LON_STEPS[i];
        lat += f64::from(alphabet.index_of(chars[2 * i + 1])?) * LAT_STEPS[i];
    }
    // Centre of the smallest named cell.
    lon += 0.5 * LON_STEPS[pairs - 1];
    lat += 0.5 * LAT_STEPS[pairs - 1];
    Some((lat - 90.0, lon - 180.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Locators of well-known stations, to the 4-character square everyone
    /// quotes, plus the full 6-character form.
    #[test]
    fn known_locations_encode_correctly() {
        // Denver, CO.
        assert_eq!(encode(39.74, -104.99, 3), "DM79mr");
        // London.
        assert_eq!(encode(51.50, -0.13, 3), "IO91wm");
        // Sydney.
        assert_eq!(&encode(-33.87, 151.21, 3)[..4], "QF56");
        // Tokyo.
        assert_eq!(&encode(35.68, 139.77, 3)[..4], "PM95");
    }

    /// The corners of the system: the origin cell AA00aa sits at the south
    /// pole on the antimeridian, and the far corner is RR99xx.
    #[test]
    fn grid_origin_and_extent() {
        assert_eq!(encode(-90.0, -180.0, 3), "AA00aa");
        assert_eq!(encode(89.999, 179.999, 3), "RR99xx");
        let (lat, lon) = decode("AA00aa").expect("valid");
        assert!(lat < -89.9 && lon < -179.9, "{lat} {lon}");
    }

    /// Decoding lands at the centre of the cell, so a round trip is always
    /// within half a cell - about 2.5 min of latitude at 6 characters.
    #[test]
    fn round_trip_stays_within_half_a_cell() {
        let cases = [
            (39.74, -104.99),
            (51.50, -0.13),
            (-33.87, 151.21),
            (0.0, 0.0),
            (-45.5, -73.25),
            (66.0, 179.5),
        ];
        for (lat, lon) in cases {
            let (dlat, dlon) = decode(&encode(lat, lon, 3)).expect("encodes to a valid locator");
            assert!(
                (dlat - lat).abs() <= 0.5 * LAT_STEPS[2] + 1e-9,
                "lat {lat} -> {dlat}"
            );
            assert!(
                (dlon - lon).abs() <= 0.5 * LON_STEPS[2] + 1e-9,
                "lon {lon} -> {dlon}"
            );
        }
    }

    /// Extended (8-character) precision is finer than 6, which is finer than 4.
    #[test]
    fn more_pairs_means_more_precision() {
        let (lat, lon) = (39.7392, -104.9903);
        let mut prev_error = f64::INFINITY;
        for pairs in 1..=4 {
            let (dlat, dlon) = decode(&encode(lat, lon, pairs)).expect("valid");
            let err = (dlat - lat).hypot(dlon - lon);
            assert!(
                err < prev_error,
                "{pairs} pairs did not improve on the last"
            );
            prev_error = err;
        }
        assert!(prev_error < 0.01);
    }

    #[test]
    fn parsing_is_case_insensitive_and_rejects_nonsense() {
        assert_eq!(decode("io91wd"), decode("IO91WD"));
        assert_eq!(decode("IO91"), decode("io91"));
        assert!(decode("IO9").is_none(), "odd length");
        assert!(decode("SS91wd").is_none(), "field letters stop at R");
        assert!(decode("IO91zz").is_none(), "subsquare letters stop at x");
        assert!(decode("I091wd").is_none(), "digit where a letter belongs");
        assert!(decode("").is_none());
        assert!(decode("IO91wm12ab").is_none(), "too long");
    }

    /// Every locator the encoder can produce must parse: the UI relies on this
    /// when a map click rewrites the grid field.
    #[test]
    fn everything_encode_produces_is_parseable() {
        let mut lat = -89.5;
        while lat <= 89.5 {
            let mut lon = -179.0;
            while lon <= 179.0 {
                let loc = encode(lat, lon, 3);
                assert!(decode(&loc).is_some(), "{loc} from {lat},{lon}");
                lon += 7.3;
            }
            lat += 6.1;
        }
    }
}
