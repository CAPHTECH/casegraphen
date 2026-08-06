use super::{MemoryValidationFinding, ValidTime};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TemporalDisposition {
    NotYetValid,
    Current,
    Expired,
}

pub(crate) fn validate_timestamp(
    value: &str,
    location: &str,
    findings: &mut Vec<MemoryValidationFinding>,
) -> Option<i64> {
    match parse_utc_timestamp(value) {
        Some(timestamp) => Some(timestamp),
        None => {
            findings.push(MemoryValidationFinding {
                code: "invalid_utc_timestamp".to_owned(),
                location: location.to_owned(),
                detail: "v0 timestamps must be canonical UTC YYYY-MM-DDTHH:MM:SSZ values"
                    .to_owned(),
            });
            None
        }
    }
}

pub(crate) fn validate_valid_time(
    valid_time: &ValidTime,
    location: &str,
    findings: &mut Vec<MemoryValidationFinding>,
) {
    let from = valid_time
        .valid_from
        .as_deref()
        .and_then(|value| validate_timestamp(value, &format!("{location}.valid_from"), findings));
    let until = valid_time
        .valid_until
        .as_deref()
        .and_then(|value| validate_timestamp(value, &format!("{location}.valid_until"), findings));
    if matches!((from, until), (Some(from), Some(until)) if until <= from) {
        findings.push(MemoryValidationFinding {
            code: "invalid_valid_time_interval".to_owned(),
            location: location.to_owned(),
            detail: "valid_until must be later than valid_from".to_owned(),
        });
    }
}

pub(crate) fn disposition(valid_time: &ValidTime, as_of: &str) -> TemporalDisposition {
    let Some(as_of) = parse_utc_timestamp(as_of) else {
        return TemporalDisposition::NotYetValid;
    };
    if valid_time
        .valid_from
        .as_deref()
        .and_then(parse_utc_timestamp)
        .is_some_and(|from| as_of < from)
    {
        return TemporalDisposition::NotYetValid;
    }
    if valid_time
        .valid_until
        .as_deref()
        .and_then(parse_utc_timestamp)
        .is_some_and(|until| as_of >= until)
    {
        return TemporalDisposition::Expired;
    }
    TemporalDisposition::Current
}

fn parse_utc_timestamp(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let year = parse_digits(bytes, 0, 4)? as i64;
    let month = parse_digits(bytes, 5, 2)? as i64;
    let day = parse_digits(bytes, 8, 2)? as i64;
    let hour = parse_digits(bytes, 11, 2)? as i64;
    let minute = parse_digits(bytes, 14, 2)? as i64;
    let second = parse_digits(bytes, 17, 2)? as i64;
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn parse_digits(bytes: &[u8], start: usize, length: usize) -> Option<u32> {
    bytes
        .get(start..start + length)?
        .iter()
        .try_fold(0_u32, |value, byte| {
            byte.is_ascii_digit()
                .then_some(value * 10 + u32::from(byte - b'0'))
        })
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => 0,
    }
}

// Howard Hinnant's civil-date mapping, shifted to Unix epoch days.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_utc_parser_checks_calendar_boundaries() {
        assert!(parse_utc_timestamp("2024-02-29T23:59:59Z").is_some());
        assert!(parse_utc_timestamp("2023-02-29T00:00:00Z").is_none());
        assert!(parse_utc_timestamp("2026-08-06T00:00:00+00:00").is_none());
    }
}
