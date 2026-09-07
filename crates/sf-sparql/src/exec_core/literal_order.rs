//! Total, value-aware literal keys used by bounded stable ORDER BY.

use std::cmp::Ordering;

use bigdecimal::BigDecimal;
use oxsdatatypes::{Date, DateTime, Duration, Time, TimezoneOffset};
use sf_core::Literal;

/// A literal and its once-parsed value domain. Fixed cross-domain ranks refine
/// SPARQL's undefined heterogeneous ordering without mixing lexical and value
/// comparisons, which would make a sort comparator non-transitive.
pub(super) struct LiteralSortKey<'a> {
    literal: &'a Literal,
    value: LiteralValueKey,
}

enum LiteralValueKey {
    Boolean(bool),
    Numeric(NumericSortKey),
    DateTime(DateTime),
    Date(Date),
    Time(Time),
    Duration(BigDecimal),
    Lexical,
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
enum NumericSortKey {
    NegativeInfinity,
    Finite(BigDecimal),
    PositiveInfinity,
    Nan,
}

pub(super) fn literal_sort_key(literal: &Literal) -> LiteralSortKey<'_> {
    LiteralSortKey {
        literal,
        value: value_key(literal),
    }
}

pub(super) fn cmp_literal(left: &Literal, right: &Literal) -> Ordering {
    cmp_literal_keys(&literal_sort_key(left), &literal_sort_key(right))
}

pub(super) fn cmp_literal_keys(left: &LiteralSortKey<'_>, right: &LiteralSortKey<'_>) -> Ordering {
    use LiteralValueKey::{Boolean, Date, DateTime, Duration, Lexical, Numeric, Time};
    match (&left.value, &right.value) {
        (Boolean(a), Boolean(b)) => a.cmp(b),
        (Numeric(a), Numeric(b)) => a.cmp(b),
        (DateTime(a), DateTime(b)) => total_calendar_cmp(a, b),
        (Date(a), Date(b)) => total_calendar_cmp(a, b),
        (Time(a), Time(b)) => total_calendar_cmp(a, b),
        (Duration(a), Duration(b)) => a.cmp(b),
        (Lexical, Lexical) => lexical_cmp(left.literal, right.literal),
        _ => value_rank(&left.value).cmp(&value_rank(&right.value)),
    }
}

fn value_rank(key: &LiteralValueKey) -> u8 {
    match key {
        LiteralValueKey::Boolean(_) => 0,
        LiteralValueKey::Numeric(_) => 1,
        LiteralValueKey::DateTime(_) => 2,
        LiteralValueKey::Date(_) => 3,
        LiteralValueKey::Time(_) => 4,
        LiteralValueKey::Duration(_) => 5,
        LiteralValueKey::Lexical => 6,
    }
}

fn total_calendar_cmp<T: PartialOrd>(left: &T, right: &T) -> Ordering {
    left.partial_cmp(right)
        .expect("UTC-normalized calendar values have a total order")
}

fn lexical_cmp(left: &Literal, right: &Literal) -> Ordering {
    left.value()
        .cmp(right.value())
        .then_with(|| left.datatype().as_str().cmp(right.datatype().as_str()))
        .then_with(|| {
            left.language()
                .unwrap_or("")
                .cmp(right.language().unwrap_or(""))
        })
}

fn value_key(literal: &Literal) -> LiteralValueKey {
    const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
    let Some(local) = literal.datatype().as_str().strip_prefix(XSD) else {
        return LiteralValueKey::Lexical;
    };
    let value = literal.value();
    if let Some(numeric) = numeric_sort_key(local, value) {
        return LiteralValueKey::Numeric(numeric);
    }
    match local {
        "boolean" => boolean_key(value)
            .map(LiteralValueKey::Boolean)
            .unwrap_or(LiteralValueKey::Lexical),
        "dateTime" | "dateTimeStamp" => normalized::<DateTime>(value)
            .map(LiteralValueKey::DateTime)
            .unwrap_or(LiteralValueKey::Lexical),
        "date" => normalized::<Date>(value)
            .map(LiteralValueKey::Date)
            .unwrap_or(LiteralValueKey::Lexical),
        "time" => normalized::<Time>(value)
            .map(LiteralValueKey::Time)
            .unwrap_or(LiteralValueKey::Lexical),
        "duration" | "yearMonthDuration" | "dayTimeDuration" => duration_key(local, value)
            .map(LiteralValueKey::Duration)
            .unwrap_or(LiteralValueKey::Lexical),
        _ => LiteralValueKey::Lexical,
    }
}

trait CalendarValue: Sized + std::str::FromStr {
    fn at_utc(self) -> Option<Self>;
}

impl CalendarValue for DateTime {
    fn at_utc(self) -> Option<Self> {
        self.adjust(Some(TimezoneOffset::UTC))
    }
}

impl CalendarValue for Date {
    fn at_utc(self) -> Option<Self> {
        self.adjust(Some(TimezoneOffset::UTC))
    }
}

impl CalendarValue for Time {
    fn at_utc(self) -> Option<Self> {
        self.adjust(Some(TimezoneOffset::UTC))
    }
}

fn normalized<T: CalendarValue>(value: &str) -> Option<T> {
    value.parse::<T>().ok()?.at_utc()
}

fn boolean_key(value: &str) -> Option<bool> {
    match value {
        "false" | "0" => Some(false),
        "true" | "1" => Some(true),
        _ => None,
    }
}

fn numeric_sort_key(local: &str, value: &str) -> Option<NumericSortKey> {
    match local {
        "double" => numeric_float64(value),
        "float" => numeric_float32(value),
        "decimal" if valid_decimal_lexical(value) => value.parse().ok().map(NumericSortKey::Finite),
        "integer" | "long" | "int" | "short" | "byte" | "nonNegativeInteger"
        | "nonPositiveInteger" | "negativeInteger" | "positiveInteger" | "unsignedLong"
        | "unsignedInt" | "unsignedShort" | "unsignedByte"
            if valid_integer_lexical(value) =>
        {
            value.parse().ok().map(NumericSortKey::Finite)
        }
        _ => None,
    }
}

fn valid_integer_lexical(value: &str) -> bool {
    let digits = value
        .strip_prefix('+')
        .or_else(|| value.strip_prefix('-'))
        .unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_decimal_lexical(value: &str) -> bool {
    let unsigned = value
        .strip_prefix('+')
        .or_else(|| value.strip_prefix('-'))
        .unwrap_or(value);
    let mut parts = unsigned.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    parts.next().is_none()
        && whole.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.is_none_or(|digits| digits.bytes().all(|byte| byte.is_ascii_digit()))
        && (!whole.is_empty() || fraction.is_some_and(|digits| !digits.is_empty()))
}

fn valid_float_lexical(value: &str) -> bool {
    let mut parts = value.split(['e', 'E']);
    let mantissa = parts.next().unwrap_or_default();
    let exponent = parts.next();
    valid_decimal_lexical(mantissa)
        && exponent.is_none_or(valid_integer_lexical)
        && parts.next().is_none()
}

fn numeric_float32(value: &str) -> Option<NumericSortKey> {
    numeric_float(value, |lexical| numeric_from_f32(lexical.parse().ok()?))
}

fn numeric_float64(value: &str) -> Option<NumericSortKey> {
    numeric_float(value, |lexical| numeric_from_f64(lexical.parse().ok()?))
}

fn numeric_float(
    value: &str,
    parsed: impl FnOnce(&str) -> Option<NumericSortKey>,
) -> Option<NumericSortKey> {
    match value {
        "INF" => Some(NumericSortKey::PositiveInfinity),
        "-INF" => Some(NumericSortKey::NegativeInfinity),
        "NaN" => Some(NumericSortKey::Nan),
        _ if valid_float_lexical(value) => parsed(value),
        _ => None,
    }
}

fn numeric_from_f32(value: f32) -> Option<NumericSortKey> {
    if value == f32::NEG_INFINITY {
        Some(NumericSortKey::NegativeInfinity)
    } else if value == f32::INFINITY {
        Some(NumericSortKey::PositiveInfinity)
    } else {
        BigDecimal::try_from(value).ok().map(NumericSortKey::Finite)
    }
}

fn numeric_from_f64(value: f64) -> Option<NumericSortKey> {
    if value == f64::NEG_INFINITY {
        Some(NumericSortKey::NegativeInfinity)
    } else if value == f64::INFINITY {
        Some(NumericSortKey::PositiveInfinity)
    } else {
        BigDecimal::try_from(value).ok().map(NumericSortKey::Finite)
    }
}

fn duration_key(local: &str, value: &str) -> Option<BigDecimal> {
    let duration = match local {
        "duration" => value.parse::<Duration>().ok()?,
        "yearMonthDuration" => value
            .parse::<oxsdatatypes::YearMonthDuration>()
            .ok()?
            .into(),
        "dayTimeDuration" => value.parse::<oxsdatatypes::DayTimeDuration>().ok()?.into(),
        _ => return None,
    };
    duration_effect_at_reference(duration)
}

/// A deterministic linear extension of XSD duration's partial order. The
/// reference is one of the four dates used by the XSD comparison algorithm, so
/// every pair XSD can order retains that order; incomparable pairs gain one.
fn duration_effect_at_reference(duration: Duration) -> Option<BigDecimal> {
    const BASE_YEAR: i128 = 1697;
    const BASE_MONTH: i128 = 2;
    let months = i128::from(duration.years()) * 12 + i128::from(duration.months());
    let target_index = BASE_YEAR * 12 + BASE_MONTH - 1 + months;
    let target_year = target_index.div_euclid(12);
    let target_month = target_index.rem_euclid(12) + 1;
    let days = civil_day(target_year, target_month, 1) - civil_day(BASE_YEAR, BASE_MONTH, 1);
    let whole_seconds = days * 86_400
        + i128::from(duration.days()) * 86_400
        + i128::from(duration.hours()) * 3_600
        + i128::from(duration.minutes()) * 60;
    let seconds: BigDecimal = duration.seconds().to_string().parse().ok()?;
    Some(BigDecimal::from(whole_seconds) + seconds)
}

fn civil_day(year: i128, month: i128, day: i128) -> i128 {
    let year = year - i128::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    era * 146_097 + year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year
}

/// The aggregation lane's existing lossy scalar conversion. ORDER BY never
/// calls it; sorting uses the exact key above.
pub(super) fn numeric_value(literal: &Literal) -> Option<f64> {
    let local = literal
        .datatype()
        .as_str()
        .strip_prefix("http://www.w3.org/2001/XMLSchema#")?;
    numeric_sort_key(local, literal.value())?;
    literal.value().parse::<f64>().ok()
}
