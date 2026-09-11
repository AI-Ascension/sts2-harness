// SPDX-License-Identifier: MIT

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn expiry() -> &'static str {
    static EXPIRY: OnceLock<String> = OnceLock::new();
    EXPIRY
        .get_or_init(|| {
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_secs()
                .saturating_add(900);
            let days = i64::try_from(seconds / 86_400).expect("day range");
            let seconds_of_day = seconds % 86_400;
            let z = days + 719_468;
            let era = if z >= 0 {
                z / 146_097
            } else {
                (z - 146_096) / 146_097
            };
            let day_of_era = z - era * 146_097;
            let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524
                - day_of_era / 146_096)
                / 365;
            let year = year_of_era + era * 400;
            let day_of_year =
                day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
            let month_prime = (5 * day_of_year + 2) / 153;
            let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
            let month = month_prime + if month_prime < 10 { 3 } else { -9 };
            let year = year + i64::from(month <= 2);
            let hour = seconds_of_day / 3_600;
            let minute = (seconds_of_day % 3_600) / 60;
            let second = seconds_of_day % 60;
            format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
        })
        .as_str()
}
