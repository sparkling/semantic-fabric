//! Query-local decoder keys and typed literal predicates; no persistent functions.
use std::sync::{Arc, Mutex};

use rusqlite::{functions::FunctionFlags, types::ValueRef, Connection};
use sf_core::query_control::{QueryCharge, QueryControl};

use crate::error::{Error, Result};

const NAME: &str = "__sf_character_key_v1";

#[cfg(test)]
mod tests;

#[derive(Default)]
struct Context {
    active: bool,
    control: Option<Arc<dyn QueryControl>>,
    failure: Option<Error>,
}

/// Rows must be reset/dropped before this guard. The owned worker also finalizes
/// its statement before teardown, while still owning the connection mutex/lease.
pub(super) struct CharacterKeyGuard<'c> {
    connection: &'c Connection,
    context: Option<Arc<Mutex<Context>>>,
    lexical: bool,
}

impl<'c> CharacterKeyGuard<'c> {
    pub(super) fn install(
        connection: &'c Connection,
        required: bool,
        control: Option<Arc<dyn QueryControl>>,
    ) -> Result<Self> {
        Self::install_kind(connection, required, control, false)
    }

    pub(super) fn install_lexical(
        connection: &'c Connection,
        required: bool,
        control: Option<Arc<dyn QueryControl>>,
    ) -> Result<Self> {
        Self::install_kind(connection, required, control, true)
    }

    fn install_kind(
        connection: &'c Connection,
        required: bool,
        control: Option<Arc<dyn QueryControl>>,
        lexical: bool,
    ) -> Result<Self> {
        let name = if lexical { "__sf_lexical_key_v1" } else { NAME };
        let mut guard = Self {
            connection,
            context: None,
            lexical,
        };
        if !required {
            return Ok(guard);
        }
        // Never redefine an application callback, including a variadic overload.
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_function_list WHERE name = ? COLLATE NOCASE)",
            [name],
            |row| row.get(0),
        )?;
        if exists {
            return Err(Error::Emit(
                "SQLite decoder key function name is already registered".into(),
            ));
        }
        if lexical {
            for name in ["__sf_numeric_cmp_v1", "__sf_iri_key_v1"] {
                if connection.query_row("SELECT EXISTS(SELECT 1 FROM pragma_function_list WHERE name = ? COLLATE NOCASE)", [name], |row| row.get::<_,bool>(0))? {
                    return Err(Error::Emit("SQLite term comparison function name is already registered".into()));
                }
            }
        }
        let context = Arc::new(Mutex::new(Context {
            active: true,
            control,
            failure: None,
        }));
        let registrations: &[(&str, i32, u8)] = if lexical {
            &[
                ("__sf_lexical_key_v1", 3, 0),
                ("__sf_numeric_cmp_v1", 5, 1),
                ("__sf_iri_key_v1", 2, 2),
            ]
        } else {
            &[(NAME, 2, 0)]
        };
        guard.context = Some(context.clone());
        for &(name, arity, kind) in registrations {
            let callback = Arc::clone(&context);
            connection.create_scalar_function(
                name,
                arity,
                FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DIRECTONLY,
                move |args| {
                    let mut state = callback.lock().unwrap_or_else(|p| p.into_inner());
                    let value = (|| {
                        if !state.active {
                            return Err(Error::Emit("inactive SQLite decoder key function".into()));
                        }
                        if kind == 1 {
                            return super::numeric_cmp::evaluate(args, state.control.as_deref());
                        }
                        if kind == 2 {
                            return super::iri_key::evaluate(args, state.control.as_deref());
                        }
                        if lexical {
                            return super::lexical_key::evaluate(args, state.control.as_deref());
                        }
                        let width = match args.get_raw(1) {
                            ValueRef::Integer(n) if n >= 0 => usize::try_from(n).ok(),
                            _ => None,
                        }
                        .ok_or_else(|| Error::Marshal("invalid CHARACTER width".into()))?;
                        character(args.get_raw(0), width, state.control.as_deref())
                    })();
                    match value {
                        Ok(value) => Ok(value),
                        Err(error) => {
                            if state.failure.is_none() {
                                state.failure = Some(error);
                            }
                            Err(rusqlite::Error::UserFunctionError(
                                std::io::Error::other("decoder predicate evaluation failed").into(),
                            ))
                        }
                    }
                },
            )?;
        }
        Ok(guard)
    }

    pub(super) fn map_error(&self, error: Error) -> Error {
        if !matches!(error, Error::Sqlite(_)) {
            return error;
        }
        self.context
            .as_ref()
            .and_then(|state| {
                state
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .failure
                    .take()
            })
            .unwrap_or(error)
    }

    pub(super) fn finish(&mut self) -> Result<()> {
        if let Some(context) = self.context.take() {
            // Even if SQLite refuses removal (an out-of-contract raw cursor),
            // leave only an inert callback, with no request/control retention.
            // Subsequent path admission then fails the collision check.
            let mut state = context.lock().unwrap_or_else(|p| p.into_inner());
            state.active = false;
            state.control = None;
            state.failure = None;
            drop(state);
            let lexical_result = self.connection.remove_function(
                if self.lexical {
                    "__sf_lexical_key_v1"
                } else {
                    NAME
                },
                if self.lexical { 3 } else { 2 },
            );
            let numeric_result = if self.lexical {
                self.connection.remove_function("__sf_numeric_cmp_v1", 5)
            } else {
                Ok(())
            };
            let iri_result = if self.lexical {
                self.connection.remove_function("__sf_iri_key_v1", 2)
            } else {
                Ok(())
            };
            lexical_result?;
            numeric_result?;
            iri_result?;
        }
        Ok(())
    }
}

impl Drop for CharacterKeyGuard<'_> {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// The same Rust lexical conversion and Unicode-scalar padding as row decoding.
/// No SQL length()/cast substitute: embedded NUL, UTF-8 and numeric storage must
/// retain their existing lexical behavior. Charge before scanning/allocating.
pub(super) fn character(
    value: ValueRef<'_>,
    width: usize,
    control: Option<&dyn QueryControl>,
) -> Result<Option<String>> {
    if let Some(control) = control {
        let input = match value {
            ValueRef::Text(bytes) | ValueRef::Blob(bytes) => bytes.len(),
            _ => 32,
        };
        let work = input
            .checked_add(width)
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(Error::QueryControl(
                sf_core::query_control::QueryControlError::SourceWorkExceeded,
            ))?;
        control.consume(QueryCharge::SourceWork, work)?;
    }
    let mut text = super::lexical(value)?;
    if let Some(text) = text.as_mut() {
        let padding = width.saturating_sub(text.chars().count());
        text.try_reserve(padding)
            .map_err(|_| Error::Marshal("CHARACTER key allocation failed".into()))?;
        text.extend(std::iter::repeat_n(' ', padding));
    }
    if let Some(control) = control {
        control.checkpoint()?;
    }
    Ok(text)
}
