use std::fmt;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedReadError(pub String);

impl fmt::Display for BoundedReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub fn read(path: &Path, maximum: usize) -> Result<Vec<u8>, BoundedReadError> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| BoundedReadError(format!("open {}: {error}", path.display())))?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| BoundedReadError(format!("read {}: {error}", path.display())))?;
    if bytes.len() > maximum {
        return Err(BoundedReadError(format!(
            "{} exceeds the {maximum} byte bound",
            path.display()
        )));
    }
    Ok(bytes)
}
