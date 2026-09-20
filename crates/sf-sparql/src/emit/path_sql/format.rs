//! Count primitive formatting output without allocation, then admit its copy.
use super::*;
use std::fmt::{Arguments, Write};

struct Counter<'a> {
    work: SourceWork<'a>,
    bytes: usize,
    failure: Option<sf_sql::Error>,
}
impl Write for Counter<'_> {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let result = self.work.charge(1).and_then(|()| {
            self.bytes = self.bytes.checked_add(value.len()).ok_or_else(|| {
                let reason = sf_core::query_control::QueryControlError::AccountingOverflow;
                sf_sql::Error::QueryControl(
                    self.work.control().map_or(reason, |c| c.terminate(reason)),
                )
            })?;
            Ok(())
        });
        result.map_err(|cause| {
            self.failure = Some(cause);
            std::fmt::Error
        })
    }
}

#[cfg(test)]
thread_local! { pub(super) static COPIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

pub(in crate::emit) fn render(work: SourceWork<'_>, args: Arguments<'_>) -> Result<String> {
    let mut counter = Counter {
        work,
        bytes: 0,
        failure: None,
    };
    if counter.write_fmt(args).is_err() {
        return Err(counter
            .failure
            .map(error)
            .unwrap_or_else(|| Error::Sql("path formatting failed".into())));
    }
    work.charge(counter.bytes).map_err(error)?;
    let mut result = String::new();
    result
        .try_reserve_exact(counter.bytes)
        .map_err(|_| Error::Sql("path SQL allocation failed".into()))?;
    #[cfg(test)]
    COPIES.with(|count| count.set(count.get() + 1));
    result
        .write_fmt(args)
        .map_err(|_| Error::Sql("path formatting failed".into()))?;
    work.checkpoint().map_err(error)?;
    Ok(result)
}

pub(super) fn join(work: SourceWork<'_>, parts: &[String], separator: &str) -> Result<String> {
    let mut counter = Counter {
        work,
        bytes: 0,
        failure: None,
    };
    for (i, part) in parts.iter().enumerate() {
        if i != 0 && counter.write_str(separator).is_err() {
            return Err(error(counter.failure.unwrap()));
        }
        if counter.write_str(part).is_err() {
            return Err(error(counter.failure.unwrap()));
        }
    }
    work.charge(counter.bytes).map_err(error)?;
    let mut result = String::new();
    result
        .try_reserve_exact(counter.bytes)
        .map_err(|_| Error::Sql("path SQL allocation failed".into()))?;
    for (i, part) in parts.iter().enumerate() {
        work.checkpoint().map_err(error)?;
        if i != 0 {
            result.push_str(separator);
        }
        result.push_str(part);
    }
    Ok(result)
}
