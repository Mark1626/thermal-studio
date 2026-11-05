//! Calendar days, with no time of day or time zone, and how templates print them.

use std::fmt;

/// How `| date` prints a day when no format is given: `5 Nov 2026`.
pub(crate) const DEFAULT_FORMAT: &str = "%-d %b %Y";

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// A day of the calendar, from year 1 to 9999. Written `2026-11-05`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: i32,
    month: u8,
    day: u8,
}

impl Date {
    pub const MIN: Date = Date {
        year: 1,
        month: 1,
        day: 1,
    };
    pub const MAX: Date = Date {
        year: 9999,
        month: 12,
        day: 31,
    };

    /// `None` if there's no such day.
    pub const fn new(year: i32, month: u32, day: u32) -> Option<Date> {
        if year < 1 || year > 9999 || month < 1 || month > 12 {
            return None;
        }
        if day < 1 || day > days_in_month(year, month) {
            return None;
        }
        Some(Date {
            year,
            month: month as u8,
            day: day as u8,
        })
    }

    /// Read `2026-11-05`.
    pub fn parse(s: &str) -> Option<Date> {
        let mut parts = s.trim().split('-');
        let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
            return None;
        }
        let num = |p: &str| {
            p.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| p.parse::<u32>().ok())
                .flatten()
        };
        Date::new(num(y)? as i32, num(m)?, num(d)?)
    }

    pub fn year(self) -> i32 {
        self.year
    }

    pub fn month(self) -> u32 {
        self.month as u32
    }

    pub fn day(self) -> u32 {
        self.day as u32
    }

    /// The day `days` later (or earlier, if negative), kept within years 1 to 9999.
    pub fn add_days(self, days: i64) -> Date {
        let n = self
            .days_since_epoch()
            .saturating_add(days)
            .clamp(Date::MIN.days_since_epoch(), Date::MAX.days_since_epoch());
        Date::from_days_since_epoch(n)
    }

    /// 0 for Monday to 6 for Sunday.
    pub fn weekday(self) -> u32 {
        // 1970-01-01 was a Thursday.
        (self.days_since_epoch() + 3).rem_euclid(7) as u32
    }

    /// Print with `%`-codes: `%Y` 2026, `%y` 26, `%m` 11, `%-m` 11, `%d` 05, `%-d` 5,
    /// `%B` November, `%b` Nov, `%A` Thursday, `%a` Thu, `%u` 4 (the day of the week, 1 for
    /// Monday to 7 for Sunday) and `%%` for `%`. Any other code is an error, which names
    /// it: a template written for a newer version may use codes this one doesn't know.
    pub fn format(self, format: &str) -> Result<String, String> {
        let month = MONTHS[self.month as usize - 1];
        let weekday = WEEKDAYS[self.weekday() as usize];
        let mut out = String::new();
        let mut chars = format.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            let unpadded = chars.next_if_eq(&'-').is_some();
            match (chars.next(), unpadded) {
                (Some('Y'), false) => out.push_str(&self.year.to_string()),
                (Some('y'), false) => out.push_str(&format!("{:02}", self.year % 100)),
                (Some('m'), false) => out.push_str(&format!("{:02}", self.month)),
                (Some('m'), true) => out.push_str(&self.month.to_string()),
                (Some('d'), false) => out.push_str(&format!("{:02}", self.day)),
                (Some('d'), true) => out.push_str(&self.day.to_string()),
                (Some('B'), false) => out.push_str(month),
                (Some('b'), false) => out.push_str(&month[..3]),
                (Some('A'), false) => out.push_str(weekday),
                (Some('a'), false) => out.push_str(&weekday[..3]),
                (Some('u'), false) => out.push_str(&(self.weekday() + 1).to_string()),
                (Some('%'), false) => out.push('%'),
                (None, _) => {
                    return Err(
                        "a date format can't end with `%` (write `%%` for a percent sign)".into(),
                    );
                }
                (Some(c), unpadded) => {
                    let code = if unpadded {
                        format!("%-{c}")
                    } else {
                        format!("%{c}")
                    };
                    return Err(format!(
                        "`{code}` isn't a date code this version of Thermal Studio knows"
                    ));
                }
            }
        }
        Ok(out)
    }

    /// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`).
    fn days_since_epoch(self) -> i64 {
        let (m, d) = (self.month as i64, self.day as i64);
        let y = self.year as i64 - (m <= 2) as i64;
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn from_days_since_epoch(n: i64) -> Date {
        let z = n + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        Date {
            year: (yoe + era * 400 + (month <= 2) as i64) as i32,
            month: month as u8,
            day: day as u8,
        }
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

const fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Date {
        Date::parse(s).unwrap()
    }

    #[test]
    fn parses_only_real_days() {
        assert_eq!(d("2026-11-05").to_string(), "2026-11-05");
        assert_eq!(d(" 2024-02-29 ").day(), 29);
        for bad in [
            "2026-02-29",
            "1900-02-29",
            "2026-13-01",
            "2026-00-10",
            "2026-04-31",
            "0000-01-01",
            "2026-1-5",
            "26-01-05",
            "2026-01-05T10:00",
            "+026-01-05",
            "today",
            "",
        ] {
            assert_eq!(Date::parse(bad), None, "{bad}");
        }
        assert!(Date::parse("2000-02-29").is_some());
    }

    #[test]
    fn day_arithmetic_round_trips() {
        assert_eq!(d("1970-01-01").days_since_epoch(), 0);
        assert_eq!(d("2026-11-05").add_days(30), d("2026-12-05"));
        assert_eq!(d("2026-12-25").add_days(7), d("2027-01-01"));
        assert_eq!(d("2024-02-28").add_days(1), d("2024-02-29"));
        assert_eq!(d("2024-03-01").add_days(-1), d("2024-02-29"));
        assert_eq!(d("2026-11-05").add_days(i64::MAX), Date::MAX);
        assert_eq!(d("2026-11-05").add_days(i64::MIN), Date::MIN);
        // Every day of a 400-year cycle survives the round trip.
        let start = d("2000-01-01").days_since_epoch();
        for n in start..start + 146_097 {
            assert_eq!(Date::from_days_since_epoch(n).days_since_epoch(), n);
        }
    }

    #[test]
    fn weekdays_and_formats() {
        let day = d("2026-11-05");
        assert_eq!(day.weekday(), 3, "a Thursday");
        let f = |day: Date, format: &str| day.format(format).unwrap();
        assert_eq!(f(d("2000-01-01"), "%A"), "Saturday");
        assert_eq!(f(day, DEFAULT_FORMAT), "5 Nov 2026");
        assert_eq!(f(day, "%a %-d %B %Y"), "Thu 5 November 2026");
        assert_eq!(f(day, "%d/%m/%y"), "05/11/26");
        assert_eq!(f(day, "%u"), "4");
        assert_eq!(f(d("2026-11-08"), "%u %a"), "7 Sun");
        assert_eq!(f(day, "%-m/%-d 100%%"), "11/5 100%");
        for (bad, why) in [
            ("%q", "`%q` isn't a date code"),
            ("%-B", "`%-B` isn't"),
            ("5%", "can't end with `%`"),
        ] {
            let e = day.format(bad).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
    }
}
