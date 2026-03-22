use super::*;

pub(super) fn format_system_time_utc(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let epoch_seconds = i64::try_from(secs).unwrap_or(i64::MAX);
    format_unix_utc(epoch_seconds)
}

pub(super) fn format_unix_utc(epoch_seconds: i64) -> String {
    let day_seconds: i64 = 86_400;
    let days = epoch_seconds.div_euclid(day_seconds);
    let day_remainder = epoch_seconds.rem_euclid(day_seconds);

    let hour = day_remainder / 3_600;
    let minute = (day_remainder % 3_600) / 60;
    let second = day_remainder % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

pub(super) fn civil_from_days(days_since_unix_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}
