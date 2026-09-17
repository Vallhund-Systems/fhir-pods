//! FHIRPath's date / dateTime / time values, enough to compare them: a
//! partial date (`2020`, `2020-03`, `2020-03-01`), an optional time
//! (`T10:00`, `:30`, `.250`) and an optional timezone (`Z`, `+10:00`).
//! Comparison follows the spec: different precisions (seconds and
//! milliseconds counting as one), or a timezone on one side only, is empty;
//! otherwise the instants are compared with offsets normalised.

use core::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Temporal {
    /// 1 year … 3 day, 4 hour, 5 minute, 6 second (milliseconds fold in).
    pub precision: u8,
    /// The instant in milliseconds, offsets applied (a partial date is
    /// taken at its start; only same-precision values are compared).
    pub millis: i64,
    pub has_date: bool,
    pub has_tz: bool,
}

fn digits(s: &[u8], n: usize) -> Option<i64> {
    let part = s.get(..n)?;
    if part.len() != n || !part.iter().all(u8::is_ascii_digit) {
        return None;
    }
    part.iter().try_fold(0i64, |acc, &c| acc.checked_mul(10)?.checked_add((c - b'0') as i64))
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn parse(s: &str) -> Option<Temporal> {
    let b = s.as_bytes();
    let mut i = 0;
    let (mut y, mut mo, mut d) = (1970i64, 1i64, 1i64);
    let mut precision = 0u8;
    let mut has_date = false;
    // a bare `time` (`08:00:00`) has no date part: a colon in third place
    let time_only = b.get(2) == Some(&b':');
    if !time_only && b.first().is_some_and(u8::is_ascii_digit) {
        y = digits(b, 4)?;
        i = 4;
        precision = 1;
        has_date = true;
        if b.get(i) == Some(&b'-') {
            mo = digits(&b[i + 1..], 2)?;
            i += 3;
            precision = 2;
            if b.get(i) == Some(&b'-') {
                d = digits(&b[i + 1..], 2)?;
                i += 3;
                precision = 3;
            }
        }
        if !(1..=12).contains(&mo) || !(1..=days_in_month(y, mo)).contains(&d) {
            return None;
        }
    }
    let (mut h, mut mi, mut sec, mut ms) = (0i64, 0i64, 0i64, 0i64);
    let mut has_tz = false;
    let mut tz = 0i64;
    let time_starts = b.get(i) == Some(&b'T') || (!has_date && b.get(i).is_some_and(u8::is_ascii_digit));
    if time_starts {
        if precision != 0 && precision < 3 {
            return None; // a time needs a full date before it
        }
        if b.get(i) == Some(&b'T') {
            i += 1;
        }
        h = digits(&b[i..], 2)?;
        if b.get(i + 2) != Some(&b':') {
            return None;
        }
        mi = digits(&b[i + 3..], 2)?;
        i += 5;
        precision = 5;
        if b.get(i) == Some(&b':') {
            sec = digits(&b[i + 1..], 2)?;
            i += 3;
            precision = 6;
            if b.get(i) == Some(&b'.') {
                let start = i + 1;
                let mut j = start;
                while b.get(j).is_some_and(u8::is_ascii_digit) {
                    j += 1;
                }
                if j == start {
                    return None;
                }
                let frac = &s[start..j];
                let mut scaled = 0i64;
                for k in 0..3 {
                    scaled = scaled * 10 + frac.as_bytes().get(k).map_or(0, |c| (c - b'0') as i64);
                }
                ms = scaled;
                i = j;
            }
        }
        if h > 23 || mi > 59 || sec > 60 {
            return None;
        }
        match b.get(i) {
            Some(b'Z') => {
                has_tz = true;
                i += 1;
            }
            Some(&sign @ (b'+' | b'-')) => {
                let th = digits(&b[i + 1..], 2)?;
                if b.get(i + 3) != Some(&b':') {
                    return None;
                }
                let tm = digits(&b[i + 4..], 2)?;
                tz = (th * 60 + tm) * if sign == b'-' { -1 } else { 1 };
                has_tz = true;
                i += 6;
            }
            _ => {}
        }
    }
    if i != b.len() || precision == 0 {
        return None;
    }
    let days = if has_date { days_from_civil(y, mo, d) } else { 0 };
    let millis = (((days * 24 + h) * 60 + mi - tz) * 60 + sec) * 1000 + ms;
    Some(Temporal { precision, millis, has_date, has_tz })
}

/// `None` when the values are not comparable (different precision, or a
/// timezone on one side only, or a date against a time).
pub fn compare(a: &Temporal, b: &Temporal) -> Option<Ordering> {
    if a.precision != b.precision || a.has_tz != b.has_tz || a.has_date != b.has_date {
        return None;
    }
    Some(a.millis.cmp(&b.millis))
}
