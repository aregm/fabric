//! Audit records, one JSON object per line. Each component appends what it did; the dashboard
//! reads them in Phase 8. Records never hold event titles or secrets.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use serde_json::Value;

#[derive(Debug, Default)]
pub struct AuditLog {
    file: Option<Mutex<File>>,
}

impl AuditLog {
    /// A log that drops every record.
    #[must_use]
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Appends to the file at `path`, creating it and its directory if needed.
    pub fn open(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Some(Mutex::new(file)),
        })
    }

    /// Appends one record. Audit failures never stop a request; they are reported on stderr.
    pub fn record(&self, record: &Value) {
        let Some(file) = &self.file else {
            return;
        };
        let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
        if let Err(error) = writeln!(file, "{record}") {
            eprintln!("audit log: {error}");
        }
    }
}
