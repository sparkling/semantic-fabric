//! Prospective logical work for source preparation, independent of compiler work.
//! Native driver allocation and physical allocator overhead are not measured here.

use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

use crate::{Error, Result};

#[derive(Clone, Copy)]
pub struct SourceWork<'a>(Option<&'a dyn QueryControl>);

impl<'a> SourceWork<'a> {
    /// `None` preserves the explicit raw/diagnostic path.
    pub const fn new(control: Option<&'a dyn QueryControl>) -> Self {
        Self(control)
    }

    pub const fn control(self) -> Option<&'a dyn QueryControl> {
        self.0
    }

    pub fn checkpoint(self) -> Result<()> {
        if let Some(control) = self.0 {
            control.checkpoint()?;
        }
        Ok(())
    }

    pub fn charge(self, units: usize) -> Result<()> {
        if let Some(control) = self.0 {
            control.checkpoint()?;
            let units = u64::try_from(units).map_err(|_| self.overflow())?;
            control.consume(QueryCharge::SourceWork, units)?;
            control.checkpoint()?;
        }
        Ok(())
    }

    /// Pay a product before evaluating the corresponding work or allocation.
    pub fn product(self, count: usize, width: usize) -> Result<()> {
        self.checkpoint()?;
        let units = count.checked_mul(width).ok_or_else(|| self.overflow())?;
        self.charge(units)
    }

    pub fn vector<T>(self, slots: usize) -> Result<Vec<T>> {
        self.charge(slots)?;
        self.product(slots, std::mem::size_of::<T>())?;
        let mut output = Vec::new();
        output.try_reserve_exact(slots).map_err(|_| allocation())?;
        self.checkpoint()?;
        Ok(output)
    }

    pub fn string(self, value: &str) -> Result<String> {
        self.charge(value.len())?;
        let mut output = String::new();
        output
            .try_reserve_exact(value.len())
            .map_err(|_| allocation())?;
        output.push_str(value);
        self.checkpoint()?;
        Ok(output)
    }

    /// Bind one value after admitting its copy, vector growth and placeholder.
    /// Growth is modelled on the logical length alone: one doubling move when
    /// the length crosses a power of two, never allocator-specific spare capacity.
    pub fn parameter(self, params: &mut Vec<String>, index: &mut usize, value: &str) -> Result<()> {
        self.checkpoint()?;
        let next_index = index.checked_add(1).ok_or_else(|| self.overflow())?;
        let next_len = params.len().checked_add(1).ok_or_else(|| self.overflow())?;
        self.charge(1)?;
        if doubles(params.len(), next_len) {
            self.product(next_len, std::mem::size_of::<String>())?;
        }
        let owned = self.string(value)?;
        params.try_reserve(1).map_err(|_| allocation())?;
        self.checkpoint()?;
        params.push(owned);
        *index = next_index;
        self.checkpoint()
    }

    /// Move nested parameter buffers in SQL text order without copying their bytes.
    pub fn append_parameters(
        self,
        params: &mut Vec<String>,
        index: &mut usize,
        mut nested: Vec<String>,
    ) -> Result<()> {
        self.checkpoint()?;
        let next_index = index
            .checked_add(nested.len())
            .ok_or_else(|| self.overflow())?;
        let next_len = params
            .len()
            .checked_add(nested.len())
            .ok_or_else(|| self.overflow())?;
        self.charge(nested.len())?;
        if !nested.is_empty() {
            if doubles(params.len(), next_len) {
                self.product(next_len, std::mem::size_of::<String>())?;
            }
            params.try_reserve(nested.len()).map_err(|_| allocation())?;
        }
        self.checkpoint()?;
        params.append(&mut nested);
        *index = next_index;
        self.checkpoint()
    }

    fn overflow(self) -> Error {
        let reason = QueryControlError::AccountingOverflow;
        Error::QueryControl(self.0.map_or(reason, |control| control.terminate(reason)))
    }
}

fn allocation() -> Error {
    Error::Emit("source preparation allocation failed".into())
}

/// Whether growing a logical length to `next` crosses a power-of-two boundary,
/// the deterministic model of one amortized doubling reallocation.
fn doubles(len: usize, next: usize) -> bool {
    len == 0 || next.next_power_of_two() != len.next_power_of_two()
}

/// Normalize a SQLite declaration without input-sized temporary allocations.
/// All names recognized by natural_xsd are ASCII and shorter than 64 bytes.
/// Unicode case folding is retained (for example long-s folds to ASCII S).
pub(crate) fn declaration_metadata(
    declaration: &str,
    work: SourceWork<'_>,
) -> Result<(Option<sf_core::datatype::XsdTypeCode>, Option<usize>)> {
    let mut name = [0_u8; 64];
    let mut length = 0;
    let mut space = false;
    let mut open = None;
    for (index, ch) in declaration.char_indices() {
        work.charge(ch.len_utf8())?;
        if ch == '(' {
            open = Some(index);
            break;
        }
        if ch.is_whitespace() {
            space = length > 0;
            continue;
        }
        if space {
            if length == name.len() {
                return Ok((None, None));
            }
            name[length] = b' ';
            length += 1;
            space = false;
        }
        for upper in ch.to_uppercase() {
            work.charge(1)?;
            if !upper.is_ascii() || length == name.len() {
                return Ok((None, None));
            }
            name[length] = upper as u8;
            length += 1;
        }
    }
    // This second normalizer only sees <=64 ASCII bytes, no parentheses or
    // expanding Unicode. Prepay its one String buffer and linear passes through
    // split('('), split_whitespace, chars/uppercase, and output copying.
    work.product(length, 5)?;
    let normalized = std::str::from_utf8(&name[..length])
        .map_err(|_| Error::Emit("invalid normalized declaration".into()))?;
    let code = sf_core::datatype::natural_xsd(normalized);
    work.checkpoint()?;
    let mut padding = None;
    if matches!(normalized, "CHAR" | "CHARACTER" | "NCHAR") {
        if let Some(open) = open {
            let start = open + 1;
            for (relative, ch) in declaration[start..].char_indices() {
                work.charge(ch.len_utf8())?;
                if ch == ')' {
                    let number = &declaration[start..start + relative];
                    work.product(number.len(), 2)?; // trim + allocation-free integer parse
                    padding = number.trim().parse::<usize>().ok();
                    break;
                }
            }
        }
    }
    work.checkpoint()?;
    Ok((code, padding))
}

/// Explicit logical capacity keeps work independent of allocator overgrant.
pub struct SourceVec<T> {
    values: Vec<T>,
    requested: usize,
}

impl<T> Default for SourceVec<T> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            requested: 0,
        }
    }
}

impl<T> SourceVec<T> {
    pub fn push(&mut self, value: T, work: SourceWork<'_>) -> Result<()> {
        self.insert(self.values.len(), value, work)
    }

    pub fn as_slice(&self) -> &[T] {
        &self.values
    }

    pub fn replace_copy(&mut self, index: usize, value: T, work: SourceWork<'_>) -> Result<()>
    where
        T: Copy,
    {
        work.charge(1)?;
        work.product(1, std::mem::size_of::<T>())?;
        let slot = self
            .values
            .get_mut(index)
            .ok_or_else(|| Error::Emit("invalid source vector replacement".into()))?;
        *slot = value;
        work.checkpoint()
    }

    pub fn insert(&mut self, index: usize, value: T, work: SourceWork<'_>) -> Result<()> {
        if index > self.values.len() {
            return Err(Error::Emit("invalid source vector insertion".into()));
        }
        work.charge(1)?;
        if self.values.len() == self.requested {
            let next = self
                .requested
                .checked_mul(2)
                .ok_or_else(|| work.overflow())?
                .max(4);
            work.charge(next)?;
            work.product(next, std::mem::size_of::<T>())?;
            work.product(self.values.len(), std::mem::size_of::<T>())?;
            self.values
                .try_reserve_exact(next - self.values.len())
                .map_err(|_| allocation())?;
            self.requested = next;
        }
        work.product(self.values.len() - index, std::mem::size_of::<T>())?;
        self.values.insert(index, value);
        work.checkpoint()
    }

    pub fn pop(&mut self) -> Option<T> {
        self.values.pop()
    }

    pub fn into_vec(self) -> Vec<T> {
        self.values
    }
}

impl SourceVec<u8> {
    /// Append with the same logical growth/copy charges as individual pushes,
    /// but checkpoint at most 4 KiB of already reserved storage at a time.
    pub fn extend_bytes(&mut self, mut bytes: &[u8], work: SourceWork<'_>) -> Result<()> {
        work.checkpoint()?;
        while !bytes.is_empty() {
            if self.values.len() == self.requested {
                self.push(bytes[0], work)?;
                bytes = &bytes[1..];
                continue;
            }
            let count = bytes
                .len()
                .min(self.requested - self.values.len())
                .min(4096);
            work.charge(count)?;
            self.values.extend_from_slice(&bytes[..count]);
            work.checkpoint()?;
            bytes = &bytes[count..];
        }
        work.checkpoint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::query_control::{QueryBudget, QueryLimits};

    fn budget(source: u64) -> QueryBudget {
        QueryBudget::new(QueryLimits::new(u64::MAX, source, u64::MAX, u64::MAX))
    }

    #[test]
    fn parameter_admission_is_capacity_independent_and_preserves_failed_input() {
        for spare in [0, 128] {
            let mut params = Vec::with_capacity(spare);
            params.push("existing".to_owned());
            let mut index = 7;
            // One existing slot: the second element crosses a doubling boundary.
            let n = 1 + 2 * std::mem::size_of::<String>() as u64 + "α\0β".len() as u64;
            let short = budget(n - 1);
            assert!(matches!(
                SourceWork::new(Some(&short)).parameter(&mut params, &mut index, "α\0β"),
                Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
            ));
            assert_eq!(params, ["existing"]);
            assert_eq!(index, 7);
            let exact = budget(n);
            SourceWork::new(Some(&exact))
                .parameter(&mut params, &mut index, "α\0β")
                .unwrap();
            assert_eq!(params, ["existing", "α\0β"]);
            assert_eq!(index, 8);
            assert_eq!(exact.consumed(QueryCharge::SourceWork), n);
            assert_eq!(exact.consumed(QueryCharge::CompilerWork), 0);
        }
        let control = budget(u64::MAX);
        let mut params = vec![];
        let mut index = usize::MAX;
        assert!(matches!(
            SourceWork::new(Some(&control)).parameter(&mut params, &mut index, "x"),
            Err(Error::QueryControl(QueryControlError::AccountingOverflow))
        ));
        assert!(params.is_empty());
        assert_eq!(
            control.checkpoint(),
            Err(QueryControlError::AccountingOverflow)
        );
    }

    #[test]
    fn nested_parameters_move_in_order_only_after_admission() {
        // Growing one slot to three crosses a doubling boundary once.
        let n = 2 + 3 * std::mem::size_of::<String>() as u64;
        for limit in [n - 1, n] {
            let control = budget(limit);
            let mut params = vec!["parent".into()];
            let mut index = 5;
            let result = SourceWork::new(Some(&control)).append_parameters(
                &mut params,
                &mut index,
                vec!["first".into(), "second".into()],
            );
            if limit == n {
                result.unwrap();
                assert_eq!(params, ["parent", "first", "second"]);
                assert_eq!(index, 7);
            } else {
                assert!(matches!(
                    result,
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
                assert_eq!(params, ["parent"]);
                assert_eq!(index, 5);
            }
        }
    }

    #[test]
    fn source_copies_are_prepaid_at_exact_boundary_without_charging_compiler() {
        let control = budget(3);
        let work = SourceWork::new(Some(&control));
        assert_eq!(work.string("abc").unwrap(), "abc");
        assert_eq!(control.consumed(QueryCharge::SourceWork), 3);
        assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
        assert!(matches!(
            work.string("x"),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
        assert!(matches!(
            work.vector::<u8>(0),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
    }

    #[test]
    fn vector_slots_and_storage_are_charged_before_reservation() {
        for limit in [5, 6] {
            let control = budget(limit);
            let result = SourceWork::new(Some(&control)).vector::<u16>(2);
            assert_eq!(result.is_ok(), limit == 6);
        }
    }

    #[test]
    fn batched_bytes_preserve_push_accounting_across_growth_boundaries() {
        let bytes = vec![0x80; 16_385];
        let individual = budget(u64::MAX);
        let mut reference = SourceVec::default();
        for byte in &bytes {
            reference
                .push(*byte, SourceWork::new(Some(&individual)))
                .unwrap();
        }
        let total = individual.consumed(QueryCharge::SourceWork);
        for width in [1, 3, 4096, bytes.len()] {
            let control = budget(total);
            let mut batched = SourceVec::default();
            for chunk in bytes.chunks(width) {
                batched
                    .extend_bytes(chunk, SourceWork::new(Some(&control)))
                    .unwrap();
            }
            assert_eq!(batched.as_slice(), reference.as_slice());
            assert_eq!(control.consumed(QueryCharge::SourceWork), total);
        }
        let short = budget(total - 1);
        assert!(matches!(
            SourceVec::default().extend_bytes(&bytes, SourceWork::new(Some(&short))),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
    }

    #[test]
    fn insertion_prepays_movement_before_mutating_values() {
        for limit in [4, 5] {
            let mut values = SourceVec::default();
            values.push(10_u16, SourceWork::new(None)).unwrap();
            values.push(20_u16, SourceWork::new(None)).unwrap();
            let control = budget(limit);
            // One visit plus two moved u16 values; existing capacity suffices.
            let result = values.insert(0, 5, SourceWork::new(Some(&control)));
            if limit == 5 {
                result.unwrap();
                assert_eq!(values.as_slice(), &[5, 10, 20]);
                assert_eq!(control.consumed(QueryCharge::SourceWork), 5);
            } else {
                assert!(matches!(
                    result,
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
                assert_eq!(values.as_slice(), &[10, 20]);
            }
        }
    }

    #[test]
    fn terminal_causes_win_over_later_work_and_overflow() {
        for reason in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            let control = budget(u64::MAX);
            control.terminate(reason);
            let work = SourceWork::new(Some(&control));
            for result in [work.product(usize::MAX, 2), work.string("x").map(|_| ())] {
                assert!(matches!(result, Err(Error::QueryControl(actual)) if actual == reason));
            }
            assert_eq!(control.consumed(QueryCharge::SourceWork), 0);
        }
    }

    #[test]
    fn overflow_is_sticky_and_raw_copy_keeps_values() {
        let control = budget(u64::MAX);
        let work = SourceWork::new(Some(&control));
        assert!(matches!(
            work.product(usize::MAX, 2),
            Err(Error::QueryControl(QueryControlError::AccountingOverflow))
        ));
        assert_eq!(
            control.checkpoint(),
            Err(QueryControlError::AccountingOverflow)
        );
        assert_eq!(SourceWork::new(None).string("α\0β").unwrap(), "α\0β");
    }
}
